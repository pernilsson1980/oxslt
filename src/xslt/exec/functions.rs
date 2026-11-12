//! The XPath side of a transform: variable lookup, and the XSLT, EXSLT
//! and extension functions XPath calls (libxslt's functions.c, keys.c and
//! the libexslt modules), with key indexes, `document()` and
//! `dyn:evaluate`.

use super::*;

// ---------- variables and functions ----------

/// Global definitions by name, and the globals an evaluation asked for.
pub(super) type LazyGlobals<'a> = (&'a SymMap<Name, usize>, &'a RefCell<Vec<Name>>);

pub(super) struct Vars<'a> {
    pub(super) locals: &'a [(Name, Value)],
    pub(super) globals: &'a SymMap<Name, Value>,
    /// Globals not computed yet, and those an evaluation asked for.
    /// libxslt computes a global when it's first referenced.
    pub(super) lazy: Option<LazyGlobals<'a>>,
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
pub(super) type KeyIndex = HashMap<String, Vec<NodeRef>>;

/// Key indexes for each (document, key name), built for all keys of a
/// document at once.
#[derive(Default)]
pub(super) struct Keys {
    pub(super) built: HashMap<(u32, Name), Rc<KeyIndex>>,
    /// Documents whose keys are built.
    pub(super) done: std::collections::HashSet<u32>,
    /// Documents whose keys are being built, to catch a key that uses one.
    pub(super) building: Vec<u32>,
}

/// Documents loaded by `document()`, by URL (None: it couldn't be
/// loaded), and those it asked for that aren't loaded yet.
#[derive(Default)]
pub(super) struct DocLoads {
    pub(super) by_url: HashMap<String, Option<u32>>,
    pub(super) pending: Vec<String>,
}

/// Result tree fragments made by extension functions during evaluations,
/// keyed by (evaluation serial, call number within the evaluation).
#[derive(Default)]
pub(super) struct Made {
    pub(super) by_key: HashMap<(u64, u64), u32>,
    pub(super) pending: Vec<((u64, u64), Document)>,
    /// Fragment-making calls so far in this run of an evaluation.
    pub(super) calls: u64,
}

pub(super) struct XsltFunctions<'a> {
    /// XSLT's current node: the context node of the instruction.
    pub(super) current: NodeRef,
    pub(super) budget: &'a Budget,
    pub(super) made: &'a RefCell<Made>,
    pub(super) serial: u64,
    pub(super) token: crate::tree::Sym,
    pub(super) ids: &'a RefCell<SymMap<NodeRef, usize>>,
    pub(super) keys: &'a RefCell<Keys>,
    pub(super) loads: &'a RefCell<DocLoads>,
    /// The base URI of the instruction being run, for document().
    pub(super) base: &'a str,
    /// Its in-scope namespaces, for QNames given as strings.
    pub(super) namespaces: &'a [Binding],
    pub(super) ss: &'a Stylesheet,
    pub(super) warnings: &'a RefCell<Vec<String>>,
    pub(super) wanted_globals: &'a RefCell<Vec<Name>>,
    pub(super) dyn_exprs: &'a RefCell<DynExprs>,
}

/// dyn:evaluate's expressions by text and namespace context: compiled, or
/// the compile error; and those an evaluation asked for.
#[derive(Default)]
pub(super) struct DynExprs {
    pub(super) compiled: HashMap<(String, Vec<Binding>), Result<Rc<Expr>, String>>,
    pub(super) pending: Vec<(String, Vec<Binding>)>,
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
        match &args[0] {
            Value::NodeSet(ns) => {
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
                // xsltKeyFunction takes a fragment as a node-set too.
                match args[1].nodes_or_tree() {
                    Some(ns) => {
                        for &n in ns {
                            look(&env.string_value_cow(n));
                        }
                    }
                    None => look(&xpath::to_string(&args[1], env)),
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
