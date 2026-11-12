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

mod functions;
use functions::*;
mod strip;
use strip::*;
mod write;
pub use write::{WriteCheck, Writes};
mod rules;
use rules::*;
mod copy;

/// The extension elements xsltproc registers, by (namespace, name).
mod limits;
mod numbering;
use limits::*;
pub use limits::{Limits, MAX_DEPTH};
const REGISTERED_EXTENSION_ELEMENTS: [(&str, &str); 6] = [
    ("http://exslt.org/common", "document"),
    ("http://www.jclark.com/xt", "document"),
    ("http://icl.com/saxon", "output"),
    ("org.apache.xalan.xslt.extensions.Redirect", "write"),
    ("http://xmlsoft.org/XSLT/namespace", "debug"),
    ("http://exslt.org/functions", "result"),
];

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
                Ok(Value::Tree(NodeRef::new(
                    id,
                    self.docs.last().unwrap().root(),
                )))
            }
        }
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

    // ---------- xsl:number ----------

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
                // A fragment is copied as its root: its content.
                v @ (Value::NodeSet(_) | Value::Tree(_)) => {
                    for &n in v.nodes_or_tree().unwrap_or_default() {
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
}
