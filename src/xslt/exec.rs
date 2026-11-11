//! Runs a compiled stylesheet against a source document.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::compile::{XML_NS, XSLT_NS};
use super::exslt;
use super::ir::*;
use super::number;
use super::output::{OutputError, ResultTree};
use super::pattern::{Alternative, Anchor, Pattern, PatternStep};
use crate::catalog::ResourceLoader;
use crate::parse::{ParseOptions, parse_with};
use crate::tree::{Builder, Document, NamePool, NodeId, NodeKind, QName, SymMap};
use crate::xpath::ast::{Axis, Expr, Name, NodeTest};
use crate::xpath::{self, Env, Focus, Functions, NodeRef, Value, Variables, XPathError};

/// libxslt's default `--maxdepth`: nested template calls.
pub const MAX_DEPTH: usize = 3_000;

/// Limits on a transform's work and on what it may write. The default is
/// no work limit, as in xsltproc, and no file writes, so a library caller
/// that doesn't ask for writes never gets any.
#[derive(Debug, Clone, Default)]
pub struct Limits {
    /// Units of work: one per instruction, per node a template is applied
    /// to, per XPath evaluation, per node an XPath step visits, per node
    /// copied or indexed for a key, and per 64 bytes `concat()` builds.
    pub max_work: Option<u64>,
    /// Wall-clock time for the whole transform.
    pub timeout: Option<Duration>,
    /// Which files `exsl:document` may write.
    pub writes: Writes,
}

/// The extension elements xsltproc registers, by (namespace, name).
const REGISTERED_EXTENSION_ELEMENTS: [(&str, &str); 6] = [
    ("http://exslt.org/common", "document"),
    ("http://www.jclark.com/xt", "document"),
    ("http://icl.com/saxon", "output"),
    ("org.apache.xalan.xslt.extensions.Redirect", "write"),
    ("http://xmlsoft.org/XSLT/namespace", "debug"),
    ("http://exslt.org/functions", "result"),
];

/// What `exsl:document` may write, as xsltproc's security preferences
/// decide (libxslt's xsltCheckWrite).
#[derive(Debug, Clone, Default)]
pub struct Writes {
    pub files: WriteCheck,
    /// Missing directories may be created.
    pub mkdir: bool,
    /// The `-o` file: an `exsl:document`'s `href` is relative to it, or to
    /// the current directory without one.
    pub output_file: Option<String>,
}

/// Which files may be written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum WriteCheck {
    /// Nothing (xsltproc's `--nowrite`); the default for library callers.
    #[default]
    Refuse,
    /// Anything the user may write: xsltproc's default.
    Allow,
    /// Paths starting with this prefix (`--writesubtree`).
    Subtree(String),
}

impl Writes {
    /// xsltproc's defaults: any file, creating directories.
    pub fn allowed() -> Writes {
        Writes {
            files: WriteCheck::Allow,
            mkdir: true,
            output_file: None,
        }
    }

    fn file_allowed(&self, path: &str) -> bool {
        match &self.files {
            WriteCheck::Refuse => false,
            WriteCheck::Allow => true,
            WriteCheck::Subtree(prefix) => path.starts_with(prefix.as_str()),
        }
    }
}

/// A transform's remaining work and its deadline, shared with XPath
/// evaluation through `Functions::tick`.
struct Budget {
    /// Whether any limit is set; without one, a tick does nothing.
    active: bool,
    left: Cell<u64>,
    limits: Limits,
    deadline: Option<Instant>,
    /// Ticks until the clock is read again.
    until_clock: Cell<u32>,
    /// Set once a limit is exceeded: the message.
    exceeded: RefCell<Option<String>>,
}

impl Budget {
    fn new(limits: &Limits) -> Self {
        let limits = Limits {
            writes: Writes::default(),
            ..limits.clone()
        };
        Budget {
            active: limits.max_work.is_some() || limits.timeout.is_some(),
            left: Cell::new(limits.max_work.unwrap_or(u64::MAX)),
            deadline: limits.timeout.map(|t| Instant::now() + t),
            limits,
            until_clock: Cell::new(4096),
            exceeded: RefCell::new(None),
        }
    }

    /// Counts `cost` work units. Called for every instruction and XPath
    /// step, so the common case, no limits, is a single test.
    #[inline]
    fn tick(&self, cost: u64) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        self.tick_limited(cost)
    }

    #[cold]
    #[inline(never)]
    fn tick_limited(&self, cost: u64) -> Result<(), String> {
        if let Some(e) = &*self.exceeded.borrow() {
            return Err(e.clone());
        }
        if let Some(max) = self.limits.max_work {
            let left = self.left.get().saturating_sub(cost);
            self.left.set(left);
            if left == 0 {
                return Err(self.exceed(format!(
                    "Transform stopped: it exceeded the work limit of {max} units"
                )));
            }
        }
        if let Some(deadline) = self.deadline {
            let n = self.until_clock.get().saturating_sub(1);
            self.until_clock.set(if n == 0 { 4096 } else { n });
            if n == 0 && Instant::now() > deadline {
                let t = self.limits.timeout.unwrap_or_default();
                return Err(self.exceed(format!(
                    "Transform stopped: it exceeded the time limit of {t:?}"
                )));
            }
        }
        Ok(())
    }

    fn exceed(&self, message: String) -> String {
        *self.exceeded.borrow_mut() = Some(message.clone());
        message
    }
}

#[derive(Debug)]
pub enum ExecError {
    /// An error libxslt reports with xsltTransformError, leaving the
    /// transform in its error state (xsltproc exit 9). libxslt goes on after
    /// some of these; oxslt stops unless it reports them (`report`).
    Runtime(String),
    /// What sets libxslt's XSLT_STATE_STOPPED (exit 10): a failed XPath
    /// evaluation in an instruction, a global variable that fails,
    /// `xsl:message terminate="yes"`, recursion past the limit.
    Terminated(String),
}

/// A global parameter given on the command line.
pub enum Param {
    /// `--stringparam`: a string.
    String(String),
    /// `--param`: an XPath expression, evaluated with the source root as context.
    Expr(Expr),
}

pub struct Transform<'a> {
    ss: &'a Stylesheet,
    names: &'a mut NamePool,
    docs: Vec<Document>,
    globals: SymMap<Name, Value>,
    /// Each global's index in `ss.globals`, for computing it on demand.
    global_defs: SymMap<Name, usize>,
    /// Globals an evaluation asked for before they were computed.
    wanted_globals: RefCell<Vec<Name>>,
    /// dyn:evaluate's expressions, compiled between evaluations.
    dyn_exprs: RefCell<DynExprs>,
    /// Globals being computed, to catch a circular definition.
    computing: Vec<Name>,
    /// The error of a global computed on demand inside an evaluation.
    global_error: RefCell<Option<ExecError>>,
    /// Command-line parameters not used yet.
    params: SymMap<Name, Param>,
    locals: Vec<(Name, Value)>,
    out: Vec<ResultTree>,
    depth: usize,
    ids: RefCell<SymMap<NodeRef, usize>>,
    keys: RefCell<Keys>,
    /// Documents `document()` loaded or asked for.
    loads: RefCell<DocLoads>,
    /// Result tree fragments extension functions made.
    made: RefCell<Made>,
    /// Numbers evaluations, so a rerun finds the fragments it made.
    serial: u64,
    /// The name `token`, for str:tokenize and str:split.
    token: crate::tree::Sym,
    /// No namespaces, for evaluations outside an instruction.
    no_scope: Rc<[Binding]>,
    loader: &'a ResourceLoader,
    options: &'a ParseOptions,
    /// Warnings raised inside XPath evaluation (format-number), written out
    /// after each instruction.
    warnings: RefCell<Vec<String>>,
    /// `cdata-section-elements` in effect, shared by every result tree.
    cdata: std::rc::Rc<Vec<Name>>,
    /// Match rules per mode, best first.
    rules: Rc<SymMap<Option<Name>, Vec<Rule>>>,
    /// For each mode and kind of node, the positions in `rules` of the rules
    /// that could match it, in the same order. Built on first use.
    rule_index: SymMap<(Option<Name>, NodeKey), Rc<[u32]>>,
    budget: Budget,
    /// What exsl:document may write; `writes.output_file` is the file being
    /// written, which a nested exsl:document's href is relative to.
    writes: Writes,
    /// libxslt's XSLT_STATE_ERROR: an error was reported that doesn't stop
    /// the transform (xsltTransformError), but its result is discarded.
    failed: bool,
    /// libxslt's currentTemplateRule: the template rule applied last, for
    /// xsl:apply-imports. None inside xsl:for-each.
    current_rule: Option<usize>,
    /// The mode of the innermost xsl:apply-templates.
    mode: Option<Name>,
    messages: &'a mut dyn Write,
}

type R<T> = Result<T, ExecError>;

fn where_(loc: &Location) -> String {
    format!(
        "runtime error: file {} line {} element {}",
        loc.file, loc.line, loc.element
    )
}

