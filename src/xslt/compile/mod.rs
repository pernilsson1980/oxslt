//! Compiles a parsed stylesheet document into the IR.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::ir::*;
use super::pattern;
use crate::catalog::ResourceLoader;
use crate::parse::{ParseOptions, parse_with};
use crate::tree::{Document, NamePool, NodeId, NodeKind, QName, Sym};
use crate::xpath::{self, ast::Expr, ast::Name};
mod body;
mod declarations;
mod modules;
pub use body::DOCUMENT_ELEMENT_NAMESPACES;

pub const XSLT_NS: &str = "http://www.w3.org/1999/XSL/Transform";
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

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

    // ---------- top level ----------

    // ---------- bodies ----------

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
    /// Later declarations of the same name win.
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
            // A circular reference isn't a compile error in libxslt: computing
            // the globals stops the transform (exit 10), which
            // `exec::compute_global` does.
            Some(1) => return,
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
