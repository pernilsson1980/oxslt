//! Compiles a parsed stylesheet document into the IR.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::ir::*;
use super::pattern;
use crate::catalog::ResourceLoader;
use crate::parse::{ParseOptions, parse_with};
use crate::tree::{Document, NamePool, NodeId, NodeKind, QName, Sym};
use crate::xpath::{self, ast::Expr, ast::Name};

pub const XSLT_NS: &str = "http://www.w3.org/1999/XSL/Transform";
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// The namespaces whose `document` element writes a file: EXSLT's and
/// James Clark's XT.
pub const DOCUMENT_ELEMENT_NAMESPACES: [&str; 2] =
    ["http://exslt.org/common", "http://www.jclark.com/xt"];

/// Saxon's namespace, for `saxon:output`, which libxslt writes as it writes
/// `exsl:document`.
const SAXON_NS: &str = "http://icl.com/saxon";

/// The output attributes xsltDocumentElem reads, in its order.
const DOCUMENT_SETTINGS: [&str; 10] = [
    "version",
    "encoding",
    "method",
    "doctype-system",
    "doctype-public",
    "standalone",
    "indent",
    "omit-xml-declaration",
    "cdata-section-elements",
    "append",
];

#[derive(Debug, Clone)]
pub struct CompileError {
    pub message: String,
    pub location: Location,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let l = &self.location;
        if l.element.is_empty() {
            write!(
                f,
                "compilation error: file {} line {}: {}",
                l.file, l.line, self.message
            )
        } else {
            write!(
                f,
                "compilation error: file {} line {} element {}\n{}",
                l.file, l.line, l.element, self.message
            )
        }
    }
}

/// Why compiling failed. `Parse` is a stylesheet that isn't well-formed XML
/// (xsltproc exit 4); `Compile` is an XSLT error in a well-formed one (exit 5).
#[derive(Debug)]
pub enum StylesheetError {
    Parse(String),
    Compile(Vec<CompileError>),
}

/// Compiles the stylesheet in `doc`, read from `file`. Included files are read
/// relative to it, with no catalogs.
pub fn compile(
    doc: &Document,
    file: &Path,
    names: &mut NamePool,
) -> Result<Stylesheet, StylesheetError> {
    compile_with(
        doc,
        file,
        names,
        &ResourceLoader::local(),
        &ParseOptions::default(),
        &mut Vec::new(),
    )
}

/// Compiles the stylesheet in `doc`, loading included files through
/// `loader` (catalogs, `--path`) and parsing them with `options`. Parser
/// warnings from included files are added to `warnings`.
pub fn compile_with(
    doc: &Document,
    file: &Path,
    names: &mut NamePool,
    loader: &ResourceLoader,
    options: &ParseOptions,
    warnings: &mut Vec<String>,
) -> Result<Stylesheet, StylesheetError> {
    let xslt = names.intern(XSLT_NS);
    let xml = names.intern(XML_NS);
    let rewritten_calls = [
        names.intern("format-number"),
        names.intern("key"),
        names.intern("function-available"),
        names.intern("element-available"),
    ];
    let mut c = Compiler {
        names,
        xslt,
        xml,
        rewritten_calls,
        ss: Stylesheet::default(),
        order: 0,
        errors: Vec::new(),
        aliases: HashMap::new(),
        including: vec![file.to_path_buf()],
        leaked_exclusions: Vec::new(),
        precedence: 0,
        modules: vec![Vec::new()],
        global_modules: Vec::new(),
        global_variables: HashSet::new(),
        module_sets: HashMap::new(),
        cdata_modules: Vec::new(),
        scopes: Default::default(),
        scope_scratch: Default::default(),
        prefix_scopes: Default::default(),
        element_names: Default::default(),
        loader,
        options,
        warnings,
    };
    c.stylesheet(doc, file)?;
    if !c.errors.is_empty() {
        return Err(StylesheetError::Compile(c.errors));
    }
    c.assign_precedence();
    c.resolve_attribute_sets();
    c.order_globals();
    if !c.errors.is_empty() {
        return Err(StylesheetError::Compile(c.errors));
    }
    Ok(c.ss)
}

struct Compiler<'n> {
    names: &'n mut NamePool,
    loader: &'n ResourceLoader,
    options: &'n ParseOptions,
    warnings: &'n mut Vec<String>,
    xslt: Sym,
    xml: Sym,
    /// format-number, key, function-available and element-available, whose
    /// literal QName arguments `xpath` resolves.
    rewritten_calls: [Sym; 4],
    ss: Stylesheet,
    /// Position of each top-level declaration, for template conflict resolution.
    order: usize,
    errors: Vec<CompileError>,
    /// `xsl:namespace-alias`: stylesheet URI to result (prefix, URI).
    aliases: HashMap<Sym, Binding>,
    /// Files being compiled, to detect include and import cycles.
    including: Vec<PathBuf>,
    /// URIs excluded by the top-level element of every module compiled so
    /// far in this import unit. libxslt pushes them onto the stylesheet's
    /// exclusion list when it starts a module and never pops them, so an
    /// included module's exclude-result-prefixes also applies to the
    /// including module's later templates. Each xsl:import starts afresh.
    leaked_exclusions: Vec<Sym>,
    /// The module being compiled. While compiling, every `precedence`
    /// field holds a module number; `assign_precedence` turns them into
    /// import precedences once the import tree is known.
    precedence: usize,
    /// Each module's imports, in the order they're declared.
    modules: Vec<Vec<usize>>,
    /// The module of each entry in `ss.globals`.
    global_modules: Vec<usize>,
    /// (module, name) of each global `xsl:variable`, for libxslt's
    /// "redefinition of global variable" check.
    global_variables: HashSet<(usize, Name)>,
    /// Attribute sets by name, one per module that defines the name.
    module_sets: HashMap<Name, Vec<(usize, AttributeSet)>>,
    /// `cdata-section-elements` names, with the module declaring each.
    cdata_modules: Vec<(usize, Name)>,
    /// Shared copies of in-scope namespace lists, for locations.
    scopes: std::cell::RefCell<crate::tree::SymMap<Vec<Binding>, std::rc::Rc<[Binding]>>>,
    /// Scratch space for looking a namespace list up in `scopes`.
    scope_scratch: std::cell::RefCell<Vec<Binding>>,
    /// Each list in `scopes`, by address, as (prefix, URI) for XPath. The
    /// lists live as long as the compiler, so their addresses stay unique.
    prefix_scopes: std::cell::RefCell<crate::tree::SymMap<usize, PrefixScope>>,
    /// Shared element names, for locations.
    element_names: std::cell::RefCell<crate::tree::SymMap<Sym, std::rc::Rc<str>>>,
}

/// One stylesheet document being compiled.
struct Doc<'d> {
    doc: &'d Document,
    file: std::rc::Rc<str>,
    path: PathBuf,
    forwards_compatible: bool,
    /// The module's URL, as libxml2 records it.
    url: std::rc::Rc<str>,
    /// Whether any element has an xml:base attribute; if none does, every
    /// element's base URI is the URL.
    has_xml_base: bool,
    /// Each element's in-scope namespaces, shared with its parent when it
    /// declares none (see `Compiler::shared_scope`).
    scope_memo: std::cell::RefCell<crate::tree::SymMap<NodeId, std::rc::Rc<[Binding]>>>,
    /// Whether `xml:space="preserve"` is in effect at each element.
    space_memo: std::cell::RefCell<crate::tree::SymMap<NodeId, bool>>,
}

type R<T> = Result<T, CompileError>;

/// In-scope prefixed namespaces as (prefix, URI), for compiling XPath.
type PrefixScope = std::rc::Rc<[(String, Sym)]>;