impl<'a> Transform<'a> {
    /// Transforms `source`. Returns the result tree, and the documents the
    /// transform created, which the result can refer to.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        ss: &'a Stylesheet,
        names: &'a mut NamePool,
        loader: &'a ResourceLoader,
        options: &'a ParseOptions,
        source: Document,
        params: Vec<(Name, Param)>,
        limits: Limits,
        messages: &'a mut dyn Write,
    ) -> R<Document> {
        let source = strip_space(source, ss, names);
        // libxslt makes CDATA sections only while the output type is xml; an
        // explicit html or text method turns cdata-section-elements off.
        let cdata = std::rc::Rc::new(match ss.output.method {
            Method::Xml | Method::Unspecified => ss.output.cdata_section_elements.clone(),
            Method::Html | Method::Text => Vec::new(),
        });
        let mut rules: SymMap<_, _> = ss.rules.clone().into_iter().collect();
        for list in rules.values_mut() {
            list.sort_by(|a, b| {
                b.precedence
                    .cmp(&a.precedence)
                    .then(b.priority.total_cmp(&a.priority))
                    .then(b.order.cmp(&a.order))
            });
        }
        let token = names.intern("token");
        let mut t = Transform {
            ss,
            names,
            docs: vec![source],
            globals: SymMap::default(),
            global_defs: SymMap::default(),
            wanted_globals: RefCell::new(Vec::new()),
            dyn_exprs: RefCell::new(DynExprs::default()),
            computing: Vec::new(),
            global_error: RefCell::new(None),
            params: SymMap::default(),
            locals: Vec::new(),
            out: vec![ResultTree::new(cdata.clone())],
            cdata,
            depth: 0,
            ids: RefCell::new(SymMap::default()),
            keys: RefCell::new(Keys::default()),
            loads: RefCell::new(DocLoads::default()),
            made: RefCell::new(Made::default()),
            serial: 0,
            token,
            no_scope: Rc::new([]),
            loader,
            options,
            warnings: RefCell::new(Vec::new()),
            rules: Rc::new(rules),
            rule_index: SymMap::default(),
            current_rule: None,
            mode: None,
            budget: Budget::new(&limits),
            writes: limits.writes.clone(),
            failed: false,
            messages,
        };
        // document() with the input's URL gives the input itself.
        if let Some(url) = t.docs[0].url.clone() {
            t.loads.borrow_mut().by_url.insert(url, Some(0));
        }
        let root = NodeRef::new(0, t.docs[0].root());
        let focus = Focus::single(root);
        t.global_defs = ss
            .globals
            .iter()
            .enumerate()
            .map(|(i, g)| (g.name, i))
            .collect();
        // Command-line parameters, as libxslt's xsltProcessUserParamInternal
        // treats them: ignored when a module declares an xsl:variable of
        // that name; a declared xsl:param takes the value when computed;
        // otherwise the parameter is a global of its own, evaluated now.
        for (name, param) in params {
            if ss.global_variable_names.contains(&name) {
                continue;
            }
            if t.global_defs.contains_key(&name) {
                t.params.insert(name, param);
                continue;
            }
            let value = match param {
                Param::String(s) => Value::String(s),
                Param::Expr(e) => t.eval(&e, &focus, &Location::default()).map_err(|_| {
                    ExecError::Runtime(format!(
                        "Evaluating user parameter {} failed",
                        t.names.str(name.1)
                    ))
                })?,
            };
            t.globals.insert(name, value);
        }
        for g in &ss.globals {
            t.compute_global(g.name)?;
        }
        t.apply(&[root], None, &[], &focus)?;
        if t.failed {
            // Reported already; libxslt discards the result.
            return Err(ExecError::Runtime(String::new()));
        }
        let tree = t.out.pop().expect("main output");
        tree.finish(t.names)
            .map_err(|e| ExecError::Runtime(format!("{e:?}")))
    }

    // ---------- evaluation ----------

    /// Runs an XPath evaluation. When `document()` asks for documents that
    /// aren't loaded yet, it fails; they're loaded here and the evaluation
    /// runs again. XPath has no side effects that a rerun would repeat:
    /// generate-id() gives the same ids, and key indexes are kept only once
    /// complete.
    fn env_eval<T>(
        &mut self,
        focus: &Focus,
        base: &str,
        namespaces: &[Binding],
        f: impl Fn(&Env) -> Result<T, XPathError>,
    ) -> Result<T, XPathError> {
        self.serial += 1;
        let serial = self.serial;
        loop {
            let warnings = self.warnings.borrow().len();
            self.made.borrow_mut().calls = 0;
            let result = {
                let vars = Vars {
                    locals: &self.locals,
                    globals: &self.globals,
                    lazy: Some((&self.global_defs, &self.wanted_globals)),
                };
                let funcs = XsltFunctions {
                    current: focus.node,
                    budget: &self.budget,
                    ids: &self.ids,
                    keys: &self.keys,
                    loads: &self.loads,
                    made: &self.made,
                    serial,
                    token: self.token,
                    base,
                    namespaces,
                    ss: self.ss,
                    warnings: &self.warnings,
                    wanted_globals: &self.wanted_globals,
                    dyn_exprs: &self.dyn_exprs,
                };
                let env = Env {
                    docs: &self.docs,
                    names: self.names,
                    vars: &vars,
                    funcs: &funcs,
                };
                f(&env)
            };
            let wanted = std::mem::take(&mut *self.wanted_globals.borrow_mut());
            let made = std::mem::take(&mut self.made.borrow_mut().pending);
            let to_compile = std::mem::take(&mut self.dyn_exprs.borrow_mut().pending);
            if self.loads.borrow().pending.is_empty()
                && wanted.is_empty()
                && made.is_empty()
                && to_compile.is_empty()
            {
                return result;
            }
            self.warnings.borrow_mut().truncate(warnings);
            for key in to_compile {
                self.compile_dynamic(key);
            }
            self.load_pending();
            for (key, doc) in made {
                self.docs.push(doc);
                let index = u32::try_from(self.docs.len() - 1)
                    .map_err(|_| XPathError::runtime("Too many documents"))?;
                self.made.borrow_mut().by_key.insert(key, index);
            }
            for name in wanted {
                if let Err(e) = self.compute_global(name) {
                    // Passed on as it is by the caller's error mapping.
                    *self.global_error.borrow_mut() = Some(e);
                    return Err(XPathError::runtime("global variable failed"));
                }
            }
        }
    }

    /// Compiles a dyn:evaluate expression in its namespace context, which
    /// may intern names; an evaluation can only read them.
    fn compile_dynamic(&mut self, key: (String, Vec<Binding>)) {
        let prefixes: Vec<(String, crate::tree::Sym)> = key
            .1
            .iter()
            .filter_map(|&(p, u)| p.map(|p| (self.names.str(p).to_string(), u)))
            .collect();
        let resolve = |p: &str| {
            if p == "xml" {
                return Some(NamePool::XML_NS);
            }
            prefixes.iter().find(|(q, _)| q == p).map(|&(_, u)| u)
        };
        let compiled = xpath::compile(&key.0, self.names, &resolve)
            .map(Rc::new)
            .map_err(|e| format!("XPath error : {e}"));
        self.dyn_exprs.borrow_mut().compiled.insert(key, compiled);
    }

    /// Computes a global variable or parameter, as libxslt does when it's
    /// first referenced: with the root as context and no local variables.
    fn compute_global(&mut self, name: Name) -> R<()> {
        if self.globals.contains_key(&name) {
            return Ok(());
        }
        let Some(&index) = self.global_defs.get(&name) else {
            return Ok(());
        };
        let g = &self.ss.globals[index];
        if self.computing.contains(&name) {
            return Err(ExecError::Terminated(format!(
                "{}\nRecursive definition of {}",
                where_(&g.location),
                self.names.str(name.1)
            )));
        }
        self.computing.push(name);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_rule = self.current_rule.take();
        let root = NodeRef::new(0, self.docs[0].root());
        let focus = Focus::single(root);
        let value = match self.params.remove(&name) {
            Some(Param::String(s)) => Ok(Value::String(s)),
            Some(Param::Expr(e)) => self.eval(&e, &focus, &g.location),
            None => self.var_value(&g.value, &focus, &g.location),
        };
        self.locals = saved_locals;
        self.current_rule = saved_rule;
        self.computing.pop();
        self.globals.insert(name, value?);
        Ok(())
    }

    /// Loads the documents `document()` asked for. One that can't be read
    /// or parsed is remembered as missing and gives an empty node-set, with
    /// a warning, as in libxslt.
    fn load_pending(&mut self) {
        let pending = std::mem::take(&mut self.loads.borrow_mut().pending);
        for url in pending {
            if self.loads.borrow().by_url.contains_key(&url) {
                continue;
            }
            let loaded = self.load_document(&url);
            let index = match loaded {
                Ok(mut doc) => {
                    doc.url = Some(url.clone());
                    let doc = strip_space(doc, self.ss, self.names);
                    self.docs.push(doc);
                    u32::try_from(self.docs.len() - 1).ok()
                }
                Err(e) => {
                    self.warnings.borrow_mut().push(e);
                    None
                }
            };
            self.loads.borrow_mut().by_url.insert(url, index);
        }
    }

    fn load_document(&mut self, url: &str) -> Result<Document, String> {
        // Through the catalogs, as xsltproc's loader does; no network.
        let (bytes, path) = self
            .loader
            .read_document(url, &None)
            .map_err(|e| format!("I/O warning : {e}"))?;
        let parsed = parse_with(
            &bytes,
            self.names,
            self.options,
            self.loader,
            &Some(path.clone()),
        )
        .map_err(|e| format!("{url}:{}: parser error : {}", e.line, e.message))?;
        for w in parsed.warnings {
            self.warnings.borrow_mut().push(format!("warning: {w}"));
        }
        let mut document = parsed.document;
        // With --xinclude, libxslt processes XIncludes in loaded documents
        // too, and ignores whether that failed.
        if self.options.xinclude {
            document.url = Some(path.to_string_lossy().into_owned());
            let processed =
                crate::xinclude::process(document, self.names, self.loader, self.options);
            for m in processed.warnings.iter().chain(&processed.errors) {
                self.warnings.borrow_mut().push(m.clone());
            }
            document = processed.document;
        }
        Ok(document)
    }

    /// An evaluation whose XPath error the caller handles. Running out of
    /// budget, or a global variable failing, still stops the transform.
    fn eval_unmapped(
        &mut self,
        e: &Expr,
        focus: &Focus,
        loc: &Location,
    ) -> R<Result<Value, XPathError>> {
        match self.env_eval(focus, &loc.base, &loc.namespaces, |env| {
            xpath::evaluate(e, env, focus)
        }) {
            Ok(v) => Ok(Ok(v)),
            Err(err) => {
                if self.budget.exceeded.borrow().is_some() || self.global_error.borrow().is_some() {
                    return Err(self.exec_error(err, || where_(loc)));
                }
                Ok(Err(err))
            }
        }
    }

    fn eval(&mut self, e: &Expr, focus: &Focus, loc: &Location) -> R<Value> {
        self.env_eval(focus, &loc.base, &loc.namespaces, |env| {
            xpath::evaluate(e, env, focus)
        })
        .map_err(|err| self.exec_error(err, || where_(loc)))
    }

    /// An XPath error as a runtime error, unless a global variable computed
    /// on demand failed: then that error, as it was.
    fn exec_error(&self, err: XPathError, place: impl FnOnce() -> String) -> ExecError {
        if let Some(m) = &*self.budget.exceeded.borrow() {
            return ExecError::Terminated(m.clone());
        }
        match self.global_error.borrow_mut().take() {
            Some(e) => e,
            // libxslt's instructions stop the transform when an evaluation
            // fails (an undefined variable also reports an error first, but
            // stopping decides the exit code).
            None => ExecError::Terminated(format!("{}\n{err}", place())),
        }
    }

    /// Charges work to the budget.
    fn tick(&self, cost: u64) -> R<()> {
        self.budget.tick(cost).map_err(ExecError::Terminated)
    }

    fn eval_string(&mut self, e: &Expr, focus: &Focus, loc: &Location) -> R<String> {
        let v = self.eval(e, focus, loc)?;
        Ok(self
            .env_eval(focus, &loc.base, &loc.namespaces, |env| {
                Ok(xpath::to_string(&v, env))
            })
            .unwrap_or_default())
    }

    /// A node-set, or None (and the evaluation's value dropped) if the
    /// expression gives something else.
    fn eval_nodes(&mut self, e: &Expr, focus: &Focus, loc: &Location) -> R<Option<Vec<NodeRef>>> {
        match self.eval(e, focus, loc)? {
            Value::NodeSet(n) => Ok(Some(n)),
            _ => Ok(None),
        }
    }

    fn avt(&mut self, a: &Avt, focus: &Focus, loc: &Location) -> R<String> {
        match a {
            Avt::Literal(s) => Ok(s.clone()),
            Avt::Parts(parts) => {
                let mut s = String::new();
                for p in parts {
                    match p {
                        AvtPart::Literal(l) => s.push_str(l),
                        AvtPart::Expr(e) => s.push_str(&self.eval_string(e, focus, loc)?),
                    }
                }
                Ok(s)
            }
        }
    }

    fn var_value(&mut self, v: &VarValue, focus: &Focus, loc: &Location) -> R<Value> {
        match v {
            VarValue::Select(e) => self.eval(e, focus, loc),
            VarValue::Empty => Ok(Value::String(String::new())),
            VarValue::Body(body) => {
                let mut doc = self.fragment(body, focus)?;
                doc.fragment = true;
                self.docs.push(doc);
                let id = u32::try_from(self.docs.len() - 1)
                    .map_err(|_| ExecError::Runtime("Too many documents".into()))?;
                Ok(Value::NodeSet(vec![NodeRef::new(
                    id,
                    self.docs.last().unwrap().root(),
                )]))
            }
        }
    }

    /// exsl:document, as libxslt's xsltDocumentElem: the file name is the
    /// href resolved against the file being written (or the current
    /// directory), checked against the write rules (making missing
    /// directories), and the body written there with the element's own
    /// output attributes; the main xsl:output doesn't apply. Failures are
    /// reported and the transform goes on.
    fn document_element(
        &mut self,
        href: Option<&Avt>,
        settings: &[(&'static str, Avt)],
        body: &Body,
        focus: &Focus,
        loc: &Location,
    ) -> R<()> {
        let Some(href) = href else {
            self.report(loc, "xsltDocumentElem: href/URI-Reference not found");
            return Ok(());
        };
        let url = self.avt(href, focus, loc)?;
        let filename = crate::uri::build(&url, self.writes.output_file.as_deref());
        match self.check_write(&filename) {
            1 => {}
            0 => {
                self.report(
                    loc,
                    &format!("xsltDocumentElem: write rights for {filename} denied"),
                );
                return Ok(());
            }
            _ => return Ok(()),
        }

        let mut output = Output::default();
        let mut append = false;
        let mut unsupported = None;
        for (name, avt) in settings {
            let value = self.avt(avt, focus, loc)?;
            let yes_no = |t: &mut Self, what: &str| match value.as_str() {
                "yes" => Some(true),
                "no" => Some(false),
                other => {
                    t.report(loc, &format!("invalid value for {what}: {other}"));
                    None
                }
            };
            match *name {
                "version" => output.version = Some(value.clone()),
                "encoding" => output.encoding = Some(value.clone()),
                "method" => match value.as_str() {
                    "xml" => output.method = Method::Xml,
                    "html" => output.method = Method::Html,
                    "text" => output.method = Method::Text,
                    "xhtml" => {
                        self.report(loc, "xsltDocumentElem: unsupported method xhtml");
                        output.method = Method::Html;
                    }
                    other if other.contains(':') => unsupported = Some(other.to_string()),
                    other => self.report(loc, &format!("invalid value for method: {other}")),
                },
                "doctype-system" => output.doctype_system = Some(value.clone()),
                "doctype-public" => output.doctype_public = Some(value.clone()),
                "standalone" => {
                    if let Some(b) = yes_no(self, "standalone") {
                        output.standalone = Some(b);
                    }
                }
                "indent" => {
                    if let Some(b) = yes_no(self, "indent") {
                        output.indent = Some(b);
                    }
                }
                "omit-xml-declaration" => {
                    if let Some(b) = yes_no(self, "omit-xml-declaration") {
                        output.omit_xml_declaration = Some(b);
                    }
                }
                // Read into a stylesheet nothing consults: CDATA comes from
                // the main xsl:output, as in libxslt.
                "cdata-section-elements" => {}
                "append" => append = value == "true" || value == "yes",
                _ => {}
            }
        }
        if let Some(m) = unsupported {
            self.report(loc, &format!("xsltDocumentElem: unsupported method ({m})"));
            return Ok(());
        }
        if append {
            output.omit_xml_declaration = Some(true);
        }

        // The body, into a document of its own; nested exsl:documents are
        // relative to this file.
        let cdata = match output.method {
            Method::Unspecified | Method::Xml => self.cdata.clone(),
            Method::Html | Method::Text => Rc::new(Vec::new()),
        };
        let saved_file = self.writes.output_file.replace(filename.clone());
        self.out.push(ResultTree::new(cdata));
        let result = self.body(body, focus);
        let tree = self.out.pop().expect("exsl:document output");
        self.writes.output_file = saved_file;
        result?;
        let doc = tree
            .finish(self.names)
            .map_err(|e| ExecError::Runtime(format!("{e:?}")))?;
        // An empty result writes no file at all (xsltSaveResultToFilename).
        if doc.first_child(doc.root()).is_none() {
            return Ok(());
        }
        let bytes = crate::serialize::serialize(doc, self.names, &output);
        let path = crate::catalog::local_path(&filename);
        let written = path.ok_or(()).and_then(|p| {
            let mut options = std::fs::OpenOptions::new();
            if append {
                options.append(true).create(true);
            } else {
                options.write(true).create(true).truncate(true);
            }
            options
                .open(p)
                .and_then(|mut f| f.write_all(&bytes))
                .map_err(|_| ())
        });
        if written.is_err() {
            self.report(
                loc,
                &format!("xsltDocumentElem: unable to save to {filename}"),
            );
        }
        Ok(())
    }

    /// xsltCheckWrite: 1 if `filename` may be written, 0 if refused (with a
    /// message), negative if a directory couldn't be made. Only local files
    /// are checked; other URLs fail when written.
    fn check_write(&mut self, filename: &str) -> i32 {
        match crate::catalog::local_path(filename) {
            Some(p) => self.check_write_path(&p.to_string_lossy()),
            None if self.writes.files == WriteCheck::Refuse => {
                self.report_plain(&format!("File write for {filename} refused"));
                0
            }
            None => 1,
        }
    }

    /// xsltCheckWritePath: the file check, then the directory, made (after
    /// checking it the same way) if it's missing.
    fn check_write_path(&mut self, path: &str) -> i32 {
        if !self.writes.file_allowed(path) {
            self.report_plain(&format!("File write for {path} refused"));
            return 0;
        }
        // xmlParserGetDirectory: up to the last '/', else the current one.
        let directory = match path.rfind('/') {
            Some(0) => "/".to_string(),
            Some(i) => path[..i].to_string(),
            None => return 1,
        };
        if std::path::Path::new(&directory).exists() {
            return 1;
        }
        if !self.writes.mkdir {
            self.report_plain(&format!("Directory creation for {path} refused"));
            return 0;
        }
        let mut ret = self.check_write_path(&directory);
        if ret == 1 {
            ret = if std::fs::create_dir(&directory).is_ok() {
                0
            } else {
                -1
            };
        }
        if ret < 0 {
            return ret;
        }
        1
    }

    /// A transform error that doesn't stop the transform but fails it, as
    /// xsltTransformError does: the result is discarded at the end.
    fn report(&mut self, loc: &Location, message: &str) {
        let _ = writeln!(self.messages, "{}\n{message}", where_(loc));
        self.failed = true;
    }

    fn report_plain(&mut self, message: &str) {
        let _ = writeln!(self.messages, "{message}");
        self.failed = true;
    }

    /// Runs `body` into a new result tree fragment.
    fn fragment(&mut self, body: &Body, focus: &Focus) -> R<Document> {
        self.out.push(ResultTree::new(self.cdata.clone()));
        let result = self.body(body, focus);
        let tree = self.out.pop().expect("fragment output");
        result?;
        tree.finish(self.names)
            .map_err(|e| ExecError::Runtime(format!("{e:?}")))
    }

    /// The string value of `body`'s output, for attributes, comments and messages.
    fn body_string(&mut self, body: &Body, focus: &Focus) -> R<String> {
        if let [Instr::Text(s)] = body.as_slice() {
            return Ok(s.clone());
        }
        let doc = self.fragment(body, focus)?;
        Ok(doc.string_value(doc.root()))
    }

    fn out(&mut self) -> (&mut ResultTree, &mut NamePool) {
        (self.out.last_mut().expect("an output is open"), self.names)
    }

    fn output_err(e: OutputError) -> ExecError {
        ExecError::Runtime(format!("{e:?}"))
    }

    // ---------- templates ----------

    fn matches(&mut self, alt: &Alternative, node: NodeRef) -> R<bool> {
        if alt.never {
            return Ok(false);
        }
        let no_scope = self.no_scope.clone();
        self.env_eval(&Focus::single(node), "", &no_scope, |env| {
            match_steps(&alt.steps, &alt.anchor, node, env)
        })
        .map_err(|e| self.exec_error(e, || "runtime error: pattern".to_string()))
    }

    /// The best template rule for `node` in `mode`, among those with
    /// precedence below `below` when it's given (xsl:apply-imports).
    fn find_template(
        &mut self,
        node: NodeRef,
        mode: Option<Name>,
        below: Option<usize>,
    ) -> R<Option<usize>> {
        let all = self.rules.clone();
        let Some(rules) = all.get(&mode) else {
            return Ok(None);
        };
        // Only the rules whose last step could match this kind of node and
        // name, in their order: an index instead of trying all of them.
        let key = self.node_key(node);
        let candidates = match self.rule_index.get(&(mode, key)) {
            Some(c) => c.clone(),
            None => {
                let c: Rc<[u32]> = rules
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| {
                        let t = &self.ss.templates[r.template];
                        let alt = &t
                            .matches
                            .as_ref()
                            .expect("rules come from match patterns")
                            .alternatives[r.alternative];
                        could_match(alt, key)
                    })
                    .map(|(i, _)| i as u32)
                    .collect();
                self.rule_index.insert((mode, key), c.clone());
                c
            }
        };
        // Sorted by precedence first, so the lower ones are a suffix. libxslt
        // (xsltNextImport) looks at every module after the current one in
        // precedence order, not only those the current module imports.
        let start = below.map_or(0, |p| {
            candidates.partition_point(|&i| rules[i as usize].precedence >= p)
        });
        let mut found = None;
        for &i in &candidates[start..] {
            let r = &rules[i as usize];
            let t = &self.ss.templates[r.template];
            let alt = &t
                .matches
                .as_ref()
                .expect("rules come from match patterns")
                .alternatives[r.alternative];
            if self.matches(alt, node)? {
                found = Some(r.template);
                break;
            }
        }
        Ok(found)
    }

    /// What template rules are indexed by: the node's kind and name.
    fn node_key(&self, n: NodeRef) -> NodeKey {
        if n.ns != 0 {
            return NodeKey::Other;
        }
        match self.docs[n.doc as usize].kind(n.node) {
            NodeKind::Element { name, .. } => NodeKey::Element(name.expanded()),
            NodeKind::Attribute { name, .. } => NodeKey::Attribute(name.expanded()),
            NodeKind::Text(_) => NodeKey::Text,
            NodeKind::Comment(_) => NodeKey::Comment,
            NodeKind::Root | NodeKind::Pi { .. } => NodeKey::Other,
        }
    }

    fn apply(
        &mut self,
        nodes: &[NodeRef],
        mode: Option<Name>,
        params: &[(Name, Value)],
        _outer: &Focus,
    ) -> R<()> {
        let size = nodes.len();
        for (i, &node) in nodes.iter().enumerate() {
            self.tick(1)?;
            let focus = Focus {
                node,
                position: i + 1,
                size,
            };
            let saved_mode = std::mem::replace(&mut self.mode, mode);
            let result = match self.find_template(node, mode, None)? {
                Some(t) => self.call_rule(t, &focus, params),
                None => self.builtin(node, mode, &focus),
            };
            self.mode = saved_mode;
            result?;
        }
        Ok(())
    }

    /// Instantiates a template as the current template rule.
    fn call_rule(&mut self, index: usize, focus: &Focus, params: &[(Name, Value)]) -> R<()> {
        let saved = self.current_rule.replace(index);
        let result = self.call(index, focus, params);
        self.current_rule = saved;
        result
    }

    // ---------- xsl:number ----------

    /// Whether `node` matches any alternative of `p`.
    fn pattern_matches(&mut self, p: &Pattern, node: NodeRef) -> R<bool> {
        for alt in &p.alternatives {
            if self.matches(alt, node)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// libxslt's xsltTestCompMatchCount: the count pattern, or by default a
    /// node of the same type and, if it has one, the same expanded name as
    /// the current node.
    fn counts(&mut self, count: Option<&Pattern>, node: NodeRef, current: NodeRef) -> R<bool> {
        if let Some(p) = count {
            return self.pattern_matches(p, node);
        }
        if (node.ns > 0) != (current.ns > 0) {
            return Ok(false);
        }
        if node.ns > 0 {
            return Ok(true);
        }
        let (a, b) = (
            self.docs[node.doc as usize].kind(node.node),
            self.docs[current.doc as usize].kind(current.node),
        );
        Ok(match (a, b) {
            (NodeKind::Element { name: x, .. }, NodeKind::Element { name: y, .. })
            | (NodeKind::Attribute { name: x, .. }, NodeKind::Attribute { name: y, .. }) => {
                x.expanded() == y.expanded()
            }
            (NodeKind::Pi { target: x, .. }, NodeKind::Pi { target: y, .. }) => x == y,
            _ => std::mem::discriminant(&a) == std::mem::discriminant(&b),
        })
    }

    /// The numbers xsl:number counts, outermost first; None when there's
    /// nothing to write (single and multiple levels with no counted node).
    fn count_numbers(
        &mut self,
        level: NumberLevel,
        count: Option<&Pattern>,
        from: Option<&Pattern>,
        node: NodeRef,
        _location: &Location,
    ) -> R<Option<Vec<f64>>> {
        // Documents can be loaded while matching, so each access borrows
        // anew.
        fn parent(docs: &[Document], n: NodeRef) -> Option<NodeRef> {
            if n.ns > 0 {
                return Some(NodeRef::new(n.doc, n.node));
            }
            docs[n.doc as usize]
                .parent(n.node)
                .map(|p| NodeRef::new(n.doc, p))
        }
        fn kind(docs: &[Document], n: NodeRef) -> NodeKind {
            docs[n.doc as usize].kind(n.node)
        }
        let is_root =
            |docs: &[Document], n: NodeRef| n.ns == 0 && matches!(kind(docs, n), NodeKind::Root);
        match level {
            NumberLevel::Invalid => Ok(None),
            NumberLevel::Single | NumberLevel::Multiple => {
                // xsltNumberFormatGetMultipleLevel: up the ancestor-or-self
                // axis, stopping at the root or a node matching `from`.
                let mut numbers = Vec::new();
                let mut ancestor = Some(node);
                while let Some(a) = ancestor {
                    if is_root(&self.docs, a) {
                        break;
                    }
                    if let Some(f) = from
                        && self.pattern_matches(f, a)?
                    {
                        break;
                    }
                    if self.counts(count, a, node)? {
                        let mut n = 1.0;
                        if a.ns == 0 && !matches!(kind(&self.docs, a), NodeKind::Attribute { .. }) {
                            let mut p = self.docs[a.doc as usize].prev_sibling(a.node);
                            while let Some(s) = p {
                                if self.counts(count, NodeRef::new(a.doc, s), node)? {
                                    n += 1.0;
                                }
                                p = self.docs[a.doc as usize].prev_sibling(s);
                            }
                        }
                        numbers.push(n);
                        if level == NumberLevel::Single {
                            break;
                        }
                    }
                    ancestor = parent(&self.docs, a);
                }
                if numbers.is_empty() {
                    return Ok(None);
                }
                numbers.reverse();
                Ok(Some(numbers))
            }
            NumberLevel::Any => {
                // xsltNumberFormatGetAnyLevel: back through the document,
                // preceding nodes and ancestors, until the root or a node
                // matching `from` (counted if it matches `count`).
                let mut n = 0.0;
                let mut cur = Some(node);
                while let Some(c) = cur {
                    if self.counts(count, c, node)? {
                        n += 1.0;
                    }
                    if let Some(f) = from
                        && self.pattern_matches(f, c)?
                    {
                        break;
                    }
                    if is_root(&self.docs, c) {
                        break;
                    }
                    let doc = &self.docs[c.doc as usize];
                    cur = if c.ns > 0 || matches!(doc.kind(c.node), NodeKind::Attribute { .. }) {
                        parent(&self.docs, c)
                    } else if let Some(mut p) = doc.prev_sibling(c.node) {
                        while let Some(l) = doc.last_child(p) {
                            p = l;
                        }
                        Some(NodeRef::new(c.doc, p))
                    } else {
                        parent(&self.docs, c)
                    };
                }
                Ok(Some(vec![n]))
            }
        }
    }

    fn apply_imports(&mut self, focus: &Focus, location: &Location) -> R<()> {
        let Some(current) = self.current_rule else {
            self.report(
                location,
                "It is an error to call 'apply-imports' when there's no current template rule.",
            );
            return Ok(());
        };
        let below = self.ss.templates[current].precedence;
        let mode = self.mode;
        match self.find_template(focus.node, mode, Some(below))? {
            Some(t) => self.call_rule(t, focus, &[]),
            None => self.builtin(focus.node, mode, focus),
        }
    }

    fn builtin(&mut self, node: NodeRef, mode: Option<Name>, focus: &Focus) -> R<()> {
        if node.ns > 0 {
            return Ok(());
        }
        match self.docs[node.doc as usize].kind(node.node) {
            NodeKind::Root | NodeKind::Element { .. } => {
                let doc = &self.docs[node.doc as usize];
                let children: Vec<NodeRef> = doc
                    .children(node.node)
                    .map(|c| NodeRef::new(node.doc, c))
                    .collect();
                self.apply(&children, mode, &[], focus)
            }
            NodeKind::Text(_) | NodeKind::Attribute { .. } => {
                let doc = &self.docs[node.doc as usize];
                let text = doc.value(node.node).unwrap_or("").to_string();
                let unescaped = doc.text_kind(node.node) == crate::tree::TextKind::Unescaped;
                let (out, names) = self.out();
                out.text(&text, unescaped, names).map_err(Self::output_err)
            }
            NodeKind::Comment(_) | NodeKind::Pi { .. } => Ok(()),
        }
    }

    fn call(&mut self, index: usize, focus: &Focus, params: &[(Name, Value)]) -> R<()> {
        let t = &self.ss.templates[index];
        if self.depth >= MAX_DEPTH {
            return Err(ExecError::Terminated(format!(
                "{}\nxsltApplySequenceConstructor: A potential infinite template recursion was detected.\n\
                 You can adjust xsltMaxDepth (--maxdepth) in order to raise the maximum number of nested template calls and variables/params (currently set to {MAX_DEPTH}).",
                where_(&t.location)
            )));
        }
        self.depth += 1;
        let saved = std::mem::take(&mut self.locals);
        let mut result = Ok(());
        for p in &t.params {
            let value = match params.iter().find(|(n, _)| *n == p.name) {
                Some((_, v)) => v.clone(),
                None => match self.var_value(&p.value, focus, &p.location) {
                    Ok(v) => v,
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                },
            };
            self.locals.push((p.name, value));
        }
        if result.is_ok() {
            result = self.body(&t.body, focus);
        }
        self.locals = saved;
        self.depth -= 1;
        result
    }

    fn with_params(&mut self, params: &[Variable], focus: &Focus) -> R<Vec<(Name, Value)>> {
        let mut out = Vec::with_capacity(params.len());
        for p in params {
            out.push((p.name, self.var_value(&p.value, focus, &p.location)?));
        }
        Ok(out)
    }

    fn sort(
        &mut self,
        nodes: Vec<NodeRef>,
        sorts: &[Sort],
        outer: &Focus,
        loc: &Location,
    ) -> R<Vec<NodeRef>> {
        // libxslt doesn't sort, or evaluate the keys of, fewer than 2 nodes.
        if sorts.is_empty() || nodes.len() <= 1 {
            return Ok(nodes);
        }
        enum Key {
            Text(String),
            Number(f64),
        }
        let size = nodes.len();
        let mut specs = Vec::new();
        for s in sorts {
            let numeric = match &s.data_type {
                Some(a) => self.avt(a, outer, loc)? == "number",
                None => false,
            };
            let descending = match &s.order {
                Some(a) => self.avt(a, outer, loc)? == "descending",
                None => false,
            };
            specs.push((numeric, descending));
        }
        let mut keyed = Vec::with_capacity(size);
        for (i, &n) in nodes.iter().enumerate() {
            let focus = Focus {
                node: n,
                position: i + 1,
                size,
            };
            let mut keys = Vec::with_capacity(sorts.len());
            for (s, &(numeric, _)) in sorts.iter().zip(&specs) {
                let text = self.eval_string(&s.select, &focus, loc)?;
                keys.push(if numeric {
                    Key::Number(xpath::number::from_string(&text))
                } else {
                    Key::Text(text)
                });
            }
            keyed.push((n, keys));
        }
        keyed.sort_by(|(_, a), (_, b)| {
            for ((x, y), &(_, descending)) in a.iter().zip(b).zip(&specs) {
                let ord = match (x, y) {
                    (Key::Text(x), Key::Text(y)) => x.cmp(y),
                    // NaN sorts before every number.
                    (Key::Number(x), Key::Number(y)) => match (x.is_nan(), y.is_nan()) {
                        (true, true) => std::cmp::Ordering::Equal,
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => x.total_cmp(y),
                    },
                    _ => std::cmp::Ordering::Equal,
                };
                let ord = if descending { ord.reverse() } else { ord };
                if ord.is_ne() {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        });
        Ok(keyed.into_iter().map(|(n, _)| n).collect())
    }

    // ---------- instructions ----------

    fn body(&mut self, body: &Body, focus: &Focus) -> R<()> {
        let mark = self.locals.len();
        let mut result = Ok(());
        for instr in body {
            result = self.instr(instr, focus);
            self.flush_warnings();
            if result.is_err() {
                break;
            }
        }
        self.locals.truncate(mark);
        result
    }

    fn flush_warnings(&mut self) {
        for w in self.warnings.get_mut().drain(..) {
            let _ = writeln!(self.messages, "{w}");
        }
    }

    fn instr(&mut self, instr: &Instr, focus: &Focus) -> R<()> {
        self.tick(1)?;
        match instr {
            Instr::Text(s) => {
                let (out, names) = self.out();
                out.text(s, false, names).map_err(Self::output_err)
            }
            Instr::UnescapedText(s) => {
                let (out, names) = self.out();
                out.text(s, true, names).map_err(Self::output_err)
            }
            Instr::LiteralElement {
                name,
                namespaces,
                inherited,
                attributes,
                attribute_sets,
                body,
                location,
            } => {
                let (out, names) = self.out();
                out.start_element(*name, names).map_err(Self::output_err)?;
                for &b in namespaces {
                    out.add_namespace(b);
                }
                for &b in inherited {
                    out.add_late_namespace(b);
                }
                self.attribute_sets(attribute_sets, focus)?;
                for (q, avt) in attributes {
                    let value = self.avt(avt, focus, location)?;
                    self.out
                        .last_mut()
                        .unwrap()
                        .add_attribute(*q, value)
                        .map_err(Self::output_err)?;
                }
                self.body(body, focus)?;
                let (out, names) = self.out();
                out.end_element(names).map_err(Self::output_err)
            }
            Instr::ValueOf {
                select,
                unescaped,
                location,
            } => {
                let s = self.eval_string(select, focus, location)?;
                let (out, names) = self.out();
                out.text(&s, *unescaped, names).map_err(Self::output_err)
            }
            Instr::ApplyTemplates {
                select,
                mode,
                sorts,
                params,
                location,
            } => {
                let nodes = match select {
                    // xsltApplyTemplates stops the transform on anything else.
                    Some(e) => self.eval_nodes(e, focus, location)?.ok_or_else(|| {
                        ExecError::Terminated(format!(
                            "{}\nThe 'select' expression did not evaluate to a node set.",
                            where_(location)
                        ))
                    })?,
                    None => {
                        if focus.node.ns > 0 {
                            Vec::new()
                        } else {
                            let doc = &self.docs[focus.node.doc as usize];
                            doc.children(focus.node.node)
                                .map(|c| NodeRef::new(focus.node.doc, c))
                                .collect()
                        }
                    }
                };
                let nodes = self.sort(nodes, sorts, focus, location)?;
                let params = self.with_params(params, focus)?;
                self.apply(&nodes, *mode, &params, focus)
            }
            Instr::CallTemplate {
                name,
                params,
                location,
            } => {
                let Some(&index) = self.ss.named.get(name) else {
                    // xsltCallTemplate reports it and goes on.
                    let message = format!(
                        "The called template '{}' was not found",
                        self.names.str(name.1)
                    );
                    self.report(location, &message);
                    return Ok(());
                };
                let params = self.with_params(params, focus)?;
                self.call(index, focus, &params)
            }
            Instr::ForEach {
                select,
                sorts,
                body,
                location,
            } => {
                // xsltForEach reports anything else and goes on.
                let Some(nodes) = self.eval_nodes(select, focus, location)? else {
                    self.report(
                        location,
                        "xsl:for-each: select didn't evaluate to a node list",
                    );
                    return Ok(());
                };
                let nodes = self.sort(nodes, sorts, focus, location)?;
                let size = nodes.len();
                // There's no current template rule inside xsl:for-each.
                let saved = self.current_rule.take();
                let mut result = Ok(());
                for (i, &node) in nodes.iter().enumerate() {
                    result = self.body(
                        body,
                        &Focus {
                            node,
                            position: i + 1,
                            size,
                        },
                    );
                    if result.is_err() {
                        break;
                    }
                }
                self.current_rule = saved;
                result
            }
            Instr::If {
                test,
                body,
                location,
            } => {
                if xpath::boolean(&self.eval(test, focus, location)?) {
                    self.body(body, focus)?;
                }
                Ok(())
            }
            Instr::Choose { whens, otherwise } => {
                for (test, body, location) in whens {
                    if xpath::boolean(&self.eval(test, focus, location)?) {
                        return self.body(body, focus);
                    }
                }
                if let Some(body) = otherwise {
                    self.body(body, focus)?;
                }
                Ok(())
            }
            Instr::Variable(v) => {
                // A local may not redefine another local of the same template.
                if self.locals.iter().any(|(n, _)| *n == v.name) {
                    // xsltRegisterVariable reports it and doesn't add the
                    // variable: the first one stays.
                    let message = format!(
                        "XSLT-variable: Redefinition of variable '{}'.",
                        self.names.str(v.name.1)
                    );
                    self.report(&v.location, &message);
                    return Ok(());
                }
                let value = self.var_value(&v.value, focus, &v.location)?;
                self.locals.push((v.name, value));
                Ok(())
            }
            Instr::Copy {
                attribute_sets,
                body,
            } => self.copy(focus, attribute_sets, body),
            Instr::CopyOf { select, location } => match self.eval(select, focus, location)? {
                Value::NodeSet(nodes) => {
                    for n in nodes {
                        self.copy_of(n, true)?;
                    }
                    Ok(())
                }
                v => {
                    let s = self
                        .env_eval(focus, "", &[], |env| Ok(xpath::to_string(&v, env)))
                        .unwrap_or_default();
                    let (out, names) = self.out();
                    out.text(&s, false, names).map_err(Self::output_err)
                }
            },
            Instr::Element {
                name,
                namespace,
                scope,
                attribute_sets,
                body,
                location,
            } => {
                let name = self.avt(name, focus, location)?;
                let ns = match namespace {
                    Some(a) => Some(self.avt(a, focus, location)?),
                    None => None,
                };
                // libxslt reports an invalid name or unbound prefix and
                // skips the instruction.
                let q = match self.computed_name(&name, ns.as_deref(), scope, true, location) {
                    Ok(q) => q,
                    Err(ExecError::Runtime(m)) => {
                        self.report_plain(&m);
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                };
                let (out, names) = self.out();
                out.start_element(q, names).map_err(Self::output_err)?;
                self.attribute_sets(attribute_sets, focus)?;
                self.body(body, focus)?;
                let (out, names) = self.out();
                out.end_element(names).map_err(Self::output_err)
            }
            Instr::Attribute {
                name,
                namespace,
                scope,
                body,
                location,
            } => {
                let name = self.avt(name, focus, location)?;
                let ns = match namespace {
                    Some(a) => Some(self.avt(a, focus, location)?),
                    None => None,
                };
                // libxslt reports an invalid name or unbound prefix and
                // skips the instruction.
                let q = match self.computed_name(&name, ns.as_deref(), scope, false, location) {
                    Ok(q) => q,
                    Err(ExecError::Runtime(m)) => {
                        self.report_plain(&m);
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                };
                if q.ns
                    .is_some_and(|u| self.names.str(u) == "http://www.w3.org/2000/xmlns/")
                {
                    self.report(
                        location,
                        "xsl:attribute: Namespace http://www.w3.org/2000/xmlns/ forbidden.",
                    );
                    return Ok(());
                }
                if q.ns.is_none() && q.prefix.is_none() && self.names.str(q.local) == "xmlns" {
                    self.report(
                        location,
                        "xsl:attribute: The effective name 'xmlns' is not allowed.",
                    );
                    return Ok(());
                }
                let value = self.body_string(body, focus)?;
                let mut q = q;
                if let Some(uri) = q.ns {
                    let (out, names) = self.out();
                    if let Some(prefix) = out.attribute_prefix(uri, q.prefix, names) {
                        q.prefix = Some(prefix);
                    }
                }
                if let Err(OutputError::AttributeNotAllowed) =
                    self.out.last_mut().unwrap().add_attribute(q, value)
                {
                    self.report(
                        location,
                        "xsl:attribute: Cannot add attributes to an element if children have been already added to the element.",
                    );
                    return Ok(());
                }
                Ok(())
            }
            Instr::Comment(body) => {
                let s = self.body_string(body, focus)?;
                if s.contains("--") || s.ends_with('-') {
                    // xsltComment reports it and adds the comment anyway (the
                    // result is discarded, as after any error).
                    self.report_plain(
                        "runtime error: xsl:comment : '--' or ending '-' not allowed in comment",
                    );
                }
                let (out, names) = self.out();
                out.comment(&s, names).map_err(Self::output_err)
            }
            Instr::ProcessingInstruction {
                name,
                body,
                location,
            } => {
                let target = self.avt(name, focus, location)?;
                let data = self.body_string(body, focus)?;
                if data.contains("?>") {
                    self.report(
                        location,
                        "xsl:processing-instruction: '?>' not allowed within PI content",
                    );
                    return Ok(());
                }
                let (out, names) = self.out();
                let target = names.intern(target.trim());
                out.pi(target, &data, names).map_err(Self::output_err)
            }
            Instr::Message { terminate, body } => {
                let text = self.body_string(body, focus)?;
                let _ = writeln!(self.messages, "{text}");
                if *terminate {
                    return Err(ExecError::Terminated(String::new()));
                }
                Ok(())
            }
            Instr::ApplyImports { location } => self.apply_imports(focus, location),
            Instr::Number {
                value,
                level,
                count,
                from,
                format,
                grouping,
                location,
            } => {
                let format = number::Format::parse(&self.avt(format, focus, location)?);
                let numbers = match value {
                    // The compiler wraps `value` in number(), as libxslt does.
                    // libxslt evaluates "number(value)" by itself: a failure
                    // only means no number. An undefined variable is reported
                    // (by the variable lookup) but doesn't stop anything.
                    Some(v) => match self.eval_unmapped(v, focus, location)? {
                        Ok(Value::Number(n)) => Some(vec![n]),
                        Ok(_) => None,
                        Err(e) => {
                            if e.message.starts_with("Undefined variable") {
                                self.report(location, &format!("XPath error : {}", e.message));
                            }
                            None
                        }
                    },
                    None => self.count_numbers(
                        *level,
                        count.as_ref(),
                        from.as_ref(),
                        focus.node,
                        location,
                    )?,
                };
                // libxslt inserts the formatted number with
                // xsltCopyTextString even when it's empty, leaving an empty
                // text node (so `<out></out>`, not `<out/>`).
                let text = match numbers {
                    Some(numbers) => {
                        let (text, warning) = number::insert_numbers(&numbers, &format, *grouping);
                        if let Some(w) = warning {
                            self.warnings.borrow_mut().push(w);
                        }
                        text
                    }
                    None => String::new(),
                };
                let (out, names) = self.out();
                if text.is_empty() {
                    out.empty_text(names).map_err(Self::output_err)
                } else {
                    out.text(&text, false, names).map_err(Self::output_err)
                }
            }
            Instr::Document {
                href,
                settings,
                body,
                location,
            } => self.document_element(href.as_ref(), settings, body, focus, location),
            Instr::Unsupported { what, location } => Err(ExecError::Runtime(format!(
                "{}\n{what} isn't supported yet",
                where_(location)
            ))),
        }
    }

    /// Resolves the name of `xsl:element` or `xsl:attribute`. Without a
    /// `namespace` attribute, the prefix is resolved in the stylesheet; an
    /// unprefixed element name takes the default namespace, an unprefixed
    /// attribute name doesn't.
    fn computed_name(
        &mut self,
        name: &str,
        ns: Option<&str>,
        scope: &[Binding],
        element: bool,
        loc: &Location,
    ) -> R<QName> {
        let name = name.trim();
        let (prefix, local) = match name.split_once(':') {
            Some((p, l)) => (Some(p), l),
            None => (None, name),
        };
        let valid = |s: &str| {
            !s.is_empty()
                && !s.contains(':')
                && !s.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.')
        };
        if !valid(local) || prefix.is_some_and(|p| !valid(p)) {
            return Err(ExecError::Runtime(format!(
                "{}\nInvalid QName '{name}'",
                where_(loc)
            )));
        }
        let uri = match ns {
            Some(u) => (!u.is_empty()).then(|| self.names.intern(u)),
            None => match prefix {
                Some("xml") => Some(self.names.intern(XML_NS)),
                Some(p) => {
                    let found = scope
                        .iter()
                        .find(|(q, _)| q.is_some_and(|q| self.names.str(q) == p))
                        .map(|&(_, u)| u);
                    Some(found.ok_or_else(|| {
                        ExecError::Runtime(format!(
                            "{}\nUndefined namespace prefix {p}",
                            where_(loc)
                        ))
                    })?)
                }
                None if element => scope
                    .iter()
                    .find(|(q, _)| q.is_none())
                    .map(|&(_, u)| u)
                    .filter(|&u| !self.names.str(u).is_empty()),
                None => None,
            },
        };
        let local = self.names.intern(local);
        let prefix = prefix.map(|p| self.names.intern(p));
        Ok(QName {
            ns: uri,
            local,
            prefix,
        })
    }

    fn attribute_sets(&mut self, sets: &[Name], focus: &Focus) -> R<()> {
        for name in sets {
            self.attribute_set(*name, focus, 0)?;
        }
        Ok(())
    }

    fn attribute_set(&mut self, name: Name, focus: &Focus, depth: usize) -> R<()> {
        let ss = self.ss;
        let Some(set) = ss.attribute_sets.get(&name) else {
            return Err(ExecError::Runtime(format!(
                "Attribute set '{}' not found",
                self.names.str(name.1)
            )));
        };
        if depth > 100 {
            return Err(ExecError::Runtime(
                "Attribute sets use each other in a cycle".into(),
            ));
        }
        for &used in &set.uses {
            self.attribute_set(used, focus, depth + 1)?;
        }
        // By the spec attribute sets see only global variables, but libxslt
        // evaluates them with the caller's locals visible (attribset44).
        for instr in &set.attributes {
            self.instr(instr, focus)?;
        }
        Ok(())
    }

    /// `xsl:copy`: a shallow copy of the context node.
    fn copy(&mut self, focus: &Focus, sets: &[Name], body: &Body) -> R<()> {
        let n = focus.node;
        if n.ns > 0 {
            let binding = self.namespace_node(n);
            self.out.last_mut().unwrap().add_namespace(binding);
            return Ok(());
        }
        let doc = &self.docs[n.doc as usize];
        match doc.kind(n.node) {
            NodeKind::Root => self.body(body, focus),
            NodeKind::Element { name, .. } => {
                let decls: Vec<Binding> = doc
                    .ns_decls(n.node)
                    .iter()
                    .map(|d| (d.prefix, d.uri))
                    .collect();
                let (out, names) = self.out();
                out.start_element(name, names).map_err(Self::output_err)?;
                for b in decls {
                    out.add_namespace(b);
                }
                self.attribute_sets(sets, focus)?;
                self.body(body, focus)?;
                let (out, names) = self.out();
                out.end_element(names).map_err(Self::output_err)
            }
            _ => self.copy_of(n, false),
        }
    }

    fn namespace_node(&self, n: NodeRef) -> Binding {
        let vars = Vars {
            locals: &[],
            globals: &self.globals,
            lazy: None,
        };
        let funcs = XsltFunctions {
            current: n,
            budget: &self.budget,
            ids: &self.ids,
            keys: &self.keys,
            loads: &self.loads,
            made: &self.made,
            serial: 0,
            token: self.token,
            base: "",
            namespaces: &[],
            ss: self.ss,
            warnings: &self.warnings,
            wanted_globals: &self.wanted_globals,
            dyn_exprs: &self.dyn_exprs,
        };
        let env = Env {
            docs: &self.docs,
            names: self.names,
            vars: &vars,
            funcs: &funcs,
        };
        env.namespaces(n)[n.ns as usize - 1]
    }

    /// `xsl:copy-of` of one node. `top`: the node itself was selected, so an
    /// element copies all its in-scope namespaces, not just its own declarations.
    fn copy_of(&mut self, n: NodeRef, top: bool) -> R<()> {
        self.tick(1)?;
        if n.ns > 0 {
            let binding = self.namespace_node(n);
            self.out.last_mut().unwrap().add_namespace(binding);
            return Ok(());
        }
        let d = n.doc as usize;
        match self.docs[d].kind(n.node) {
            NodeKind::Root => {
                let children: Vec<NodeId> = self.docs[d].children(n.node).collect();
                for c in children {
                    self.copy_of(NodeRef::new(n.doc, c), false)?;
                }
                Ok(())
            }
            NodeKind::Element { name, .. } => {
                let bindings: Vec<Binding> = if top {
                    // xmlGetNsList's order: the element's own declarations,
                    // then each ancestor's new ones.
                    let doc = &self.docs[d];
                    let mut seen = Vec::new();
                    let mut list = Vec::new();
                    let mut cur = Some(n.node);
                    while let Some(e) = cur {
                        for x in doc.ns_decls(e) {
                            if !seen.contains(&x.prefix) {
                                seen.push(x.prefix);
                                list.push((x.prefix, x.uri));
                            }
                        }
                        cur = doc.parent(e);
                    }
                    list
                } else {
                    self.docs[d]
                        .ns_decls(n.node)
                        .iter()
                        .map(|x| (x.prefix, x.uri))
                        .collect()
                };
                let attrs: Vec<(QName, String, bool)> = self.docs[d]
                    .attributes(n.node)
                    .map(|a| {
                        (
                            self.docs[d].name(a).unwrap(),
                            self.docs[d].value(a).unwrap_or("").to_string(),
                            self.docs[d].is_valueless(a),
                        )
                    })
                    .collect();
                let children: Vec<NodeId> = self.docs[d].children(n.node).collect();
                let (out, names) = self.out();
                out.start_element(name, names).map_err(Self::output_err)?;
                for b in bindings {
                    out.add_namespace(b);
                }
                for (q, v, valueless) in attrs {
                    if valueless {
                        out.add_valueless_attribute(q).map_err(Self::output_err)?;
                    } else {
                        out.add_attribute(q, v).map_err(Self::output_err)?;
                    }
                }
                for c in children {
                    self.copy_of(NodeRef::new(n.doc, c), false)?;
                }
                let (out, names) = self.out();
                out.end_element(names).map_err(Self::output_err)
            }
            NodeKind::Attribute { name, .. } => {
                let value = self.docs[d].value(n.node).unwrap_or("").to_string();
                let out = self.out.last_mut().unwrap();
                let added = if self.docs[d].is_valueless(n.node) {
                    out.add_valueless_attribute(name)
                } else {
                    out.add_attribute(name, value)
                };
                if added.is_err() {
                    self.report_plain(
                        "runtime error: Attribute nodes must be added before any child nodes to an element.",
                    );
                }
                Ok(())
            }
            // The source documents and the output are separate fields, so
            // text is copied straight from one to the other.
            NodeKind::Text(_) => {
                let doc = &self.docs[d];
                let s = doc.value(n.node).unwrap_or("");
                // Unescaped text in a fragment stays unescaped when copied.
                let unescaped = doc.text_kind(n.node) == crate::tree::TextKind::Unescaped;
                let out = self.out.last_mut().expect("an output is open");
                out.text(s, unescaped, self.names).map_err(Self::output_err)
            }
            NodeKind::Comment(_) => {
                let s = self.docs[d].value(n.node).unwrap_or("");
                let out = self.out.last_mut().expect("an output is open");
                out.comment(s, self.names).map_err(Self::output_err)
            }
            NodeKind::Pi { target, .. } => {
                let s = self.docs[d].value(n.node).unwrap_or("");
                let out = self.out.last_mut().expect("an output is open");
                out.pi(target, s, self.names).map_err(Self::output_err)
            }
        }
    }
}

// ---------- patterns ----------

/// A node's kind and name, as far as choosing template rules goes. Root,
/// namespace and processing-instruction nodes are `Other`, which every rule
/// is a candidate for.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
enum NodeKey {
    Element(Name),
    Attribute(Name),
    Text,
    Comment,
    Other,
}

/// Whether `alt` could match a node with `key`, judged by its last step's
/// axis and node test alone, as `step_matches` and `matches_test` would.
/// False only when it certainly can't; the full match decides the rest.
fn could_match(alt: &Alternative, key: NodeKey) -> bool {
    if alt.never {
        return false;
    }
    // An anchor alone (`/`, `id('x')`, `key('k', 'v')`) can match anything.
    let Some(last) = alt.steps.last() else {
        return true;
    };
    let attribute_axis = last.axis == Axis::Attribute;
    let named = |name: Name| match &last.test {
        NodeTest::Node | NodeTest::Any => true,
        NodeTest::Name(n) => *n == name,
        NodeTest::Namespace(ns) => *ns == name.0,
        NodeTest::Text | NodeTest::Comment | NodeTest::Pi(_) => false,
    };
    match key {
        NodeKey::Element(name) => !attribute_axis && named(name),
        NodeKey::Attribute(name) => attribute_axis && named(name),
        NodeKey::Text => !attribute_axis && matches!(last.test, NodeTest::Node | NodeTest::Text),
        NodeKey::Comment => {
            !attribute_axis && matches!(last.test, NodeTest::Node | NodeTest::Comment)
        }
        NodeKey::Other => true,
    }
}

fn match_steps(
    steps: &[PatternStep],
    anchor: &Anchor,
    node: NodeRef,
    env: &Env,
) -> Result<bool, XPathError> {
    let Some((last, rest)) = steps.split_last() else {
        return match anchor {
            Anchor::Root => Ok(xpath::is_root(node, env)),
            Anchor::Call(e) => in_set(e, node, env),
            Anchor::None => Ok(false),
        };
    };
    if !step_matches(last, node, env)? {
        return Ok(false);
    }
    let parent = env.parent(node);
    if rest.is_empty() {
        return match anchor {
            Anchor::None => Ok(true),
            Anchor::Root => Ok(last.descendant || parent.is_some_and(|p| xpath::is_root(p, env))),
            Anchor::Call(e) => {
                if last.descendant {
                    let mut cur = parent;
                    while let Some(a) = cur {
                        if in_set(e, a, env)? {
                            return Ok(true);
                        }
                        cur = env.parent(a);
                    }
                    Ok(false)
                } else {
                    match parent {
                        Some(p) => in_set(e, p, env),
                        None => Ok(false),
                    }
                }
            }
        };
    }
    if last.descendant {
        let mut cur = parent;
        while let Some(a) = cur {
            if match_steps(rest, anchor, a, env)? {
                return Ok(true);
            }
            cur = env.parent(a);
        }
        Ok(false)
    } else {
        match parent {
            Some(p) => match_steps(rest, anchor, p, env),
            None => Ok(false),
        }
    }
}

fn in_set(e: &Expr, node: NodeRef, env: &Env) -> Result<bool, XPathError> {
    use crate::xpath::ast::{CoreFn, Function};
    // In a pattern, libxslt looks up id('…')'s whole literal as one ID
    // (xsltTestCompMatch's XSLT_OP_ID); it doesn't split it into tokens as
    // the XPath function does, so id('b d c') matches nothing.
    if let Expr::Call(Function::Core(CoreFn::Id), args) = e
        && let [Expr::Literal(id)] = args.as_slice()
    {
        let doc = env.doc(node);
        return Ok(node.ns == 0 && doc.element_by_id(id) == Some(node.node));
    }
    Ok(xpath::evaluate_nodes(e, env, &Focus::single(node))?.contains(&node))
}

fn step_matches(step: &PatternStep, node: NodeRef, env: &Env) -> Result<bool, XPathError> {
    let is_attr = xpath::is_attribute(node, env);
    let ok_kind = match step.axis {
        Axis::Attribute => is_attr,
        _ => !is_attr && node.ns == 0 && !xpath::is_root(node, env),
    };
    if !ok_kind || !xpath::node_matches(&step.test, step.axis, node, env) {
        return Ok(false);
    }
    if step.predicates.is_empty() {
        return Ok(true);
    }
    // Predicates that can't depend on position are checked on the node
    // alone. Ranking it among its siblings costs time proportional to their
    // number, which made matching quadratic (darktable's patterns).
    if step.predicates.iter().all(xpath::is_position_independent) {
        let focus = Focus::single(node);
        for p in &step.predicates {
            if !xpath::boolean(&xpath::evaluate(p, env, &focus)?) {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let Some(parent) = env.parent(node) else {
        return Ok(false);
    };
    Ok(xpath::step_nodes(step.axis, &step.test, &step.predicates, parent, env)?.contains(&node))
}

// ---------- variables and functions ----------

/// Global definitions by name, and the globals an evaluation asked for.
type LazyGlobals<'a> = (&'a SymMap<Name, usize>, &'a RefCell<Vec<Name>>);

struct Vars<'a> {
    locals: &'a [(Name, Value)],
    globals: &'a SymMap<Name, Value>,
    /// Globals not computed yet, and those an evaluation asked for.
    /// libxslt computes a global when it's first referenced.
    lazy: Option<LazyGlobals<'a>>,
}

impl Variables for Vars<'_> {
    fn get(&self, name: Name) -> Option<Value> {
        if let Some((_, v)) = self.locals.iter().rev().find(|(n, _)| *n == name) {
            return Some(v.clone());
        }
        if let Some(v) = self.globals.get(&name) {
            return Some(v.clone());
        }
        if let Some((defs, wanted)) = self.lazy
            && defs.contains_key(&name)
        {
            wanted.borrow_mut().push(name);
        }
        None
    }
}

/// One key over one document: each value to the nodes with it, in
/// document order.
type KeyIndex = HashMap<String, Vec<NodeRef>>;

/// Key indexes for each (document, key name), built for all keys of a
/// document at once.
#[derive(Default)]
struct Keys {
    built: HashMap<(u32, Name), Rc<KeyIndex>>,
    /// Documents whose keys are built.
    done: std::collections::HashSet<u32>,
    /// Documents whose keys are being built, to catch a key that uses one.
    building: Vec<u32>,
}

/// Documents loaded by `document()`, by URL (None: it couldn't be
/// loaded), and those it asked for that aren't loaded yet.
#[derive(Default)]
struct DocLoads {
    by_url: HashMap<String, Option<u32>>,
    pending: Vec<String>,
}

/// Result tree fragments made by extension functions during evaluations,
/// keyed by (evaluation serial, call number within the evaluation).
#[derive(Default)]
struct Made {
    by_key: HashMap<(u64, u64), u32>,
    pending: Vec<((u64, u64), Document)>,
    /// Fragment-making calls so far in this run of an evaluation.
    calls: u64,
}

struct XsltFunctions<'a> {
    /// XSLT's current node: the context node of the instruction.
    current: NodeRef,
    budget: &'a Budget,
    made: &'a RefCell<Made>,
    serial: u64,
    token: crate::tree::Sym,
    ids: &'a RefCell<SymMap<NodeRef, usize>>,
    keys: &'a RefCell<Keys>,
    loads: &'a RefCell<DocLoads>,
    /// The base URI of the instruction being run, for document().
    base: &'a str,
    /// Its in-scope namespaces, for QNames given as strings.
    namespaces: &'a [Binding],
    ss: &'a Stylesheet,
    warnings: &'a RefCell<Vec<String>>,
    wanted_globals: &'a RefCell<Vec<Name>>,
    dyn_exprs: &'a RefCell<DynExprs>,
}

/// dyn:evaluate's expressions by text and namespace context: compiled, or
/// the compile error; and those an evaluation asked for.
#[derive(Default)]
struct DynExprs {
    compiled: HashMap<(String, Vec<Binding>), Result<Rc<Expr>, String>>,
    pending: Vec<(String, Vec<Binding>)>,
}

/// A QName from a string, in the {uri}local form the compiler gives
/// literal prefixed names, as an interned name. None if a part isn't
/// interned, so nothing can have that name.
fn runtime_qname(qname: &str, names: &NamePool) -> Option<Name> {
    let qname = qname.trim();
    let (uri, local) = match qname.strip_prefix('{').and_then(|r| r.split_once('}')) {
        Some((u, l)) => (Some(u), l),
        None => (None, qname),
    };
    let ns = match uri {
        Some(u) => Some(names.get(u)?),
        None => None,
    };
    Some((ns, names.get(local)?))
}

impl XsltFunctions<'_> {
    /// dyn:evaluate (libexslt's exsltDynEvaluateFunction): the string
    /// compiled as XPath in the instruction's namespace context and
    /// evaluated in the current context. An empty string, or one that
    /// doesn't compile or evaluate, gives an empty node-set and an error
    /// message, not a failure. Requests to load, compute or compile
    /// something first, and a spent budget, still fail, so the evaluation
    /// runs again.
    fn dyn_evaluate(&self, args: &[Value], env: &Env, focus: &Focus) -> Result<Value, XPathError> {
        let [arg] = args else {
            return Err(XPathError::runtime("Invalid number of arguments"));
        };
        let text = xpath::to_string(arg, env);
        if text.is_empty() {
            return Ok(Value::NodeSet(Vec::new()));
        }
        let key = (text, self.namespaces.to_vec());
        let compiled = self.dyn_exprs.borrow().compiled.get(&key).cloned();
        let unable = |detail: &str| {
            self.warnings.borrow_mut().push(format!(
                "{detail}\ndyn:evaluate() : unable to evaluate expression '{}'",
                key.0
            ));
            Ok(Value::NodeSet(Vec::new()))
        };
        match compiled {
            None => {
                self.dyn_exprs.borrow_mut().pending.push(key);
                Err(XPathError::runtime("dyn:evaluate() : compiling"))
            }
            Some(Err(message)) => unable(&message),
            Some(Ok(expr)) => match xpath::evaluate(&expr, env, focus) {
                Ok(v) => Ok(v),
                Err(e) if self.must_retry() => Err(e),
                Err(e) => unable(&e.to_string()),
            },
        }
    }

    /// Whether a failed evaluation asked for something to be done first,
    /// or ran out of budget: errors dyn:evaluate mustn't swallow.
    fn must_retry(&self) -> bool {
        !self.loads.borrow().pending.is_empty()
            || !self.made.borrow().pending.is_empty()
            || !self.wanted_globals.borrow().is_empty()
            || !self.dyn_exprs.borrow().pending.is_empty()
            || self.budget.exceeded.borrow().is_some()
    }

    /// The index of key `name` over document `doc`. As libxslt does
    /// (xsltInitAllDocKeys), the first lookup on a document indexes every
    /// key there, in `ss.key_order`. That's visible: a key whose `use`
    /// calls generate-id() numbers the document's nodes then.
    fn key_index(&self, doc: u32, name: Name, env: &Env) -> Result<Rc<KeyIndex>, XPathError> {
        if !self.keys.borrow().done.contains(&doc) {
            if self.keys.borrow().building.contains(&doc) {
                return Err(XPathError::runtime(format!(
                    "key() : the definition of key '{}' uses a key",
                    env.names.str(name.1)
                )));
            }
            self.keys.borrow_mut().building.push(doc);
            let result = self.build_keys(doc, env);
            self.keys.borrow_mut().building.pop();
            let indexes = result?;
            let mut keys = self.keys.borrow_mut();
            for (n, index) in indexes {
                keys.built.insert((doc, n), Rc::new(index));
            }
            keys.done.insert(doc);
        }
        Ok(self
            .keys
            .borrow()
            .built
            .get(&(doc, name))
            .cloned()
            .unwrap_or_default())
    }

    /// Every key over `doc`: one definition at a time, its matching nodes
    /// in document order.
    fn build_keys(&self, doc: u32, env: &Env) -> Result<HashMap<Name, KeyIndex>, XPathError> {
        let mut indexes: HashMap<Name, KeyIndex> = HashMap::new();
        for &(name, i) in &self.ss.key_order {
            let def = &self.ss.keys[&name][i];
            let index = indexes.entry(name).or_default();
            for id in env.docs[doc as usize].all_nodes() {
                env.funcs.tick(1)?;
                let node = NodeRef::new(doc, id);
                let mut matched = false;
                for alt in &def.matches.alternatives {
                    if !alt.never && match_steps(&alt.steps, &alt.anchor, node, env)? {
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    continue;
                }
                // current() in the use expression is the node being indexed.
                let funcs = XsltFunctions {
                    current: node,
                    budget: self.budget,
                    ids: self.ids,
                    keys: self.keys,
                    loads: self.loads,
                    made: self.made,
                    serial: self.serial,
                    token: self.token,
                    base: self.base,
                    namespaces: self.namespaces,
                    ss: self.ss,
                    warnings: self.warnings,
                    wanted_globals: self.wanted_globals,
                    dyn_exprs: self.dyn_exprs,
                };
                let inner = Env {
                    docs: env.docs,
                    names: env.names,
                    vars: env.vars,
                    funcs: &funcs,
                };
                let mut add = |value: String| {
                    let nodes = index.entry(value).or_default();
                    if nodes.last() != Some(&node) {
                        nodes.push(node);
                    }
                };
                match xpath::evaluate(&def.use_, &inner, &Focus::single(node))? {
                    Value::NodeSet(ns) => {
                        for n in ns {
                            add(inner.string_value(n));
                        }
                    }
                    v => add(xpath::to_string(&v, &inner)),
                }
            }
        }
        // Several definitions of one name can add nodes out of order.
        for index in indexes.values_mut() {
            for nodes in index.values_mut() {
                nodes.sort_unstable();
                nodes.dedup();
            }
        }
        Ok(indexes)
    }
}

impl XsltFunctions<'_> {
    /// xmlNodeGetBase for a node given to document(): an attribute's or
    /// PI's is its parent's. None for a namespace node, which libxslt
    /// doesn't count as a real node.
    fn node_base(&self, n: NodeRef, env: &Env) -> Option<Option<String>> {
        if n.ns > 0 {
            return None;
        }
        let doc = env.doc(n);
        let target = match doc.kind(n.node) {
            NodeKind::Attribute { .. } | NodeKind::Pi { .. } => {
                doc.parent(n.node).unwrap_or(n.node)
            }
            _ => n.node,
        };
        let xml_base = Some((NamePool::XML_NS, NamePool::BASE));
        Some(doc.base_uri(target, xml_base))
    }

    /// libxslt's xsltDocumentFunction.
    fn document(&self, args: &[Value], env: &Env) -> Result<Value, XPathError> {
        let second = match args.get(1) {
            Some(Value::NodeSet(ns)) => Some(ns.as_slice()),
            Some(_) => {
                return Err(XPathError::type_error(
                    "document() : invalid arg expecting a nodeset",
                ));
            }
            None => None,
        };
        // The base when a second argument gives one: its first node's.
        let given_base = second
            .and_then(|ns| ns.first())
            .and_then(|&n| self.node_base(n, env));
        let mut out = Vec::new();
        // A result tree fragment is a string to libxslt's document()
        // (XPATH_XSLT_TREE isn't XPATH_NODESET), so it's resolved against
        // the instruction, not the fragment. DocBook's man pages depend on
        // it: document($uri) with $uri built by xsl:with-param content.
        let fragment = |ns: &[NodeRef]| matches!(ns, [n] if n.ns == 0 && env.doc(*n).fragment && n.node == env.doc(*n).root());
        match &args[0] {
            Value::NodeSet(ns) if !fragment(ns) => {
                for &n in ns {
                    let base = match second {
                        Some(_) => given_base.clone(),
                        None => self.node_base(n, env),
                    };
                    let uri = env.string_value(n);
                    out.extend(self.document_uri(&uri, base, env)?);
                }
                out.sort_unstable();
                out.dedup();
            }
            v => {
                let uri = xpath::to_string(v, env);
                out = self.document_uri(&uri, given_base, env)?;
            }
        }
        Ok(Value::NodeSet(out))
    }

    /// One document() lookup. `base`: Some(base URI) from a node, or None
    /// for the instruction's.
    fn document_uri(
        &self,
        uri: &str,
        base: Option<Option<String>>,
        env: &Env,
    ) -> Result<Vec<NodeRef>, XPathError> {
        let base = match base {
            Some(b) => b,
            None => Some(self.base.to_string()),
        };
        let resolved = crate::uri::build(uri, base.as_deref());
        let (url, fragment) = match resolved.split_once('#') {
            Some((u, f)) => (u.to_string(), Some(f)),
            None => (resolved.clone(), None),
        };
        let found = self.loads.borrow().by_url.get(&url).copied();
        let doc = match found {
            Some(Some(d)) => d,
            Some(None) => return Ok(vec![]),
            None => {
                self.loads.borrow_mut().pending.push(url);
                return Err(XPathError::runtime("document() : loading"));
            }
        };
        let d = &env.docs[doc as usize];
        match fragment {
            None => Ok(vec![NodeRef::new(doc, d.root())]),
            // XPointer's bare-name form: the element with that ID.
            Some(f) => match d.element_by_id(f) {
                Some(e) => Ok(vec![NodeRef::new(doc, e)]),
                None => {
                    self.warnings
                        .borrow_mut()
                        .push(format!("document() : XPointer evaluation failed: #{f}"));
                    Ok(vec![])
                }
            },
        }
    }
}

impl XsltFunctions<'_> {
    /// A QName given as a string: its namespace URI and local name. A
    /// literal's prefix the compiler resolved ({uri}local); otherwise the
    /// instruction's namespaces resolve it, and `default` says whether an
    /// unprefixed name takes the default namespace. Err(prefix) when the
    /// prefix isn't bound.
    fn string_qname(
        &self,
        s: &str,
        env: &Env,
        default: bool,
    ) -> Result<(Option<String>, String), String> {
        let s = s.trim();
        if let Some((uri, local)) = s.strip_prefix('{').and_then(|r| r.split_once('}')) {
            return Ok((
                (!uri.is_empty()).then(|| uri.to_string()),
                local.to_string(),
            ));
        }
        let lookup = |prefix: Option<&str>| -> Option<String> {
            if prefix == Some("xml") {
                return Some(XML_NS.to_string());
            }
            self.namespaces
                .iter()
                .find(|(p, _)| p.map(|p| env.names.str(p)) == prefix)
                .map(|&(_, u)| env.names.str(u).to_string())
        };
        match s.split_once(':') {
            Some((prefix, local)) => match lookup(Some(prefix)) {
                Some(uri) => Ok((Some(uri), local.to_string())),
                None => Err(prefix.to_string()),
            },
            None if default => Ok((lookup(None).filter(|u| !u.is_empty()), s.to_string())),
            None => Ok((None, s.to_string())),
        }
    }

    /// The children of a fragment an extension function made, once it's
    /// among the documents; until then, a failure that has env_eval add it.
    fn fragment_nodes(&self, doc: Document, env: &Env) -> Result<Value, XPathError> {
        let mut made = self.made.borrow_mut();
        let key = (self.serial, made.calls);
        made.calls += 1;
        match made.by_key.get(&key) {
            Some(&d) => {
                let doc = &env.docs[d as usize];
                Ok(Value::NodeSet(
                    doc.children(doc.root())
                        .map(|c| NodeRef::new(d, c))
                        .collect(),
                ))
            }
            None => {
                made.pending.push((key, doc));
                Err(XPathError::runtime("extension function: making a fragment"))
            }
        }
    }
}

impl Functions for XsltFunctions<'_> {
    fn tick(&self, cost: u64) -> Result<(), XPathError> {
        self.budget.tick(cost).map_err(XPathError::runtime)
    }

    fn call(
        &self,
        name: Name,
        args: Vec<Value>,
        env: &Env,
        focus: &Focus,
    ) -> Option<Result<Value, XPathError>> {
        if let Some(ns) = name.0 {
            let local = env.names.str(name.1);
            if env.names.str(ns) == exslt::DYNAMIC && local == "evaluate" {
                return Some(self.dyn_evaluate(&args, env, focus));
            }
            if env.names.str(ns) == exslt::SAXON {
                match local {
                    // libexslt: the URL of the context's document.
                    "systemId" => {
                        let url = env.doc(focus.node).url.clone().unwrap_or_default();
                        return Some(Ok(Value::String(url)));
                    }
                    // The line of the context node or the argument's first
                    // node (an attribute's element's); -1 if unknown.
                    "line-number" => {
                        let node = match args.first() {
                            Some(Value::NodeSet(ns)) => ns.first().copied(),
                            Some(_) => return Some(Err(XPathError::type_error("Invalid type"))),
                            None => Some(focus.node),
                        };
                        let line = node.map_or(-1.0, |n| {
                            let doc = env.doc(n);
                            let id = match doc.kind(n.node) {
                                NodeKind::Attribute { .. } => doc.parent(n.node).unwrap_or(n.node),
                                _ => n.node,
                            };
                            match doc.line(id) {
                                0 => -1.0,
                                l => f64::from(l),
                            }
                        });
                        return Some(Ok(Value::Number(line)));
                    }
                    _ => {}
                }
            }
            let outcome = exslt::call(env.names.str(ns), local, &args, env, self.token)?;
            return Some(outcome.and_then(|o| match o {
                exslt::Outcome::Value(v) => Ok(v),
                exslt::Outcome::Fragment(doc) => self.fragment_nodes(*doc, env),
            }));
        }
        let arity = |n: usize, min: usize, max: usize| {
            if n < min || n > max {
                Err(XPathError::runtime("Invalid number of arguments"))
            } else {
                Ok(())
            }
        };
        let local = env.names.str(name.1);
        Some((|| match local {
            "current" => {
                arity(args.len(), 0, 0)?;
                Ok(Value::NodeSet(vec![self.current]))
            }
            "key" => {
                arity(args.len(), 2, 2)?;
                let qname = xpath::to_string(&args[0], env);
                // libxslt gives an empty node-set for an undeclared key.
                let Some(name) =
                    runtime_qname(&qname, env.names).filter(|n| self.ss.keys.contains_key(n))
                else {
                    return Ok(Value::NodeSet(vec![]));
                };
                let index = self.key_index(focus.node.doc, name, env)?;
                let mut out = Vec::new();
                let mut look = |value: &str| {
                    if let Some(nodes) = index.get(value) {
                        out.extend_from_slice(nodes);
                    }
                };
                match &args[1] {
                    Value::NodeSet(ns) => {
                        for &n in ns {
                            look(&env.string_value_cow(n));
                        }
                    }
                    v => look(&xpath::to_string(v, env)),
                }
                out.sort_unstable();
                out.dedup();
                Ok(Value::NodeSet(out))
            }
            "document" => {
                arity(args.len(), 1, 2)?;
                self.document(&args, env)
            }
            "generate-id" => {
                arity(args.len(), 0, 1)?;
                let node = match args.first() {
                    None => Some(focus.node),
                    Some(Value::NodeSet(n)) => n.first().copied(),
                    Some(_) => {
                        return Err(XPathError::type_error("generate-id() needs a node-set"));
                    }
                };
                Ok(Value::String(match node {
                    None => String::new(),
                    Some(n) => {
                        // libxslt numbers nodes in the order generate-id() first sees them.
                        let mut ids = self.ids.borrow_mut();
                        let next = ids.len() + 1;
                        format!("id{}", *ids.entry(n).or_insert(next))
                    }
                }))
            }
            "system-property" => {
                arity(args.len(), 1, 1)?;
                let prop = xpath::to_string(&args[0], env);
                Ok(match prop.rsplit(':').next().unwrap_or("") {
                    "version" if prop.contains(':') => Value::Number(1.0),
                    "vendor" if prop.contains(':') => Value::String("libxslt".into()),
                    "vendor-url" if prop.contains(':') => {
                        Value::String("http://xmlsoft.org/XSLT/".into())
                    }
                    _ => Value::String(String::new()),
                })
            }
            "function-available" => {
                arity(args.len(), 1, 1)?;
                let f = xpath::to_string(&args[0], env);
                // An unbound prefix: libxslt complains and looks the local
                // name up without a namespace.
                let (uri, local) = match self.string_qname(&f, env, false) {
                    Ok(q) => q,
                    Err(prefix) => {
                        self.warnings.borrow_mut().push(format!(
                            "function-available() : prefix {prefix} is not bound"
                        ));
                        (None, f.rsplit(':').next().unwrap_or("").to_string())
                    }
                };
                let available = match uri {
                    Some(uri) => exslt::is_available(&uri, &local),
                    None => {
                        crate::xpath::ast::CoreFn::from_name(&local).is_some()
                            || matches!(
                                local.as_str(),
                                "current"
                                    | "document"
                                    | "format-number"
                                    | "generate-id"
                                    | "key"
                                    | "system-property"
                                    | "function-available"
                                    | "element-available"
                                    | "unparsed-entity-uri"
                            )
                    }
                };
                Ok(Value::Boolean(available))
            }
            "element-available" => {
                arity(args.len(), 1, 1)?;
                let e = xpath::to_string(&args[0], env);
                let (uri, local) = self.string_qname(&e, env, true).map_err(|p| {
                    XPathError::runtime(format!("element-available() : prefix {p} is not bound"))
                })?;
                let local = local.as_str();
                // As libxslt answers: every instruction, the elements
                // that only appear inside instructions, and xsl:document.
                let supported = [
                    "apply-imports",
                    "apply-templates",
                    "attribute",
                    "call-template",
                    "choose",
                    "comment",
                    "copy",
                    "copy-of",
                    "document",
                    "element",
                    "fallback",
                    "for-each",
                    "if",
                    "message",
                    "number",
                    "otherwise",
                    "param",
                    "processing-instruction",
                    "sort",
                    "text",
                    "value-of",
                    "variable",
                    "when",
                    "with-param",
                ];
                // The extension elements libxslt and libexslt register,
                // whether or not oxslt implements them (as with
                // function-available). DocBook's chunker asks for
                // saxon:output first and names its chunks by the answer.
                let xslt = uri.as_deref() == Some(XSLT_NS);
                let extension = uri
                    .as_deref()
                    .is_some_and(|u| REGISTERED_EXTENSION_ELEMENTS.contains(&(u, local)));
                Ok(Value::Boolean(
                    (xslt && supported.contains(&local)) || extension,
                ))
            }
            "format-number" => {
                arity(args.len(), 2, 3)?;
                let number = xpath::to_number(&args[0], env);
                let pattern = xpath::to_string(&args[1], env);
                let format = match args.get(2) {
                    None => &self.ss.decimal_format,
                    Some(v) => {
                        let qname = xpath::to_string(v, env);
                        // The compiler turns a literal prefixed name into {uri}local.
                        let (uri, local) =
                            match qname.strip_prefix('{').and_then(|r| r.split_once('}')) {
                                Some((u, l)) => (Some(u), l),
                                None => (None, qname.as_str()),
                            };
                        let ns = uri.map(|u| env.names.get(u));
                        let found = match (env.names.get(local), ns) {
                            (Some(l), None) => self.ss.decimal_formats.get(&(None, l)),
                            (Some(l), Some(Some(u))) => self.ss.decimal_formats.get(&(Some(u), l)),
                            _ => None,
                        };
                        match found {
                            Some(f) => f,
                            None => {
                                return Err(XPathError::runtime(format!(
                                    "format-number() : undeclared decimal format '{qname}'"
                                )));
                            }
                        }
                    }
                };
                let result = format.format(&pattern, number);
                if let Some(w) = result.warning {
                    self.warnings.borrow_mut().push(w);
                }
                Ok(Value::String(result.text))
            }
            "unparsed-entity-uri" => {
                arity(args.len(), 1, 1)?;
                let name = xpath::to_string(&args[0], env);
                let doc = env.doc(focus.node);
                Ok(Value::String(
                    doc.unparsed_entity_uri(&name).unwrap_or("").to_string(),
                ))
            }
            _ => Err(XPathError::runtime(format!(
                "Unregistered function {local}"
            ))),
        })())
        .filter(|r| !matches!(r, Err(e) if e.message.starts_with("Unregistered")))
    }
}

// ---------- whitespace stripping ----------

/// Applies `xsl:strip-space` and `xsl:preserve-space` to the source: drops
/// whitespace-only text nodes whose parent matches a strip rule, unless an
/// `xml:space="preserve"` applies.
fn strip_space(doc: Document, ss: &Stylesheet, names: &mut NamePool) -> Document {
    if !ss.space.iter().any(|r| r.strip) {
        return doc;
    }
    let xml = names.intern(XML_NS);
    let space = NamePool::SPACE;
    let _ = XSLT_NS;
    let mut levels: Vec<usize> = ss.space.iter().map(|r| r.precedence).collect();
    levels.sort_unstable_by(|a, b| b.cmp(a));
    levels.dedup();
    let strips = |name: QName| -> bool {
        // libxslt's xsltFindElemSpaceHandling asks each module in turn,
        // highest precedence first: a name, then ns:*, then the module's *.
        // Names and ns:* tests sit in a hash table that never replaces an
        // entry, so the first declaration wins there; `*` is a flag, so the
        // latest wins.
        for &level in &levels {
            let rules = || ss.space.iter().filter(move |r| r.precedence == level);
            if let Some(r) =
                rules().find(|r| matches!(r.test, SpaceTest::Name(n) if n == name.expanded()))
            {
                return r.strip;
            }
            if let Some(r) =
                rules().find(|r| matches!(r.test, SpaceTest::Namespace(ns) if ns == name.ns))
            {
                return r.strip;
            }
            if let Some(r) = rules().rfind(|r| r.test == SpaceTest::Any) {
                return r.strip;
            }
        }
        false
    };
    let mut b = Builder::new();
    if doc.has_doctype {
        b.set_has_doctype();
    }
    for (name, uri) in doc.unparsed_entities() {
        b.add_unparsed_entity(name, uri);
    }
    // The rebuilt document keeps the input's IDs, on the new elements.
    let mut ids: HashMap<NodeId, Vec<&str>> = HashMap::new();
    for (value, el) in doc.ids() {
        ids.entry(el).or_default().push(value);
    }
    // (node, preserve) pairs; preserve tracks xml:space on the way down.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        doc: &Document,
        id: NodeId,
        preserve: bool,
        b: &mut Builder,
        strips: &dyn Fn(QName) -> bool,
        ids: &HashMap<NodeId, Vec<&str>>,
        xml: crate::tree::Sym,
        space: crate::tree::Sym,
    ) {
        for c in doc.children(id) {
            match doc.kind(c) {
                NodeKind::Element { name, .. } => {
                    let mut keep = preserve;
                    for a in doc.attributes(c) {
                        let q = doc.name(a).unwrap();
                        if q.ns == Some(xml) && q.local == space {
                            keep = doc.value(a) == Some("preserve");
                        }
                    }
                    let _ = b.start_element(name, doc.line(c));
                    for d in doc.ns_decls(c) {
                        b.add_ns_decl(*d);
                    }
                    for a in doc.attributes(c) {
                        let _ = b.add_attribute_maybe_valueless(
                            doc.name(a).unwrap(),
                            doc.value(a).unwrap_or(""),
                            doc.is_valueless(a),
                        );
                    }
                    for value in ids.get(&c).into_iter().flatten() {
                        b.add_id(value);
                    }
                    walk(doc, c, keep, b, strips, ids, xml, space);
                    b.end_element();
                }
                NodeKind::Text(_) => {
                    let text = doc.value(c).unwrap_or("");
                    let whitespace = text
                        .chars()
                        .all(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r'));
                    let parent_name = doc.name(id);
                    let strip = whitespace && !preserve && parent_name.is_some_and(strips);
                    if !strip {
                        // Adjacent text nodes (around a dropped entity
                        // reference) stay apart.
                        if doc
                            .prev_sibling(c)
                            .is_some_and(|p| matches!(doc.kind(p), NodeKind::Text(_)))
                        {
                            b.break_text();
                        }
                        let _ = b.text(text, doc.line(c));
                    }
                }
                NodeKind::Comment(_) => {
                    let _ = b.comment(doc.value(c).unwrap_or(""), doc.line(c));
                }
                NodeKind::Pi { target, .. } => {
                    let _ = b.pi(target, doc.value(c).unwrap_or(""), doc.line(c));
                }
                NodeKind::Root | NodeKind::Attribute { .. } => {}
            }
        }
    }
    walk(&doc, doc.root(), false, &mut b, &strips, &ids, xml, space);
    let mut stripped = b.finish();
    stripped.url = doc.url.clone();
    stripped
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which keys each pattern's alternatives are candidates for.
    #[test]
    fn rule_index_candidates() {
        let mut names = NamePool::new();
        let uri = names.intern("urn:x");
        let resolve = move |p: &str| (p == "x").then_some(uri);
        let a = (None, names.intern("a"));
        let b = (None, names.intern("b"));
        let xa = (Some(uri), names.intern("a"));
        let keys = [
            NodeKey::Element(a),
            NodeKey::Element(xa),
            NodeKey::Attribute(a),
            NodeKey::Attribute(b),
            NodeKey::Text,
            NodeKey::Comment,
            NodeKey::Other,
        ];
        // E a, E x:a, @a, @b, text, comment, other
        let cases: &[(&str, [bool; 7])] = &[
            ("a", [true, false, false, false, false, false, true]),
            ("x:a", [false, true, false, false, false, false, true]),
            ("x:*", [false, true, false, false, false, false, true]),
            ("*", [true, true, false, false, false, false, true]),
            ("node()", [true, true, false, false, true, true, true]),
            ("@a", [false, false, true, false, false, false, true]),
            ("@*", [false, false, true, true, false, false, true]),
            ("@node()", [false, false, true, true, false, false, true]),
            ("text()", [false, false, false, false, true, false, true]),
            ("comment()", [false, false, false, false, false, true, true]),
            (
                "processing-instruction()",
                [false, false, false, false, false, false, true],
            ),
            ("b/a", [true, false, false, false, false, false, true]),
            ("b//a[1]", [true, false, false, false, false, false, true]),
            ("/", [true; 7]),
            ("id('i')", [true; 7]),
            (
                "key('k', 'v')//a",
                [true, false, false, false, false, false, true],
            ),
        ];
        for (pattern, want) in cases {
            let expr = xpath::compile(pattern, &mut names, &resolve).unwrap();
            let p = super::super::pattern::from_expr(expr, false).unwrap();
            let alt = &p.alternatives[0];
            let got: Vec<bool> = keys.iter().map(|&k| could_match(alt, k)).collect();
            assert_eq!(got, want.to_vec(), "{pattern}");
        }
    }
}