impl Compiler<'_> {
    // ---------- tree helpers ----------

    fn local(&self, doc: &Document, el: NodeId) -> Option<&str> {
        doc.name(el).map(|q| self.names.str(q.local))
    }

    fn is_xslt(&self, doc: &Document, el: NodeId) -> bool {
        matches!(doc.kind(el), NodeKind::Element { .. })
            && doc.name(el).is_some_and(|q| q.ns == Some(self.xslt))
    }

    fn is_xslt_named(&self, doc: &Document, el: NodeId, local: &str) -> bool {
        self.is_xslt(doc, el) && self.local(doc, el) == Some(local)
    }

    /// An unprefixed attribute's value.
    fn attr<'d>(&self, doc: &'d Document, el: NodeId, local: &str) -> Option<&'d str> {
        doc.attributes(el).find_map(|a| {
            let q = doc.name(a)?;
            (q.ns.is_none() && self.names.str(q.local) == local).then(|| doc.value(a).unwrap_or(""))
        })
    }

    /// An attribute in the XSLT namespace (`xsl:version` on a literal result element).
    fn xsl_attr<'d>(&self, doc: &'d Document, el: NodeId, local: &str) -> Option<&'d str> {
        doc.attributes(el).find_map(|a| {
            let q = doc.name(a)?;
            (q.ns == Some(self.xslt) && self.names.str(q.local) == local)
                .then(|| doc.value(a).unwrap_or(""))
        })
    }

    fn location(&self, d: &Doc, el: NodeId) -> Location {
        let element = match d.doc.name(el) {
            Some(q) => self
                .element_names
                .borrow_mut()
                .entry(q.local)
                .or_insert_with(|| self.names.str(q.local).into())
                .clone(),
            None => "".into(),
        };
        let base = if d.has_xml_base {
            let names = Some((self.xml, NamePool::BASE));
            match &d.doc.url {
                Some(_) => d.doc.base_uri(el, names).map(Into::into),
                None => d
                    .doc
                    .base_uri(el, names)
                    .map(|b| crate::uri::build(&b, Some(&d.url)).into()),
            }
            .unwrap_or_else(|| d.url.clone())
        } else {
            d.url.clone()
        };
        let namespaces = self.shared_scope(d, el);
        Location {
            file: d.file.clone(),
            line: d.doc.line(el),
            element,
            base,
            namespaces,
        }
    }

    fn error(&self, d: &Doc, el: NodeId, message: impl Into<String>) -> CompileError {
        CompileError {
            message: message.into(),
            location: self.location(d, el),
        }
    }

    /// In-scope namespaces of a stylesheet element, nearest declaration first.
    /// An undeclared default namespace (`xmlns=""`) is reported with an empty URI.
    fn in_scope(&self, doc: &Document, el: NodeId) -> Vec<Binding> {
        let mut out = Vec::new();
        in_scope_into(doc, el, &mut out);
        out
    }

    /// [`Self::in_scope`], shared: one list for every element with the same
    /// namespaces in scope.
    fn shared_scope(&self, d: &Doc, el: NodeId) -> std::rc::Rc<[Binding]> {
        if let Some(rc) = d.scope_memo.borrow().get(&el) {
            return rc.clone();
        }
        // An element that declares nothing has its parent's namespaces.
        let rc = match d.doc.parent(el) {
            Some(parent) if d.doc.ns_decls(el).is_empty() => self.shared_scope(d, parent),
            _ => self.scope_list(d.doc, el),
        };
        d.scope_memo.borrow_mut().insert(el, rc.clone());
        rc
    }

    /// The shared copy of `el`'s in-scope namespace list.
    fn scope_list(&self, doc: &Document, el: NodeId) -> std::rc::Rc<[Binding]> {
        let mut scratch = self.scope_scratch.borrow_mut();
        scratch.clear();
        in_scope_into(doc, el, &mut scratch);
        let mut scopes = self.scopes.borrow_mut();
        if let Some(rc) = scopes.get(scratch.as_slice()) {
            return rc.clone();
        }
        let rc: std::rc::Rc<[Binding]> = scratch.as_slice().into();
        scopes.insert(scratch.clone(), rc.clone());
        rc
    }

    /// The prefixed bindings in scope, as (prefix, URI), for XPath.
    fn prefix_scope(&self, d: &Doc, el: NodeId) -> PrefixScope {
        let scope = self.shared_scope(d, el);
        self.prefix_scopes
            .borrow_mut()
            .entry(std::rc::Rc::as_ptr(&scope) as *const Binding as usize)
            .or_insert_with(|| {
                scope
                    .iter()
                    .filter_map(|&(p, u)| p.map(|p| (self.names.str(p).to_string(), u)))
                    .collect()
            })
            .clone()
    }

    fn resolve_prefix(&self, scope: &[Binding], prefix: &str) -> Option<Sym> {
        if prefix == "xml" {
            return Some(self.xml);
        }
        scope
            .iter()
            .find(|(p, _)| p.is_some_and(|p| self.names.str(p) == prefix))
            .map(|&(_, uri)| uri)
    }

    /// A QName attribute value (template name, mode, variable name).
    fn qname(&mut self, d: &Doc, el: NodeId, value: &str) -> R<Name> {
        let value = value.trim();
        let (prefix, local) = match value.split_once(':') {
            Some((p, l)) => (Some(p), l),
            None => (None, value),
        };
        if local.is_empty() || prefix.is_some_and(str::is_empty) {
            return Err(self.error(d, el, format!("Invalid QName '{value}'")));
        }
        let ns =
            match prefix {
                None => None,
                Some(p) => {
                    let scope = self.in_scope(d.doc, el);
                    Some(self.resolve_prefix(&scope, p).ok_or_else(|| {
                        self.error(d, el, format!("Undefined namespace prefix {p}"))
                    })?)
                }
            };
        Ok((ns, self.names.intern(local)))
    }

    /// A pattern, compiled as an expression. libxslt's own pattern compiler
    /// doesn't accept the extra slashes libxml2's XPath compiler skips
    /// (`///x`, `/ /x`), so they're rejected here first.
    fn pattern_xpath(&mut self, d: &Doc, el: NodeId, text: &str) -> R<Expr> {
        use xpath::lexer::Tok;
        if let Ok(toks) = xpath::lexer::tokenize(text) {
            let slash = |t: &xpath::lexer::Token| matches!(t.tok, Tok::Slash | Tok::SlashSlash);
            if toks.windows(2).any(|w| slash(&w[0]) && slash(&w[1])) {
                return Err(self.error(d, el, format!("Failed to compile pattern '{text}'")));
            }
        }
        self.xpath(d, el, text)
    }

    fn xpath(&mut self, d: &Doc, el: NodeId, expr: &str) -> R<Expr> {
        let scope = self.prefix_scope(d, el);
        let xml = self.xml;
        let resolve = |p: &str| {
            if p == "xml" {
                return Some(xml);
            }
            scope.iter().find(|(q, _)| q == p).map(|&(_, u)| u)
        };
        let mut compiled = xpath::compile(expr, self.names, &resolve)
            .map_err(|e| self.error(d, el, format!("XPath error: {e} in '{expr}'")))?;
        // format-number()'s decimal-format name and key()'s key name are
        // QNames resolved in the instruction's namespace context. When one is
        // a literal, resolve it now into {uri}local form, which the functions
        // understand.
        let [format_number, key, function_available, element_available] = self.rewritten_calls;
        let mut unresolved = None;
        rewrite_calls(&mut compiled, &mut |e| {
            if let Expr::Call(xpath::ast::Function::Other((None, f)), args) = e {
                let (index, what) = match *f {
                    f if f == format_number => (2, "format-number"),
                    f if f == key => (0, "key"),
                    f if f == function_available => (0, "function-available"),
                    f if f == element_available => (0, "element-available"),
                    _ => return,
                };
                if let Some(Expr::Literal(name)) = args.get_mut(index)
                    && let Some((prefix, local)) = name.trim().split_once(':')
                {
                    match resolve(prefix) {
                        Some(uri) => *name = format!("{{{}}}{local}", self.names.str(uri)),
                        // libxslt reports these two at runtime.
                        None if what.ends_with("-available") => {}
                        None => unresolved = Some((what, prefix.to_string())),
                    }
                }
            }
        });
        if let Some((what, p)) = unresolved {
            return Err(self.error(
                d,
                el,
                format!("{what} : No namespace found for QName prefix '{p}'"),
            ));
        }
        Ok(compiled)
    }

    fn required<'d>(&self, d: &Doc<'d>, el: NodeId, local: &str) -> R<&'d str> {
        self.attr(d.doc, el, local)
            .ok_or_else(|| self.error(d, el, format!("Attribute '{local}' is required")))
    }

    fn avt(&mut self, d: &Doc, el: NodeId, value: &str) -> R<Avt> {
        if !value.contains(['{', '}']) {
            return Ok(Avt::Literal(value.to_string()));
        }
        let mut parts = Vec::new();
        let mut lit = String::new();
        let mut chars = value.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            match c {
                '{' if chars.peek().map(|&(_, n)| n) == Some('{') => {
                    chars.next();
                    lit.push('{');
                }
                '}' if chars.peek().map(|&(_, n)| n) == Some('}') => {
                    chars.next();
                    lit.push('}');
                }
                '}' => {
                    return Err(self.error(
                        d,
                        el,
                        format!("Unmatched '}}' in attribute value template '{value}'"),
                    ));
                }
                '{' => {
                    // Scan to the closing brace, skipping string literals.
                    let start = i + 1;
                    let mut quote = None;
                    let mut end = None;
                    for (j, c) in chars.by_ref() {
                        match (quote, c) {
                            (None, '"' | '\'') => quote = Some(c),
                            (Some(q), c) if c == q => quote = None,
                            (None, '}') => {
                                end = Some(j);
                                break;
                            }
                            _ => {}
                        }
                    }
                    let end = end.ok_or_else(|| {
                        self.error(
                            d,
                            el,
                            format!("Unclosed '{{' in attribute value template '{value}'"),
                        )
                    })?;
                    if !lit.is_empty() {
                        parts.push(AvtPart::Literal(std::mem::take(&mut lit)));
                    }
                    let expr = self.xpath(d, el, &value[start..end])?;
                    parts.push(AvtPart::Expr(expr));
                }
                c => lit.push(c),
            }
        }
        if !lit.is_empty() {
            parts.push(AvtPart::Literal(lit));
        }
        Ok(Avt::Parts(parts))
    }

    fn opt_avt(&mut self, d: &Doc, el: NodeId, local: &str) -> R<Option<Avt>> {
        match self.attr(d.doc, el, local) {
            Some(v) => Ok(Some(self.avt(d, el, v)?)),
            None => Ok(None),
        }
    }

    fn qname_list(&mut self, d: &Doc, el: NodeId, value: Option<&str>) -> R<Vec<Name>> {
        let Some(value) = value else {
            return Ok(Vec::new());
        };
        value
            .split_ascii_whitespace()
            .map(|q| self.qname(d, el, q))
            .collect()
    }

    /// Whitespace-only stylesheet text is kept only inside `xsl:text` or
    /// under `xml:space="preserve"`.
    /// Whether the nearest `xml:space` at or above `el` says "preserve".
    fn preserves_space(&self, d: &Doc, el: NodeId) -> bool {
        if let Some(&p) = d.space_memo.borrow().get(&el) {
            return p;
        }
        let own = d.doc.attributes(el).find_map(|a| {
            let q = d.doc.name(a).unwrap();
            (q.ns == Some(self.xml) && q.local == NamePool::SPACE)
                .then(|| d.doc.value(a) == Some("preserve"))
        });
        let p = match own {
            Some(p) => p,
            None => d
                .doc
                .parent(el)
                .is_some_and(|parent| self.preserves_space(d, parent)),
        };
        d.space_memo.borrow_mut().insert(el, p);
        p
    }

    /// Namespace URIs excluded from literal result elements at `el`: the XSLT
    /// namespace, plus `exclude-result-prefixes` and `extension-element-prefixes`
    /// on `el`, its ancestors and the `xsl:stylesheet` element.
    fn excluded(&self, d: &Doc, el: NodeId) -> HashSet<Sym> {
        self.excluded_uris(d, el, true)
    }

    /// The excluded URIs; `with_extensions` adds the extension namespaces'
    /// URIs (for an element's own declarations).
    fn excluded_uris(&self, d: &Doc, el: NodeId, with_extensions: bool) -> HashSet<Sym> {
        let mut out = HashSet::from([self.xslt]);
        out.extend(self.leaked_exclusions.iter().copied());
        let attrs: &[&str] = if with_extensions {
            &["exclude-result-prefixes", "extension-element-prefixes"]
        } else {
            &["exclude-result-prefixes"]
        };
        let mut cur = Some(el);
        while let Some(e) = cur {
            if matches!(d.doc.kind(e), NodeKind::Element { .. }) {
                let scope = self.in_scope(d.doc, e);
                let is_xslt = self.is_xslt(d.doc, e);
                for &attr in attrs {
                    let v = if is_xslt {
                        self.attr(d.doc, e, attr)
                    } else {
                        self.xsl_attr(d.doc, e, attr)
                    };
                    for p in v.unwrap_or("").split_ascii_whitespace() {
                        let uri = if p == "#default" {
                            scope.iter().find(|(q, _)| q.is_none()).map(|&(_, u)| u)
                        } else {
                            self.resolve_prefix(&scope, p)
                        };
                        out.extend(uri);
                    }
                }
            }
            cur = d.doc.parent(e);
        }
        out
    }

    /// `xsl:namespace-alias` as libxslt applies it (xsltGetNamespace,
    /// xsltCopyNamespaceList): the URI is replaced, the stylesheet's prefix
    /// is kept. An alias to `#default` with no default namespace means none.
    fn alias(&self, ns: Option<Sym>, prefix: Option<Sym>) -> (Option<Sym>, Option<Sym>) {
        match ns.and_then(|u| self.aliases.get(&u)) {
            Some(&(_, uri)) => {
                if self.names.str(uri).is_empty() {
                    (None, None)
                } else {
                    (Some(uri), prefix)
                }
            }
            None => (ns, prefix),
        }
    }

    // ---------- top level ----------

    /// Compiles one stylesheet document, into the current module.
    fn stylesheet(&mut self, doc: &Document, file: &Path) -> Result<(), StylesheetError> {
        let display = file.file_name().map_or_else(
            || file.display().to_string(),
            |f| f.to_string_lossy().into_owned(),
        );
        let root = doc
            .children(doc.root())
            .find(|&c| matches!(doc.kind(c), NodeKind::Element { .. }));
        let Some(root) = root else {
            return Err(StylesheetError::Parse("no document element".into()));
        };
        let version = if self.is_xslt(doc, root) {
            self.attr(doc, root, "version")
        } else {
            self.xsl_attr(doc, root, "version")
        };
        let forwards_compatible = version.is_some_and(|v| v.trim() != "1.0");
        let url: std::rc::Rc<str> = doc
            .url
            .clone()
            .unwrap_or_else(|| file.to_string_lossy().into_owned())
            .into();
        let xml_base = Some(NamePool::BASE);
        let has_xml_base = xml_base.is_some()
            && doc.all_nodes().any(|n| {
                doc.name(n).is_some_and(|q| {
                    q.ns == Some(self.xml)
                        && Some(q.local) == xml_base
                        && matches!(doc.kind(n), NodeKind::Attribute { .. })
                })
            });
        let d = Doc {
            doc,
            file: display.into(),
            path: file.to_path_buf(),
            forwards_compatible,
            url,
            has_xml_base,
            scope_memo: Default::default(),
            space_memo: Default::default(),
        };

        // The module's own exclusions join the import unit's list
        // (xsltParseStylesheetExcludePrefix on the document element).
        let own = if self.is_xslt(doc, root) {
            self.attr(doc, root, "exclude-result-prefixes")
        } else {
            self.xsl_attr(doc, root, "exclude-result-prefixes")
        };
        if let Some(prefixes) = own {
            let scope = self.in_scope(doc, root);
            for p in prefixes.split_ascii_whitespace() {
                let uri = if p == "#default" {
                    scope.iter().find(|(q, _)| q.is_none()).map(|&(_, u)| u)
                } else {
                    self.resolve_prefix(&scope, p)
                };
                self.leaked_exclusions.extend(uri);
            }
        }

        if !self.is_xslt(doc, root) {
            // A simplified stylesheet: the literal result element is the
            // body of a template matching "/".
            if version.is_none() {
                self.errors.push(self.error(
                    &d,
                    root,
                    "The document element isn't xsl:stylesheet and has no xsl:version",
                ));
                return Ok(());
            }
            match self.literal_element(&d, root) {
                Ok(instr) => {
                    let expr = Expr::Path(xpath::ast::Path {
                        start: xpath::ast::PathStart::Root,
                        steps: vec![],
                    });
                    let matches = pattern::from_expr(expr, false).ok();
                    self.add_template(
                        Template {
                            matches,
                            name: None,
                            mode: None,
                            params: vec![],
                            body: vec![instr],
                            location: self.location(&d, root),
                            precedence: 0,
                        },
                        None,
                    );
                }
                Err(e) => self.errors.push(e),
            }
            return Ok(());
        }
        let local = self.local(doc, root).unwrap_or("");
        if local != "stylesheet" && local != "transform" {
            self.errors.push(self.error(
                &d,
                root,
                format!("Unexpected document element xsl:{local}"),
            ));
            return Ok(());
        }

        // Aliases apply to the whole stylesheet, wherever they're declared.
        for child in doc.children(root) {
            if self.is_xslt_named(doc, child, "namespace-alias")
                && let Err(e) = self.namespace_alias(&d, child)
            {
                self.errors.push(e);
            }
        }
        // Imports come first. An included file's imports become imports of
        // the including module, after the ones it already has, as in
        // libxslt.
        let children: Vec<NodeId> = doc.children(root).collect();
        let (imports, rest): (Vec<NodeId>, Vec<NodeId>) = children
            .iter()
            .partition(|&&c| self.is_xslt_named(doc, c, "import"));
        let mut seen = false;
        for &child in &children {
            if imports.contains(&child) {
                if seen {
                    self.errors.push(self.error(
                        &d,
                        child,
                        "xsl:import must come before any other top-level element",
                    ));
                } else if let Err(e) = self.import(&d, child) {
                    self.errors.push(e);
                }
            } else if matches!(doc.kind(child), NodeKind::Element { .. }) {
                seen = true;
            }
        }
        for child in rest {
            if let Err(e) = self.top_level(&d, child) {
                self.errors.push(e);
            }
        }
        Ok(())
    }

    fn namespace_alias(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let scope = self.in_scope(d.doc, el);
        let lookup = |c: &Self, p: &str| -> Option<(Option<Sym>, Sym)> {
            if p == "#default" {
                let uri = scope
                    .iter()
                    .find(|(q, _)| q.is_none())
                    .map_or_else(|| c.names.get("").unwrap_or(c.xslt), |&(_, u)| u);
                Some((None, uri))
            } else {
                let uri = c.resolve_prefix(&scope, p)?;
                let prefix = scope
                    .iter()
                    .find(|(q, u)| *u == uri && q.is_some_and(|q| c.names.str(q) == p))
                    .and_then(|&(q, _)| q);
                Some((prefix, uri))
            }
        };
        let from = self.required(d, el, "stylesheet-prefix")?;
        let to = self.required(d, el, "result-prefix")?;
        let (_, from_uri) = lookup(self, from)
            .ok_or_else(|| self.error(d, el, format!("Undefined namespace prefix {from}")))?;
        let (to_prefix, to_uri) = if to == "#default" && !scope.iter().any(|(q, _)| q.is_none()) {
            (None, self.names.intern(""))
        } else {
            lookup(self, to)
                .ok_or_else(|| self.error(d, el, format!("Undefined namespace prefix {to}")))?
        };
        self.aliases.insert(from_uri, (to_prefix, to_uri));
        Ok(())
    }

    fn top_level(&mut self, d: &Doc, el: NodeId) -> R<()> {
        if !matches!(d.doc.kind(el), NodeKind::Element { .. }) {
            if let NodeKind::Text(_) = d.doc.kind(el)
                && !d.doc.value(el).unwrap_or("").trim().is_empty()
            {
                return Err(self.error(
                    d,
                    el,
                    "Text isn't allowed at the top level of a stylesheet",
                ));
            }
            return Ok(());
        }
        if !self.is_xslt(d.doc, el) {
            // Top-level elements in other namespaces are ignored; with no
            // namespace, libxslt rejects them.
            if d.doc.name(el).is_some_and(|q| q.ns.is_none()) {
                let name = d.doc.name(el).map(|q| self.names.str(q.local).to_string());
                return Err(self.error(
                    d,
                    el,
                    format!(
                        "Found a top-level element {} with null namespace URI",
                        name.unwrap_or_default()
                    ),
                ));
            }
            return Ok(());
        }
        self.order += 1;
        let local = self.local(d.doc, el).unwrap_or("").to_string();
        match local.as_str() {
            "template" => self.template(d, el),
            "variable" | "param" => {
                if local == "variable"
                    && let Some(name) = self.attr(d.doc, el, "name")
                {
                    let name = self.qname(d, el, name)?;
                    self.ss.global_variable_names.insert(name);
                    // libxslt's xsltRegisterGlobalVariable: two xsl:variables
                    // of one name in a stylesheet and its includes are an
                    // error; params, or a param and a variable, aren't.
                    if !self.global_variables.insert((self.precedence, name)) {
                        let shown = self.attr(d.doc, el, "name").unwrap_or("").to_string();
                        return Err(self.error(
                            d,
                            el,
                            format!("redefinition of global variable {shown}"),
                        ));
                    }
                }
                let v = self.variable(d, el)?;
                self.ss.globals.push(v);
                self.global_modules.push(self.precedence);
                Ok(())
            }
            "output" => self.output(d, el),
            "strip-space" | "preserve-space" => self.space(d, el, local == "strip-space"),
            "include" => self.include(d, el),
            "attribute-set" => self.attribute_set(d, el),
            "decimal-format" => self.decimal_format(d, el),
            "namespace-alias" => Ok(()),
            "import" => Ok(()), // compiled first, by stylesheet()
            "key" => self.key(d, el),
            // Even in forwards-compatible mode: libxslt 1.1.35's
            // xsltParseStylesheetTop counts an unknown top-level XSLT element
            // as an error, unlike an unknown instruction in a template.
            _ => Err(self.error(
                d,
                el,
                format!("xsltParseStylesheetTop: unknown {local} element"),
            )),
        }
    }

    fn add_template(&mut self, mut t: Template, priority: Option<f64>) {
        t.precedence = self.precedence;
        let index = self.ss.templates.len();
        if let Some(p) = &t.matches {
            let rules = self.ss.rules.entry(t.mode).or_default();
            for (alternative, alt) in p.alternatives.iter().enumerate() {
                rules.push(Rule {
                    template: index,
                    alternative,
                    priority: priority.unwrap_or(alt.default_priority),
                    order: self.order,
                    precedence: self.precedence,
                });
            }
        }
        self.ss.templates.push(t);
    }

    fn template(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let matches = match self.attr(d.doc, el, "match") {
            Some(m) => {
                let expr = self.pattern_xpath(d, el, m)?;
                Some(
                    pattern::from_expr(expr, false)
                        .map_err(|e| self.error(d, el, format!("{} '{m}'", e.0)))?,
                )
            }
            None => None,
        };
        let name = match self.attr(d.doc, el, "name") {
            Some(n) => Some(self.qname(d, el, n)?),
            None => None,
        };
        if matches.is_none() && name.is_none() {
            return Err(self.error(d, el, "xsl:template needs a match or name attribute"));
        }
        let mode = match self.attr(d.doc, el, "mode") {
            Some(m) => Some(self.qname(d, el, m)?),
            None => None,
        };
        let priority = match self.attr(d.doc, el, "priority") {
            Some(p) => {
                let n = xpath::number::from_string(p);
                if n.is_nan() {
                    return Err(self.error(d, el, format!("Invalid priority '{p}'")));
                }
                Some(n)
            }
            None => None,
        };
        let mut params = Vec::new();
        let mut rest = Vec::new();
        let mut in_params = true;
        for child in d.doc.children(el) {
            if in_params && self.is_xslt_named(d.doc, child, "param") {
                params.push(self.variable(d, child)?);
                continue;
            }
            if matches!(d.doc.kind(child), NodeKind::Element { .. })
                || !self.ignorable_text(d, child, el)
            {
                in_params = false;
            }
            rest.push(child);
        }
        let body = self.nodes(d, el, &rest)?;
        let location = self.location(d, el);
        self.add_template(
            Template {
                matches,
                name,
                mode,
                params,
                body,
                location,
                precedence: 0,
            },
            priority,
        );
        Ok(())
    }

    fn variable(&mut self, d: &Doc, el: NodeId) -> R<Variable> {
        let name = self.required(d, el, "name")?;
        let name = self.qname(d, el, name)?;
        let value = match self.attr(d.doc, el, "select") {
            Some(s) => VarValue::Select(self.xpath(d, el, s)?),
            None => {
                let body = self.body(d, el)?;
                if body.is_empty() {
                    VarValue::Empty
                } else {
                    VarValue::Body(body)
                }
            }
        };
        Ok(Variable {
            name,
            value,
            location: self.location(d, el),
        })
    }

    /// `xsl:decimal-format`, as libxslt's xsltParseStylesheetDecimalFormat:
    /// an unnamed one changes the default format; a second one with the same
    /// name is ignored.
    fn decimal_format(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let mut format = match self.attr(d.doc, el, "name") {
            Some(n) => {
                let name = self.qname(d, el, n)?;
                if self.ss.decimal_formats.contains_key(&name) {
                    return Ok(());
                }
                Some((name, super::number_format::DecimalFormat::default()))
            }
            None => None,
        };
        // libxslt's format-number uses only the main stylesheet's unnamed
        // format (style->decimalFormat): an imported one is ignored, even in
        // the imported module's own templates. A named format is looked up
        // through the imports, and its first declaration wins.
        if format.is_none() && self.precedence != 0 {
            return Ok(());
        }
        let target = match &mut format {
            Some((_, f)) => f,
            None => &mut self.ss.decimal_format,
        };
        let doc = d.doc;
        let xslt = self.xslt;
        let names = &*self.names;
        let get = |local: &str| {
            doc.attributes(el).find_map(|a| {
                let q = doc.name(a)?;
                (q.ns.is_none() && names.str(q.local) == local && Some(q.ns) != Some(Some(xslt)))
                    .then(|| doc.value(a).unwrap_or("").to_string())
            })
        };
        for (attr, field) in [
            ("decimal-separator", &mut target.decimal_point),
            ("grouping-separator", &mut target.grouping),
            ("infinity", &mut target.infinity),
            ("minus-sign", &mut target.minus_sign),
            ("NaN", &mut target.nan),
            ("percent", &mut target.percent),
            ("per-mille", &mut target.per_mille),
            ("zero-digit", &mut target.zero_digit),
            ("digit", &mut target.digit),
            ("pattern-separator", &mut target.pattern_separator),
        ] {
            if let Some(v) = get(attr) {
                *field = v;
            }
        }
        if let Some((name, f)) = format {
            self.ss.decimal_formats.insert(name, f);
        }
        Ok(())
    }

    fn output(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let doc = d.doc;
        if let Some(m) = self.attr(doc, el, "method") {
            self.ss.output.method = match m.trim() {
                "xml" => Method::Xml,
                "html" => Method::Html,
                "text" => Method::Text,
                // A QName method is an extension; treat it as xml.
                other if other.contains(':') => Method::Xml,
                other => {
                    return Err(self.error(d, el, format!("Unsupported output method '{other}'")));
                }
            };
        }
        let text = |c: &Self, name: &str| c.attr(doc, el, name).map(str::to_string);
        let flag = |c: &Self, name: &str| c.attr(doc, el, name).map(|v| v.trim() == "yes");
        // Each attribute overrides an earlier xsl:output's, independently.
        let o = Output {
            version: text(self, "version"),
            encoding: text(self, "encoding"),
            doctype_public: text(self, "doctype-public"),
            doctype_system: text(self, "doctype-system"),
            media_type: text(self, "media-type"),
            omit_xml_declaration: flag(self, "omit-xml-declaration"),
            standalone: flag(self, "standalone"),
            indent: flag(self, "indent"),
            ..Output::default()
        };
        // libxslt (the fix for bug #339570) expands an unprefixed name with the
        // default namespace in effect on xsl:output, unlike other QNames.
        let cdata_attr = self.attr(doc, el, "cdata-section-elements");
        let mut cdata = self.qname_list(d, el, cdata_attr)?;
        let default_ns = self
            .in_scope(doc, el)
            .into_iter()
            .find(|(p, _)| p.is_none())
            .map(|(_, u)| u);
        if let Some(uri) = default_ns.filter(|&u| !self.names.str(u).is_empty()) {
            for (raw, name) in cdata_attr
                .unwrap_or("")
                .split_ascii_whitespace()
                .zip(cdata.iter_mut())
            {
                if !raw.contains(':') {
                    name.0 = Some(uri);
                }
            }
        }
        let out = &mut self.ss.output;
        out.version = o.version.or(out.version.take());
        out.encoding = o.encoding.or(out.encoding.take());
        out.doctype_public = o.doctype_public.or(out.doctype_public.take());
        out.doctype_system = o.doctype_system.or(out.doctype_system.take());
        out.media_type = o.media_type.or(out.media_type.take());
        out.omit_xml_declaration = o.omit_xml_declaration.or(out.omit_xml_declaration);
        out.standalone = o.standalone.or(out.standalone);
        out.indent = o.indent.or(out.indent);
        let module = self.precedence;
        self.cdata_modules
            .extend(cdata.into_iter().map(|name| (module, name)));
        Ok(())
    }

    fn space(&mut self, d: &Doc, el: NodeId, strip: bool) -> R<()> {
        let elements = self.required(d, el, "elements")?;
        for token in elements.split_ascii_whitespace() {
            let test = if token == "*" {
                SpaceTest::Any
            } else if let Some(prefix) = token.strip_suffix(":*") {
                let scope = self.in_scope(d.doc, el);
                let uri = self.resolve_prefix(&scope, prefix).ok_or_else(|| {
                    self.error(d, el, format!("Undefined namespace prefix {prefix}"))
                })?;
                SpaceTest::Namespace(Some(uri))
            } else {
                SpaceTest::Name(self.qname(d, el, token)?)
            };
            self.ss.space.push(SpaceRule {
                strip,
                test,
                order: self.order,
                precedence: self.precedence,
            });
        }
        Ok(())
    }

    fn include(&mut self, d: &Doc, el: NodeId) -> R<()> {
        self.load_module(d, el, true)
    }

    /// `xsl:import`: a module of its own, with lower precedence than this
    /// one.
    fn import(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let saved = self.precedence;
        let module = self.modules.len();
        self.modules.push(Vec::new());
        self.modules[saved].push(module);
        self.precedence = module;
        let saved_exclusions = std::mem::take(&mut self.leaked_exclusions);
        let result = self.load_module(d, el, false);
        self.leaked_exclusions = saved_exclusions;
        self.precedence = saved;
        result
    }

    fn load_module(&mut self, d: &Doc, el: NodeId, included: bool) -> R<()> {
        let what = if included { "inclusion" } else { "import" };
        let href = self.required(d, el, "href")?;
        // Through the loader, so catalogs can map a URL to a local file.
        let (bytes, path, by_path) = self
            .loader
            .read_document_how(href, &Some(d.path.clone()))
            .map_err(|e| self.error(d, el, format!("Failed to load '{href}': {e}")))?;
        if self.including.contains(&path) {
            return Err(self.error(d, el, format!("Recursive {what} of '{href}'")));
        }
        let parsed = parse_with(
            &bytes,
            self.names,
            self.options,
            self.loader,
            &Some(path.clone()),
        )
        .map_err(|e| self.error(d, el, format!("Failed to parse '{href}': {e}")))?;
        self.warnings.extend(parsed.warnings);
        let mut doc = parsed.document;
        let base = self.location(d, el).base;
        // The URL as requested, so relative ones stay relative; but a
        // module found in a --path directory is named by the path found, as
        // libxml2 names it, and its own includes resolve from there.
        doc.url = Some(if by_path {
            path.to_string_lossy().into_owned()
        } else {
            crate::uri::build(href, Some(&base))
        });
        self.including.push(path.clone());
        let result = self.stylesheet(&doc, &path);
        self.including.pop();
        match result {
            Ok(()) => Ok(()),
            Err(StylesheetError::Parse(m)) => {
                Err(self.error(d, el, format!("Failed to parse '{href}': {m}")))
            }
            Err(StylesheetError::Compile(mut errs)) => {
                let first = errs.remove(0);
                self.errors.extend(errs);
                Err(first)
            }
        }
    }

    fn key(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let name = self.required(d, el, "name")?;
        let name = self.qname(d, el, name)?;
        let m = self.required(d, el, "match")?;
        let expr = self.pattern_xpath(d, el, m)?;
        let matches = pattern::from_expr(expr, false)
            .map_err(|e| self.error(d, el, format!("{} '{m}'", e.0)))?;
        let use_text = self.required(d, el, "use")?;
        let use_ = self.xpath(d, el, use_text)?;
        // libxslt compiles `use` with XML_XPATH_NOVAR too.
        if pattern::contains_variable(&use_) {
            return Err(self.error(
                d,
                el,
                format!("xsl:key : XPath compilation failed '{use_text}'"),
            ));
        }
        let location = self.location(d, el);
        self.ss.keys.entry(name).or_default().push(KeyDef {
            matches,
            use_,
            location,
            precedence: self.precedence,
            order: self.order,
        });
        Ok(())
    }

    fn attribute_set(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let name = self.required(d, el, "name")?;
        let name = self.qname(d, el, name)?;
        let uses = self.attr(d.doc, el, "use-attribute-sets");
        let uses = self.qname_list(d, el, uses)?;
        let mut attributes = Vec::new();
        for child in d.doc.children(el) {
            if self.is_xslt_named(d.doc, child, "attribute") {
                attributes.push(self.instruction(d, child)?);
            } else if matches!(d.doc.kind(child), NodeKind::Element { .. }) {
                return Err(self.error(
                    d,
                    child,
                    "Only xsl:attribute is allowed in xsl:attribute-set",
                ));
            }
        }
        // Several sets with the same name in one module merge, later
        // attributes winning. Modules merge in assign_precedence.
        let module = self.precedence;
        let defs = self.module_sets.entry(name).or_default();
        let set = match defs.iter_mut().find(|(m, _)| *m == module) {
            Some((_, set)) => set,
            None => {
                defs.push((
                    module,
                    AttributeSet {
                        uses: vec![],
                        attributes: vec![],
                    },
                ));
                &mut defs.last_mut().expect("just pushed").1
            }
        };
        set.uses.extend(uses);
        set.attributes.extend(attributes);
        Ok(())
    }

    // ---------- bodies ----------

    fn ignorable_text(&self, d: &Doc, node: NodeId, parent: NodeId) -> bool {
        let doc = d.doc;
        match doc.kind(node) {
            NodeKind::Text(_) => {
                doc.value(node)
                    .unwrap_or("")
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
                    && !self.is_xslt_named(doc, parent, "text")
                    && !self.preserves_space(d, parent)
            }
            NodeKind::Comment(_) | NodeKind::Pi { .. } => true,
            _ => false,
        }
    }

    fn body(&mut self, d: &Doc, el: NodeId) -> R<Body> {
        let children: Vec<NodeId> = d.doc.children(el).collect();
        self.nodes(d, el, &children)
    }

    fn nodes(&mut self, d: &Doc, parent: NodeId, nodes: &[NodeId]) -> R<Body> {
        let mut out = Vec::new();
        for &n in nodes {
            match d.doc.kind(n) {
                NodeKind::Text(_) => {
                    if !self.ignorable_text(d, n, parent) {
                        out.push(Instr::Text(d.doc.value(n).unwrap_or("").to_string()));
                    }
                }
                NodeKind::Element { .. } => {
                    if self.is_xslt(d.doc, n) {
                        if self.is_xslt_named(d.doc, n, "fallback") {
                            continue;
                        }
                        out.push(self.instruction(d, n)?);
                    } else if self.is_extension(d, n) {
                        match self.document_element(d, n)? {
                            Some(instr) => out.push(instr),
                            None => out.extend(self.fallback(d, n)?),
                        }
                    } else {
                        out.push(self.literal_element(d, n)?);
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Whether `el` is an extension element. libxslt (xsltCheckExtPrefix)
    /// compares the element's prefix, as written, with the declared
    /// extension prefixes; another prefix for the same URI doesn't count.
    fn is_extension(&self, d: &Doc, el: NodeId) -> bool {
        let Some(q) = d.doc.name(el) else {
            return false;
        };
        q.ns.is_some() && self.extension_prefixes(d, el).contains(&q.prefix)
    }

    /// The prefixes declared by `extension-element-prefixes` on `el` and
    /// its ancestors (`#default` as None).
    fn extension_prefixes(&self, d: &Doc, el: NodeId) -> HashSet<Option<Sym>> {
        let mut out = HashSet::new();
        let mut cur = Some(el);
        while let Some(e) = cur {
            if matches!(d.doc.kind(e), NodeKind::Element { .. }) {
                let v = if self.is_xslt(d.doc, e) {
                    self.attr(d.doc, e, "extension-element-prefixes")
                } else {
                    self.xsl_attr(d.doc, e, "extension-element-prefixes")
                };
                for p in v.unwrap_or("").split_ascii_whitespace() {
                    match p {
                        "#default" => {
                            out.insert(None);
                        }
                        p => {
                            if let Some(sym) = self.names.get(p) {
                                out.insert(Some(sym));
                            }
                        }
                    }
                }
            }
            cur = d.doc.parent(e);
        }
        out
    }

    /// `exsl:document` or `xt:document` (libxslt's xsltDocumentComp): None
    /// for other extension elements.
    fn document_element(&mut self, d: &Doc, el: NodeId) -> R<Option<Instr>> {
        let q = d.doc.name(el).unwrap();
        let ns = q.ns.map(|u| self.names.str(u)).unwrap_or("");
        let local = self.names.str(q.local);
        let saxon = ns == SAXON_NS && local == "output";
        if !saxon && (local != "document" || !DOCUMENT_ELEMENT_NAMESPACES.contains(&ns)) {
            return Ok(None);
        }
        let href = if saxon {
            // saxon:output's file: libxslt takes a static value at compile
            // time; an AVT makes it look for saxon:file or saxon:href
            // instead, which stylesheets don't have, and fail.
            match self.attr(d.doc, el, "file") {
                Some(v) if !v.contains('{') => Some(self.avt(d, el, v)?),
                _ => {
                    let in_saxon = |name: &str| {
                        d.doc.attributes(el).find_map(|a| {
                            let q = d.doc.name(a)?;
                            (q.ns.is_some_and(|u| self.names.str(u) == SAXON_NS)
                                && self.names.str(q.local) == name)
                                .then(|| d.doc.value(a).unwrap_or("").to_string())
                        })
                    };
                    match in_saxon("file").or_else(|| in_saxon("href")) {
                        Some(v) => Some(self.avt(d, el, &v)?),
                        None => None,
                    }
                }
            }
        } else {
            match self.attr(d.doc, el, "href") {
                Some(v) => Some(self.avt(d, el, v)?),
                None => None,
            }
        };
        let mut settings = Vec::new();
        for name in DOCUMENT_SETTINGS {
            if let Some(v) = self.attr(d.doc, el, name) {
                settings.push((name, self.avt(d, el, v)?));
            }
        }
        let body = self.body(d, el)?;
        Ok(Some(Instr::Document {
            href,
            settings,
            body,
            location: self.location(d, el),
        }))
    }

    /// An unknown extension element: its `xsl:fallback` children run instead.
    fn fallback(&mut self, d: &Doc, el: NodeId) -> R<Body> {
        let mut out = Vec::new();
        let mut found = false;
        for child in d.doc.children(el).collect::<Vec<_>>() {
            if self.is_xslt_named(d.doc, child, "fallback") {
                found = true;
                out.extend(self.body(d, child)?);
            }
        }
        if !found {
            let local = self.local(d.doc, el).unwrap_or("").to_string();
            out.push(Instr::Unsupported {
                what: format!("extension element {local}"),
                location: self.location(d, el),
            });
        }
        Ok(out)
    }

    /// A literal result element. Its namespace nodes are, as libxslt copies
    /// them: the declarations written on the element itself, then, only for
    /// an element directly in a template body ("first level"), the
    /// template's inherited namespaces. A literal element inside xsl:if,
    /// xsl:for-each or a variable doesn't inherit them, unlike the spec.
    fn literal_element(&mut self, d: &Doc, el: NodeId) -> R<Instr> {
        let q = d.doc.name(el).unwrap();
        let (ns, prefix) = self.alias(q.ns, q.prefix);
        let name = QName {
            ns,
            local: q.local,
            prefix,
        };
        // Exclusions written on this element apply to its descendants only.
        let excluded = match d.doc.parent(el) {
            Some(p) if matches!(d.doc.kind(p), NodeKind::Element { .. }) => self.excluded(d, p),
            _ => HashSet::from([self.xslt]),
        };
        let mut candidates: Vec<(Binding, bool)> = d
            .doc
            .ns_decls(el)
            .iter()
            .map(|n| ((n.prefix, n.uri), true))
            .collect();
        let parent = d.doc.parent(el);
        if let Some(t) = parent.filter(|&p| self.is_xslt_named(d.doc, p, "template")) {
            // xsltGetInheritedNsList: from xsl:template upwards, nearest first,
            // minus stylesheet-level exclusions and extension prefixes.
            // Extension namespaces are skipped by prefix
            // (xsltCheckExtPrefix), exclusions by URI.
            let template_excluded = self.excluded_uris(d, t, false);
            let extension = self.extension_prefixes(d, t);
            for b in self.in_scope(d.doc, t) {
                let ext = b.0.is_some() && extension.contains(&b.0);
                if !template_excluded.contains(&b.1) && !ext {
                    candidates.push((b, false));
                }
            }
        }
        let mut namespaces: Vec<Binding> = Vec::new();
        let mut inherited: Vec<Binding> = Vec::new();
        for ((p, uri), own) in candidates {
            if self.names.str(uri).is_empty()
                || (own && excluded.contains(&uri))
                || uri == self.xslt
            {
                continue;
            }
            if namespaces.iter().chain(&inherited).any(|(q, _)| *q == p) {
                continue;
            }
            let uri = match self.aliases.get(&uri) {
                Some(&(_, au)) => au,
                None => uri,
            };
            if self.names.str(uri).is_empty() {
                continue;
            }
            if own {
                namespaces.push((p, uri));
            } else {
                inherited.push((p, uri));
            }
        }
        let mut attributes = Vec::new();
        let mut attribute_sets = Vec::new();
        for a in d.doc.attributes(el).collect::<Vec<_>>() {
            let aq = d.doc.name(a).unwrap();
            let value = d.doc.value(a).unwrap_or("").to_string();
            if aq.ns == Some(self.xslt) {
                if self.names.str(aq.local) == "use-attribute-sets" {
                    attribute_sets = self.qname_list(d, el, Some(&value))?;
                }
                continue;
            }
            let (ans, aprefix) = self.alias(aq.ns, aq.prefix);
            let avt = self.avt(d, el, &value)?;
            attributes.push((
                QName {
                    ns: ans,
                    local: aq.local,
                    prefix: aprefix,
                },
                avt,
            ));
        }
        let body = self.body(d, el)?;
        let location = self.location(d, el);
        Ok(Instr::LiteralElement {
            name,
            namespaces,
            inherited,
            attributes,
            attribute_sets,
            body,
            location,
        })
    }

    fn sorts_and_params(
        &mut self,
        d: &Doc,
        el: NodeId,
    ) -> R<(Vec<Sort>, Vec<Variable>, Vec<NodeId>)> {
        let mut sorts = Vec::new();
        let mut params = Vec::new();
        let mut rest = Vec::new();
        for child in d.doc.children(el) {
            if self.is_xslt_named(d.doc, child, "sort") {
                let select = self.attr(d.doc, child, "select").unwrap_or(".").to_string();
                let select = self.xpath(d, child, &select)?;
                let data_type = self.opt_avt(d, child, "data-type")?;
                let order = self.opt_avt(d, child, "order")?;
                let case_order = self.opt_avt(d, child, "case-order")?;
                // `lang` is ignored for now: the reference xsltproc has no
                // locales installed, so it sorts as without one.
                sorts.push(Sort {
                    select,
                    data_type,
                    order,
                    case_order,
                });
            } else if self.is_xslt_named(d.doc, child, "with-param") {
                params.push(self.variable(d, child)?);
            } else {
                rest.push(child);
            }
        }
        Ok((sorts, params, rest))
    }

    fn instruction(&mut self, d: &Doc, el: NodeId) -> R<Instr> {
        let local = self.local(d.doc, el).unwrap_or("").to_string();
        let location = self.location(d, el);
        let doc = d.doc;
        Ok(match local.as_str() {
            "apply-templates" => {
                let select = match self.attr(doc, el, "select") {
                    Some(s) => Some(self.xpath(d, el, s)?),
                    None => None,
                };
                let mode = match self.attr(doc, el, "mode") {
                    Some(m) => Some(self.qname(d, el, m)?),
                    None => None,
                };
                let (sorts, params, _) = self.sorts_and_params(d, el)?;
                Instr::ApplyTemplates {
                    select,
                    mode,
                    sorts,
                    params,
                    location,
                }
            }
            "call-template" => {
                let name = self.required(d, el, "name")?;
                let name = self.qname(d, el, name)?;
                let (_, params, _) = self.sorts_and_params(d, el)?;
                Instr::CallTemplate {
                    name,
                    params,
                    location,
                }
            }
            "value-of" => {
                let select = self.required(d, el, "select")?;
                let unescaped = self
                    .attr(doc, el, "disable-output-escaping")
                    .is_some_and(|v| v.trim() == "yes");
                Instr::ValueOf {
                    select: self.xpath(d, el, select)?,
                    unescaped,
                    location,
                }
            }
            "for-each" => {
                let select = self.required(d, el, "select")?;
                let select = self.xpath(d, el, select)?;
                let (sorts, _, rest) = self.sorts_and_params(d, el)?;
                let body = self.nodes(d, el, &rest)?;
                Instr::ForEach {
                    select,
                    sorts,
                    body,
                    location,
                }
            }
            "if" => {
                let test = self.required(d, el, "test")?;
                Instr::If {
                    test: self.xpath(d, el, test)?,
                    body: self.body(d, el)?,
                    location,
                }
            }
            "choose" => {
                let mut whens = Vec::new();
                let mut otherwise = None;
                for child in doc.children(el) {
                    if self.is_xslt_named(doc, child, "when") {
                        let test = self.required(d, child, "test")?;
                        let test = self.xpath(d, child, test)?;
                        let location = self.location(d, child);
                        whens.push((test, self.body(d, child)?, location));
                    } else if self.is_xslt_named(doc, child, "otherwise") {
                        otherwise = Some(self.body(d, child)?);
                    } else if matches!(doc.kind(child), NodeKind::Element { .. }) {
                        return Err(self.error(
                            d,
                            child,
                            "Only xsl:when and xsl:otherwise are allowed in xsl:choose",
                        ));
                    }
                }
                if whens.is_empty() {
                    return Err(self.error(d, el, "xsl:choose needs at least one xsl:when"));
                }
                Instr::Choose { whens, otherwise }
            }
            "text" => {
                let mut s = String::new();
                for child in doc.children(el) {
                    match doc.kind(child) {
                        NodeKind::Text(_) => s.push_str(doc.value(child).unwrap_or("")),
                        NodeKind::Element { .. } => {
                            return Err(self.error(d, child, "xsl:text can only contain text"));
                        }
                        _ => {}
                    }
                }
                if self
                    .attr(doc, el, "disable-output-escaping")
                    .is_some_and(|v| v.trim() == "yes")
                {
                    Instr::UnescapedText(s)
                } else {
                    Instr::Text(s)
                }
            }
            "variable" => Instr::Variable(self.variable(d, el)?),
            "param" => {
                return Err(self.error(
                    d,
                    el,
                    "xsl:param is only allowed at the top level or first in xsl:template",
                ));
            }
            "copy" => {
                let sets = self.attr(doc, el, "use-attribute-sets");
                Instr::Copy {
                    attribute_sets: self.qname_list(d, el, sets)?,
                    body: self.body(d, el)?,
                }
            }
            "copy-of" => {
                let select = self.required(d, el, "select")?;
                Instr::CopyOf {
                    select: self.xpath(d, el, select)?,
                    location,
                }
            }
            "element" | "attribute" => {
                let raw = self.required(d, el, "name")?;
                let name = self.avt(d, el, raw)?;
                let namespace = self.opt_avt(d, el, "namespace")?;
                let scope = self.in_scope(doc, el);
                // libxslt checks a literal name at compile time.
                if let Avt::Literal(literal) = &name {
                    let literal = literal.trim();
                    let (prefix, local_part) = match literal.split_once(':') {
                        Some((p, l)) => (Some(p), l),
                        None => (None, literal),
                    };
                    if !is_ncname(local_part) || prefix.is_some_and(|p| !is_ncname(p)) {
                        return Err(self.error(d, el, format!(
                            "xsl:{local}: The value '{literal}' of the attribute 'name' is not a valid QName."
                        )));
                    }
                    if let Some(p) = prefix
                        && namespace.is_none()
                        && p != "xml"
                        && self.resolve_prefix(&scope, p).is_none()
                    {
                        return Err(self.error(d, el, format!("xsl:{local}: The QName '{literal}' has no namespace binding in scope in the stylesheet")));
                    }
                }
                let body = self.body(d, el)?;
                if local == "element" {
                    let sets = self.attr(doc, el, "use-attribute-sets");
                    let attribute_sets = self.qname_list(d, el, sets)?;
                    Instr::Element {
                        name,
                        namespace,
                        scope,
                        attribute_sets,
                        body,
                        location,
                    }
                } else {
                    Instr::Attribute {
                        name,
                        namespace,
                        scope,
                        body,
                        location,
                    }
                }
            }
            "comment" => Instr::Comment(self.body(d, el)?),
            "processing-instruction" => {
                let name = self.required(d, el, "name")?;
                let name = self.avt(d, el, name)?;
                Instr::ProcessingInstruction {
                    name,
                    body: self.body(d, el)?,
                    location,
                }
            }
            "message" => {
                let terminate = self
                    .attr(doc, el, "terminate")
                    .is_some_and(|t| t.trim() == "yes");
                Instr::Message {
                    terminate,
                    body: self.body(d, el)?,
                }
            }
            "apply-imports" => Instr::ApplyImports { location },
            "number" => self.number(d, el, location)?,
            _ if d.forwards_compatible => {
                let mut body = Vec::new();
                for child in doc.children(el).collect::<Vec<_>>() {
                    if self.is_xslt_named(doc, child, "fallback") {
                        body.extend(self.body(d, child)?);
                    }
                }
                return Ok(Instr::Choose {
                    whens: vec![(Expr::Literal("x".into()), body, location)],
                    otherwise: None,
                });
            }
            _ => return Err(self.error(d, el, format!("Unknown XSLT instruction xsl:{local}"))),
        })
    }

    // ---------- import precedence ----------

    /// Numbers the modules by a post-order walk of the import tree (the
    /// main module highest) and resolves what precedence decides: named
    /// templates and global variables keep only their highest-precedence
    /// definition (named templates the later one on a tie, globals the
    /// first).
    fn assign_precedence(&mut self) {
        fn walk(modules: &[Vec<usize>], m: usize, rank: &mut [usize], next: &mut usize) {
            for &i in &modules[m] {
                walk(modules, i, rank, next);
            }
            rank[m] = *next;
            *next += 1;
        }
        let mut rank = vec![0; self.modules.len()];
        walk(&self.modules, 0, &mut rank, &mut 0);
        for t in &mut self.ss.templates {
            t.precedence = rank[t.precedence];
        }
        for rules in self.ss.rules.values_mut() {
            for r in rules {
                r.precedence = rank[r.precedence];
            }
        }
        for r in &mut self.ss.space {
            r.precedence = rank[r.precedence];
        }
        let mut key_order = Vec::new();
        for (&name, defs) in &mut self.ss.keys {
            for (i, k) in defs.iter_mut().enumerate() {
                k.precedence = rank[k.precedence];
                key_order.push((std::cmp::Reverse(k.precedence), k.order, name, i));
            }
        }
        key_order.sort_by_key(|&(p, o, _, _)| (p, o));
        self.ss.key_order = key_order.into_iter().map(|(_, _, n, i)| (n, i)).collect();
        for (index, t) in self.ss.templates.iter().enumerate() {
            if let Some(name) = t.name {
                match self.ss.named.get(&name) {
                    Some(&old) if self.ss.templates[old].precedence > t.precedence => {}
                    _ => {
                        self.ss.named.insert(name, index);
                    }
                }
            }
        }
        let mut best: HashMap<Name, usize> = HashMap::new();
        for (g, &m) in self.ss.globals.iter().zip(&self.global_modules) {
            let p = best.entry(g.name).or_insert(rank[m]);
            *p = (*p).max(rank[m]);
        }
        // On a tie (a stylesheet and its includes) the first declaration in
        // document order wins: libxslt appends globals to the stylesheet's
        // list and xsltEvalGlobalVariables skips names already defined.
        let mut keep = self.global_modules.iter().map(|&m| rank[m]);
        let mut kept = HashSet::new();
        self.ss
            .globals
            .retain(|g| keep.next() == Some(best[&g.name]) && kept.insert(g.name));

        // libxslt takes cdata-section-elements from the first module, in
        // precedence order, that has any (XSLT_GET_IMPORT_PTR); they don't
        // merge across imports.
        if let Some(top) = self.cdata_modules.iter().map(|&(m, _)| rank[m]).max() {
            self.ss.output.cdata_section_elements = self
                .cdata_modules
                .iter()
                .filter(|&&(m, _)| rank[m] == top)
                .map(|&(_, n)| n)
                .collect();
        }

        // Attribute sets merge as libxslt's xsltMergeSetsCB does: the
        // highest-precedence definition's attributes, then each lower one's
        // whose names aren't already there.
        for (name, mut defs) in std::mem::take(&mut self.module_sets) {
            defs.sort_by_key(|(m, _)| std::cmp::Reverse(rank[*m]));
            let mut defs = defs.into_iter().map(|(_, set)| set);
            let mut merged = defs.next().expect("a set has a definition");
            for lower in defs {
                merged.uses.extend(lower.uses);
                for attr in lower.attributes {
                    let key = self.attribute_key(&attr);
                    if !merged
                        .attributes
                        .iter()
                        .any(|a| self.attribute_key(a) == key)
                    {
                        merged.attributes.push(attr);
                    }
                }
            }
            self.ss.attribute_sets.insert(name, merged);
        }
    }

    // ---------- xsl:number ----------

    /// As libxslt's xsltNumberComp: `level`, `count`, `from`,
    /// `grouping-separator` and `grouping-size` are read as they are, not
    /// as attribute value templates; only `format` is one.
    fn number(&mut self, d: &Doc, el: NodeId, location: Location) -> R<Instr> {
        // libxslt evaluates "number(" + value + ")".
        let value = match self.attr(d.doc, el, "value") {
            Some(v) => Some(Expr::Call(
                xpath::ast::Function::Core(xpath::ast::CoreFn::Number),
                vec![self.xpath(d, el, v)?],
            )),
            None => None,
        };
        let level = match self.attr(d.doc, el, "level") {
            None | Some("single") => NumberLevel::Single,
            Some("multiple") => NumberLevel::Multiple,
            Some("any") => NumberLevel::Any,
            Some(other) => {
                let e = self.error(
                    d,
                    el,
                    format!("xsl:number : invalid value {other} for level"),
                );
                self.warnings.push(e.to_string());
                NumberLevel::Invalid
            }
        };
        let pattern = |c: &mut Self, attr: &str| -> R<Option<pattern::Pattern>> {
            match c.attr(d.doc, el, attr) {
                Some(p) => {
                    let expr = c.pattern_xpath(d, el, p)?;
                    let pat = pattern::from_expr(expr, true)
                        .map_err(|e| c.error(d, el, format!("{} '{p}'", e.0)))?;
                    Ok(Some(pat))
                }
                None => Ok(None),
            }
        };
        let count = pattern(self, "count")?;
        let from = pattern(self, "from")?;
        let format = match self.attr(d.doc, el, "format") {
            Some(f) => self.avt(d, el, f)?,
            None => Avt::Literal(String::new()),
        };
        let separator = self
            .attr(d.doc, el, "grouping-separator")
            .and_then(|s| s.chars().next());
        let grouping = match self.attr(d.doc, el, "grouping-size") {
            Some(size) => {
                let n = xpath::number::from_string(size);
                super::number::Grouping {
                    separator,
                    // C's (int) of a double, as x86 does it for NaN.
                    size: if n.is_nan() {
                        i64::from(i32::MIN)
                    } else {
                        n as i64
                    },
                }
            }
            None => super::number::Grouping::default(),
        };
        Ok(Instr::Number {
            value,
            level,
            count,
            from,
            format,
            grouping,
            location,
        })
    }

    // ---------- attribute sets ----------

    /// libxslt's xsltResolveStylesheetAttributeSet: each set's list becomes
    /// its own attributes (from every definition with its name, in order),
    /// followed by those of the sets it uses, in turn, minus any whose
    /// compile-time name is already in the list. The set's own attributes
    /// therefore win and keep their place. A cycle is an error.
    fn resolve_attribute_sets(&mut self) {
        let names: Vec<Name> = self.ss.attribute_sets.keys().copied().collect();
        let mut state: HashMap<Name, u8> = HashMap::new(); // 1 = resolving, 2 = done
        for n in names {
            self.resolve_set(n, &mut state, 0);
        }
    }

    fn resolve_set(&mut self, name: Name, state: &mut HashMap<Name, u8>, depth: usize) {
        match state.get(&name) {
            Some(2) => return,
            Some(_) => {
                self.errors.push(CompileError {
                    message: format!(
                        "xsl:attribute-set : use-attribute-sets recursion detected on {}",
                        self.names.str(name.1)
                    ),
                    location: Location::default(),
                });
                state.insert(name, 2);
                return;
            }
            None => {}
        }
        if depth > 100 {
            return;
        }
        state.insert(name, 1);
        let uses = self
            .ss
            .attribute_sets
            .get(&name)
            .map(|s| s.uses.clone())
            .unwrap_or_default();
        for used in uses {
            // An unknown set in use-attribute-sets is skipped, as in libxslt.
            if !self.ss.attribute_sets.contains_key(&used) {
                continue;
            }
            self.resolve_set(used, state, depth + 1);
            let other = self.ss.attribute_sets[&used].attributes.clone();
            for attr in other {
                let key = self.attribute_key(&attr);
                let present = self.ss.attribute_sets[&name]
                    .attributes
                    .iter()
                    .any(|a| self.attribute_key(a) == key);
                if !present {
                    self.ss
                        .attribute_sets
                        .get_mut(&name)
                        .expect("set exists")
                        .attributes
                        .push(attr);
                }
            }
        }
        if let Some(set) = self.ss.attribute_sets.get_mut(&name) {
            set.uses.clear();
        }
        state.insert(name, 2);
    }

    /// The compile-time name of an `xsl:attribute`, as libxslt's merge
    /// compares it: local name and namespace when the name is literal, and
    /// (None, None) when it's a template.
    fn attribute_key(&self, i: &Instr) -> (Option<String>, Option<String>) {
        let Instr::Attribute {
            name,
            namespace,
            scope,
            ..
        } = i
        else {
            return (None, None);
        };
        let Avt::Literal(n) = name else {
            return (None, None);
        };
        let (prefix, local) = match n.trim().split_once(':') {
            Some((p, l)) => (Some(p), l.to_string()),
            None => (None, n.trim().to_string()),
        };
        let ns = match namespace {
            Some(Avt::Literal(u)) => (!u.is_empty()).then(|| u.clone()),
            Some(Avt::Parts(_)) => None,
            None => prefix
                .and_then(|p| self.resolve_prefix(scope, p))
                .map(|u| self.names.str(u).to_string()),
        };
        (Some(local), ns)
    }

    // ---------- globals ----------

    /// Orders global variables so each comes after the globals it uses.
    /// A circular reference is an error. Later declarations of the same name win.
    fn order_globals(&mut self) {
        let globals = std::mem::take(&mut self.ss.globals);
        let mut by_name: HashMap<Name, Variable> = HashMap::new();
        let mut order: Vec<Name> = Vec::new();
        for g in globals {
            if !by_name.contains_key(&g.name) {
                order.push(g.name);
            }
            by_name.insert(g.name, g);
        }
        let mut state: HashMap<Name, u8> = HashMap::new(); // 1 = visiting, 2 = done
        let mut sorted = Vec::new();
        for name in order {
            self.visit_global(name, &by_name, &mut state, &mut sorted);
        }
        self.ss.globals = sorted;
    }

    fn visit_global(
        &mut self,
        name: Name,
        by_name: &HashMap<Name, Variable>,
        state: &mut HashMap<Name, u8>,
        out: &mut Vec<Variable>,
    ) {
        match state.get(&name) {
            Some(2) => return,
            Some(1) => {
                let v = &by_name[&name];
                self.errors.push(CompileError {
                    message: format!(
                        "Circular reference to global variable {}",
                        self.names.str(name.1)
                    ),
                    location: v.location.clone(),
                });
                return;
            }
            _ => {}
        }
        let Some(var) = by_name.get(&name) else {
            return;
        };
        state.insert(name, 1);
        let mut refs = Vec::new();
        refs_in_value(&var.value, &mut refs);
        for r in refs {
            if by_name.contains_key(&r) {
                self.visit_global(r, by_name, state, out);
            }
        }
        state.insert(name, 2);
        out.push(var.clone());
    }
}

/// Calls `f` on every node of an expression, parents first.
fn rewrite_calls(e: &mut Expr, f: &mut dyn FnMut(&mut Expr)) {
    f(e);
    match e {
        Expr::Or(a, b)
        | Expr::And(a, b)
        | Expr::Compare(_, a, b)
        | Expr::Arith(_, a, b)
        | Expr::Union(a, b) => {
            rewrite_calls(a, f);
            rewrite_calls(b, f);
        }
        Expr::Negate(a) => rewrite_calls(a, f),
        Expr::Call(_, args) => args.iter_mut().for_each(|a| rewrite_calls(a, f)),
        Expr::Filter(p, preds) => {
            rewrite_calls(p, f);
            preds.iter_mut().for_each(|a| rewrite_calls(a, f));
        }
        Expr::Path(p) => {
            if let xpath::ast::PathStart::Expr(s) = &mut p.start {
                rewrite_calls(s, f);
            }
            for st in &mut p.steps {
                st.predicates.iter_mut().for_each(|a| rewrite_calls(a, f));
            }
        }
        Expr::Literal(_) | Expr::Number(_) | Expr::Variable(_) => {}
    }
}

/// An XML NCName, approximately: a name without a colon.
pub fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_')
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '\u{B7}'))
}

/// Variable names referenced by a value, including inside bodies.
fn refs_in_value(v: &VarValue, out: &mut Vec<Name>) {
    match v {
        VarValue::Select(e) => refs_in_expr(e, out),
        VarValue::Body(b) => refs_in_body(b, out),
        VarValue::Empty => {}
    }
}

fn refs_in_avt(a: &Avt, out: &mut Vec<Name>) {
    if let Avt::Parts(parts) = a {
        for p in parts {
            if let AvtPart::Expr(e) = p {
                refs_in_expr(e, out);
            }
        }
    }
}

fn refs_in_body(body: &Body, out: &mut Vec<Name>) {
    for i in body {
        match i {
            Instr::Text(_)
            | Instr::UnescapedText(_)
            | Instr::ApplyImports { .. }
            | Instr::Unsupported { .. } => {}
            Instr::Document {
                href,
                settings,
                body,
                ..
            } => {
                href.iter().for_each(|a| refs_in_avt(a, out));
                settings.iter().for_each(|(_, a)| refs_in_avt(a, out));
                refs_in_body(body, out);
            }
            Instr::LiteralElement {
                attributes, body, ..
            } => {
                attributes.iter().for_each(|(_, a)| refs_in_avt(a, out));
                refs_in_body(body, out);
            }
            Instr::ValueOf { select, .. } | Instr::CopyOf { select, .. } => {
                refs_in_expr(select, out)
            }
            Instr::Number { value, format, .. } => {
                if let Some(v) = value {
                    refs_in_expr(v, out);
                }
                refs_in_avt(format, out);
            }
            Instr::ApplyTemplates {
                select,
                sorts,
                params,
                ..
            } => {
                if let Some(s) = select {
                    refs_in_expr(s, out);
                }
                sorts.iter().for_each(|s| refs_in_expr(&s.select, out));
                params.iter().for_each(|p| refs_in_value(&p.value, out));
            }
            Instr::CallTemplate { params, .. } => {
                params.iter().for_each(|p| refs_in_value(&p.value, out))
            }
            Instr::ForEach {
                select,
                sorts,
                body,
                ..
            } => {
                refs_in_expr(select, out);
                sorts.iter().for_each(|s| refs_in_expr(&s.select, out));
                refs_in_body(body, out);
            }
            Instr::If { test, body, .. } => {
                refs_in_expr(test, out);
                refs_in_body(body, out);
            }
            Instr::Choose { whens, otherwise } => {
                for (t, b, _) in whens {
                    refs_in_expr(t, out);
                    refs_in_body(b, out);
                }
                if let Some(b) = otherwise {
                    refs_in_body(b, out);
                }
            }
            Instr::Variable(v) => refs_in_value(&v.value, out),
            Instr::Copy { body, .. } | Instr::Comment(body) | Instr::Message { body, .. } => {
                refs_in_body(body, out)
            }
            Instr::Element {
                name,
                namespace,
                body,
                ..
            }
            | Instr::Attribute {
                name,
                namespace,
                body,
                ..
            } => {
                refs_in_avt(name, out);
                if let Some(n) = namespace {
                    refs_in_avt(n, out);
                }
                refs_in_body(body, out);
            }
            Instr::ProcessingInstruction { name, body, .. } => {
                refs_in_avt(name, out);
                refs_in_body(body, out);
            }
        }
    }
}

fn refs_in_expr(e: &Expr, out: &mut Vec<Name>) {
    use crate::xpath::ast::PathStart;
    match e {
        Expr::Variable(n) => out.push(*n),
        Expr::Or(a, b)
        | Expr::And(a, b)
        | Expr::Compare(_, a, b)
        | Expr::Arith(_, a, b)
        | Expr::Union(a, b) => {
            refs_in_expr(a, out);
            refs_in_expr(b, out);
        }
        Expr::Negate(a) => refs_in_expr(a, out),
        Expr::Call(_, args) => args.iter().for_each(|a| refs_in_expr(a, out)),
        Expr::Filter(p, preds) => {
            refs_in_expr(p, out);
            preds.iter().for_each(|a| refs_in_expr(a, out));
        }
        Expr::Path(p) => {
            if let PathStart::Expr(s) = &p.start {
                refs_in_expr(s, out);
            }
            for s in &p.steps {
                s.predicates.iter().for_each(|a| refs_in_expr(a, out));
            }
        }
        Expr::Literal(_) | Expr::Number(_) => {}
    }
}

/// The in-scope namespaces of `el`, nearest declaration first, appended to
/// `out`. An undeclared default namespace (`xmlns=""`) has an empty URI.
fn in_scope_into(doc: &Document, el: NodeId, out: &mut Vec<Binding>) {
    let start = out.len();
    let mut cur = Some(el);
    while let Some(e) = cur {
        for d in doc.ns_decls(e) {
            if !out[start..].iter().any(|(p, _)| *p == d.prefix) {
                out.push((d.prefix, d.uri));
            }
        }
        cur = doc.parent(e);
    }
}
