//! Evaluation of compiled XPath expressions.
//!
//! Values use ordinary heap storage (`Vec`, `String`): the PoC defers bump
//! arenas until benchmarks show they're needed (decided 2026-11-08).
//!
//! Where libxml2 departs from the XPath 1.0 spec, this follows libxml2, since
//! xsltproc compatibility is the goal. Each such place is commented.

use super::ast::*;
use super::number;
use super::{NodeRef, Value, XPathError};
use crate::tree::{Document, NamePool, NodeId, NodeKind, Sym};

/// Variable values, supplied by the caller (XSLT).
pub trait Variables {
    fn get(&self, name: Name) -> Option<Value>;
}

/// Functions other than XPath's core library: XSLT's, and extensions.
/// Returns `None` if the function isn't known.
pub trait Functions {
    fn call(
        &self,
        name: Name,
        args: Vec<Value>,
        env: &Env,
        focus: &Focus,
    ) -> Option<Result<Value, XPathError>>;

    /// Charges `cost` units of work to the caller's budget, failing once
    /// it's spent. Evaluation calls this as it goes, so a limit on a
    /// transform's work covers its XPath too. No limit by default.
    fn tick(&self, _cost: u64) -> Result<(), XPathError> {
        Ok(())
    }
}

/// No variables and no extra functions.
pub struct Empty;

impl Variables for Empty {
    fn get(&self, _: Name) -> Option<Value> {
        None
    }
}

impl Functions for Empty {
    fn call(
        &self,
        _: Name,
        _: Vec<Value>,
        _: &Env,
        _: &Focus,
    ) -> Option<Result<Value, XPathError>> {
        None
    }
}

/// Everything an evaluation can see besides the focus.
pub struct Env<'a> {
    pub docs: &'a [Document],
    pub names: &'a NamePool,
    pub vars: &'a dyn Variables,
    pub funcs: &'a dyn Functions,
}

/// The context node, position and size.
#[derive(Copy, Clone, Debug)]
pub struct Focus {
    pub node: NodeRef,
    pub position: usize,
    pub size: usize,
}

impl Focus {
    pub fn single(node: NodeRef) -> Focus {
        Focus {
            node,
            position: 1,
            size: 1,
        }
    }
}

/// What kind of node a [`NodeRef`] is, resolved against its document.
enum Kind {
    Root,
    Element,
    Attribute,
    Text,
    Comment,
    Pi(Sym),
    /// A namespace node: prefix (None for the default namespace) and URI.
    Namespace(Option<Sym>, Sym),
}

impl Env<'_> {
    pub fn doc(&self, n: NodeRef) -> &Document {
        &self.docs[n.doc as usize]
    }

    fn at(&self, n: NodeRef, node: NodeId) -> NodeRef {
        NodeRef {
            doc: n.doc,
            node,
            ns: 0,
        }
    }

    fn kind(&self, n: NodeRef) -> Kind {
        if n.ns > 0 {
            let (prefix, uri) = self.namespaces(n)[n.ns as usize - 1];
            return Kind::Namespace(prefix, uri);
        }
        match self.doc(n).kind(n.node) {
            NodeKind::Root => Kind::Root,
            NodeKind::Element { .. } => Kind::Element,
            NodeKind::Attribute { .. } => Kind::Attribute,
            NodeKind::Text(_) => Kind::Text,
            NodeKind::Comment(_) => Kind::Comment,
            NodeKind::Pi { target, .. } => Kind::Pi(target),
        }
    }

    /// The in-scope namespaces of the element `n` refers to (its `node`), in
    /// libxml2's namespace-axis order: the `xml` namespace first, then the
    /// others in reverse of xmlGetNsList's order (the element's own
    /// declarations, then each ancestor's new ones). An undeclared default
    /// namespace (`xmlns=""`) has no node.
    pub fn namespaces(&self, n: NodeRef) -> Vec<(Option<Sym>, Sym)> {
        let doc = self.doc(n);
        let mut seen: Vec<Option<Sym>> = Vec::new();
        let mut list: Vec<(Option<Sym>, Sym)> = Vec::new();
        let xml_prefix = Some(crate::tree::NamePool::XML);
        let mut cur = Some(n.node);
        while let Some(e) = cur {
            for d in doc.ns_decls(e) {
                if seen.contains(&d.prefix) || (d.prefix.is_some() && d.prefix == xml_prefix) {
                    continue;
                }
                seen.push(d.prefix);
                if !(d.prefix.is_none() && self.names.str(d.uri).is_empty()) {
                    list.push((d.prefix, d.uri));
                }
            }
            cur = doc.parent(e);
        }
        let mut out = Vec::with_capacity(list.len() + 1);
        if let (Some(p), Some(uri)) = (xml_prefix, Some(crate::tree::NamePool::XML_NS)) {
            out.push((Some(p), uri));
        }
        out.extend(list.into_iter().rev());
        out
    }

    pub fn string_value(&self, n: NodeRef) -> String {
        self.string_value_cow(n).into_owned()
    }

    /// The string value without copying where the node holds it directly
    /// (attributes, text, comments, PIs, namespace nodes). Only the root and
    /// elements, whose value is a concatenation, allocate.
    pub fn string_value_cow(&self, n: NodeRef) -> std::borrow::Cow<'_, str> {
        use std::borrow::Cow;
        if let Kind::Namespace(_, uri) = self.kind(n) {
            return Cow::Borrowed(self.names.str(uri));
        }
        let doc = self.doc(n);
        match doc.value(n.node) {
            Some(v) => Cow::Borrowed(v),
            None => Cow::Owned(doc.string_value(n.node)),
        }
    }

    pub fn parent(&self, n: NodeRef) -> Option<NodeRef> {
        if n.ns > 0 {
            return Some(self.at(n, n.node));
        }
        self.doc(n).parent(n.node).map(|p| self.at(n, p))
    }

    /// The node's expanded name, for elements and attributes.
    fn expanded_name(&self, n: NodeRef) -> Option<Name> {
        if n.ns > 0 {
            return None;
        }
        self.doc(n).name(n.node).map(|q| q.expanded())
    }

    /// `local-name()`
    pub fn local_name(&self, n: NodeRef) -> String {
        match self.kind(n) {
            Kind::Element | Kind::Attribute => self
                .names
                .str(self.doc(n).name(n.node).unwrap().local)
                .to_string(),
            Kind::Pi(target) => self.names.str(target).to_string(),
            Kind::Namespace(prefix, _) => {
                prefix.map_or(String::new(), |p| self.names.str(p).to_string())
            }
            _ => String::new(),
        }
    }

    /// `name()`: the qualified name as written in the source.
    pub fn qualified_name(&self, n: NodeRef) -> String {
        match self.kind(n) {
            Kind::Element | Kind::Attribute => {
                let q = self.doc(n).name(n.node).unwrap();
                match q.prefix {
                    Some(p) => format!("{}:{}", self.names.str(p), self.names.str(q.local)),
                    None => self.names.str(q.local).to_string(),
                }
            }
            _ => self.local_name(n),
        }
    }

    /// `namespace-uri()`
    pub fn namespace_uri(&self, n: NodeRef) -> String {
        match self.kind(n) {
            Kind::Element | Kind::Attribute => self
                .doc(n)
                .name(n.node)
                .unwrap()
                .ns
                .map_or(String::new(), |u| self.names.str(u).to_string()),
            _ => String::new(),
        }
    }
}

type R<T> = Result<T, XPathError>;

pub fn evaluate(expr: &Expr, env: &Env, focus: &Focus) -> R<Value> {
    env.funcs.tick(1)?;
    Ok(match expr {
        Expr::Or(a, b) => {
            Value::Boolean(boolean(&evaluate(a, env, focus)?) || boolean(&evaluate(b, env, focus)?))
        }
        Expr::And(a, b) => {
            Value::Boolean(boolean(&evaluate(a, env, focus)?) && boolean(&evaluate(b, env, focus)?))
        }
        Expr::Compare(op, a, b) => {
            let (a, b) = (evaluate(a, env, focus)?, evaluate(b, env, focus)?);
            Value::Boolean(compare(*op, &a, &b, env))
        }
        Expr::Arith(op, a, b) => {
            let a = to_number(&evaluate(a, env, focus)?, env);
            let b = to_number(&evaluate(b, env, focus)?, env);
            Value::Number(match op {
                ArithOp::Add => a + b,
                ArithOp::Sub => a - b,
                ArithOp::Mul => a * b,
                ArithOp::Div => a / b,
                ArithOp::Mod => a % b, // C fmod: the result has the dividend's sign
            })
        }
        Expr::Negate(a) => Value::Number(-to_number(&evaluate(a, env, focus)?, env)),
        Expr::Union(a, b) => {
            let (Value::NodeSet(mut x), Value::NodeSet(y)) =
                (evaluate(a, env, focus)?, evaluate(b, env, focus)?)
            else {
                return Err(XPathError::type_error(
                    "Union of values that aren't node-sets",
                ));
            };
            x.extend(y);
            sort_dedup(&mut x);
            Value::NodeSet(x)
        }
        Expr::Literal(s) => Value::String(s.clone()),
        Expr::Number(n) => Value::Number(*n),
        Expr::Variable(name) => env.vars.get(*name).ok_or_else(|| {
            XPathError::runtime(format!("Undefined variable {}", env.names.str(name.1)))
        })?,
        Expr::Call(f, args) => call(f, args, env, focus)?,
        Expr::Filter(primary, predicates) => {
            let Value::NodeSet(nodes) = evaluate(primary, env, focus)? else {
                return Err(XPathError::type_error(
                    "Predicate on a value that isn't a node-set",
                ));
            };
            Value::NodeSet(apply_predicates(nodes, predicates, env)?)
        }
        Expr::Path(path) => Value::NodeSet(eval_path(path, env, focus)?),
    })
}

/// Evaluates an expression that must give a node-set.
pub fn evaluate_nodes(expr: &Expr, env: &Env, focus: &Focus) -> R<Vec<NodeRef>> {
    match evaluate(expr, env, focus)? {
        Value::NodeSet(n) => Ok(n),
        _ => Err(XPathError::type_error("Expression doesn't give a node-set")),
    }
}

pub fn sort_dedup(nodes: &mut Vec<NodeRef>) {
    nodes.sort_unstable();
    nodes.dedup();
}

fn eval_path(path: &Path, env: &Env, focus: &Focus) -> R<Vec<NodeRef>> {
    // The starting nodes, borrowed where there's only one, so the common
    // relative path doesn't allocate for them.
    let root;
    let started;
    let start: &[NodeRef] = match &path.start {
        PathStart::Root => {
            root = env.at(focus.node, env.doc(focus.node).root());
            std::slice::from_ref(&root)
        }
        PathStart::Context => std::slice::from_ref(&focus.node),
        PathStart::Expr(e) => {
            started = evaluate_nodes(e, env, focus)?;
            &started
        }
    };
    if path.steps.is_empty() {
        return Ok(start.to_vec());
    }
    let mut current: Vec<NodeRef> = Vec::new();
    // One scratch buffer for every context node's axis, so a step over many
    // nodes doesn't allocate once per node.
    let mut found = Vec::new();
    for (i, step) in path.steps.iter().enumerate() {
        let context: &[NodeRef] = if i == 0 { start } else { &current };
        let mut next = Vec::new();
        for &n in context {
            found.clear();
            axis_nodes(step.axis, n, env, &mut found);
            env.funcs.tick(1 + found.len() as u64)?;
            found.retain(|&m| matches_test(&step.test, step.axis, m, env));
            if step.predicates.is_empty() {
                if context.len() == 1 {
                    // The only context node: its nodes are the step's result.
                    next = std::mem::take(&mut found);
                } else {
                    next.extend_from_slice(&found);
                }
                continue;
            }
            // Predicates count positions in axis order (reverse axes from the
            // context node outwards); `found` is already in axis order.
            let mut selected = std::mem::take(&mut found);
            for pred in &step.predicates {
                selected = filter(selected, pred, env)?;
            }
            next.extend_from_slice(&selected);
            found = selected;
        }
        // Restore document order only where it's actually lost. From one
        // context node a forward axis is already in order and a reverse axis
        // exactly reversed; the self, attribute and namespace axes of a
        // sorted node-set stay sorted and free of duplicates.
        if context.len() <= 1 {
            if step.axis.is_reverse() {
                next.reverse();
            }
        } else if !matches!(
            step.axis,
            Axis::SelfAxis | Axis::Attribute | Axis::Namespace
        ) {
            sort_dedup(&mut next);
        }
        current = next;
    }
    Ok(current)
}

/// The nodes on `axis` from `n` that pass `test` and then each predicate in
/// turn, in axis order. XSLT patterns use this to rank a node among its
/// siblings, as `child::a[2]` does.
pub fn step_nodes(
    axis: Axis,
    test: &NodeTest,
    predicates: &[Expr],
    n: NodeRef,
    env: &Env,
) -> R<Vec<NodeRef>> {
    let mut found = Vec::new();
    axis_nodes(axis, n, env, &mut found);
    found.retain(|&m| matches_test(test, axis, m, env));
    for pred in predicates {
        found = filter(found, pred, env)?;
    }
    Ok(found)
}

/// Whether a predicate's result can't depend on the context position or
/// size: its value is never a number (a number would be compared with the
/// position) and it doesn't call position() or last() outside nested
/// predicates. Such a predicate can be evaluated on a node alone, without
/// ranking it among its siblings.
pub fn is_position_independent(e: &Expr) -> bool {
    never_number(e) && !uses_position(e)
}

fn never_number(e: &Expr) -> bool {
    use CoreFn::*;
    match e {
        Expr::Or(..)
        | Expr::And(..)
        | Expr::Compare(..)
        | Expr::Union(..)
        | Expr::Path(_)
        | Expr::Literal(_) => true,
        Expr::Filter(..) => true, // a node-set, or a type error
        Expr::Call(Function::Core(f), _) => matches!(
            f,
            Not | Boolean
                | True
                | False
                | Contains
                | StartsWith
                | Lang
                | String
                | Concat
                | Id
                | LocalName
                | NamespaceUri
                | Name
                | SubstringBefore
                | SubstringAfter
                | Substring
                | NormalizeSpace
                | Translate
        ),
        Expr::Number(_)
        | Expr::Arith(..)
        | Expr::Negate(_)
        | Expr::Variable(_)
        | Expr::Call(Function::Other(_), _) => false,
    }
}

/// Whether position() or last() is called in this expression's own context.
fn uses_position(e: &Expr) -> bool {
    match e {
        Expr::Call(Function::Core(CoreFn::Position | CoreFn::Last), _) => true,
        Expr::Call(_, args) => args.iter().any(uses_position),
        Expr::Or(a, b)
        | Expr::And(a, b)
        | Expr::Compare(_, a, b)
        | Expr::Arith(_, a, b)
        | Expr::Union(a, b) => uses_position(a) || uses_position(b),
        Expr::Negate(a) => uses_position(a),
        // A filter's or a path step's predicates have a context of their own.
        Expr::Filter(p, _) => uses_position(p),
        Expr::Path(p) => matches!(&p.start, PathStart::Expr(s) if uses_position(s)),
        Expr::Literal(_) | Expr::Number(_) | Expr::Variable(_) => false,
    }
}

/// Whether `n` passes `test` on `axis` (which sets the principal node type).
pub fn node_matches(test: &NodeTest, axis: Axis, n: NodeRef, env: &Env) -> bool {
    matches_test(test, axis, n, env)
}

/// Whether `n` is the root node of its document.
pub fn is_root(n: NodeRef, env: &Env) -> bool {
    n.ns == 0 && matches!(env.doc(n).kind(n.node), NodeKind::Root)
}

/// Whether `n` is an attribute node.
pub fn is_attribute(n: NodeRef, env: &Env) -> bool {
    n.ns == 0 && matches!(env.doc(n).kind(n.node), NodeKind::Attribute { .. })
}

fn apply_predicates(mut nodes: Vec<NodeRef>, predicates: &[Expr], env: &Env) -> R<Vec<NodeRef>> {
    for pred in predicates {
        nodes = filter(nodes, pred, env)?;
    }
    Ok(nodes)
}

fn filter(nodes: Vec<NodeRef>, pred: &Expr, env: &Env) -> R<Vec<NodeRef>> {
    let size = nodes.len();
    let mut out = Vec::new();
    for (i, n) in nodes.into_iter().enumerate() {
        let focus = Focus {
            node: n,
            position: i + 1,
            size,
        };
        let keep = match evaluate(pred, env, &focus)? {
            Value::Number(x) => x == (i + 1) as f64,
            v => boolean(&v),
        };
        if keep {
            out.push(n);
        }
    }
    Ok(out)
}

/// Appends the nodes on `axis` from `n`, in axis order.
fn axis_nodes(axis: Axis, n: NodeRef, env: &Env, out: &mut Vec<NodeRef>) {
    let doc = env.doc(n);
    let is_attr_or_ns = n.ns > 0 || matches!(doc.kind(n.node), NodeKind::Attribute { .. });
    let children = |id: NodeId| {
        doc.children(id).map(move |c| NodeRef {
            doc: n.doc,
            node: c,
            ns: 0,
        })
    };
    match axis {
        Axis::SelfAxis => out.push(n),
        Axis::Child => {
            if !is_attr_or_ns {
                out.extend(children(n.node));
            }
        }
        Axis::Attribute => {
            if !is_attr_or_ns {
                out.extend(doc.attributes(n.node).map(|a| env.at(n, a)));
            }
        }
        Axis::Namespace => {
            if !is_attr_or_ns && matches!(doc.kind(n.node), NodeKind::Element { .. }) {
                let count = env.namespaces(n).len() as u32;
                out.extend((1..=count).map(|i| NodeRef {
                    doc: n.doc,
                    node: n.node,
                    ns: i,
                }));
            }
        }
        Axis::Parent => out.extend(env.parent(n)),
        Axis::Ancestor | Axis::AncestorOrSelf => {
            if axis == Axis::AncestorOrSelf {
                out.push(n);
            }
            let mut cur = env.parent(n);
            while let Some(p) = cur {
                out.push(p);
                cur = env.parent(p);
            }
        }
        Axis::Descendant | Axis::DescendantOrSelf => {
            if axis == Axis::DescendantOrSelf {
                out.push(n);
            }
            if !is_attr_or_ns {
                descendants(n, env, out);
            }
        }
        Axis::FollowingSibling | Axis::PrecedingSibling => {
            if is_attr_or_ns {
                return;
            }
            let step = if axis == Axis::FollowingSibling {
                Document::next_sibling
            } else {
                Document::prev_sibling
            };
            let mut cur = step(doc, n.node);
            while let Some(s) = cur {
                out.push(env.at(n, s));
                cur = step(doc, s);
            }
        }
        Axis::Following => {
            // libxml2 (xmlXPathNextFollowing) starts from an attribute's or
            // namespace node's parent element and skips that element's
            // children; the spec would include them.
            let mut cur = if is_attr_or_ns {
                env.parent(n)
            } else {
                Some(n)
            };
            while let Some(c) = cur {
                let mut sib = doc.next_sibling(c.node);
                while let Some(s) = sib {
                    let s_ref = env.at(n, s);
                    out.push(s_ref);
                    descendants(s_ref, env, out);
                    sib = doc.next_sibling(s);
                }
                cur = env.parent(c);
            }
        }
        Axis::Preceding => {
            // Nodes before the context node, excluding ancestors, nearest first.
            let start = if is_attr_or_ns {
                env.parent(n)
            } else {
                Some(n)
            };
            let mut cur = start;
            while let Some(c) = cur {
                let mut sib = doc.prev_sibling(c.node);
                while let Some(s) = sib {
                    let s_ref = env.at(n, s);
                    let mut sub = vec![s_ref];
                    descendants(s_ref, env, &mut sub);
                    out.extend(sub.into_iter().rev());
                    sib = doc.prev_sibling(s);
                }
                cur = env.parent(c);
            }
        }
    }
}

/// Appends the descendants of `n` in document order, iteratively.
fn descendants(n: NodeRef, env: &Env, out: &mut Vec<NodeRef>) {
    let doc = env.doc(n);
    let mut stack: Vec<NodeId> = Vec::new();
    let push_children = |stack: &mut Vec<NodeId>, id: NodeId| {
        let start = stack.len();
        stack.extend(doc.children(id));
        stack[start..].reverse();
    };
    push_children(&mut stack, n.node);
    while let Some(id) = stack.pop() {
        out.push(env.at(n, id));
        push_children(&mut stack, id);
    }
}

fn matches_test(test: &NodeTest, axis: Axis, n: NodeRef, env: &Env) -> bool {
    let kind = env.kind(n);
    let principal = match axis {
        Axis::Attribute => matches!(kind, Kind::Attribute),
        Axis::Namespace => matches!(kind, Kind::Namespace(..)),
        _ => matches!(kind, Kind::Element),
    };
    match test {
        NodeTest::Node => true,
        NodeTest::Text => matches!(kind, Kind::Text),
        NodeTest::Comment => matches!(kind, Kind::Comment),
        NodeTest::Pi(target) => match kind {
            Kind::Pi(t) => target
                .as_deref()
                .is_none_or(|want| env.names.str(t) == want),
            _ => false,
        },
        NodeTest::Any => principal,
        NodeTest::Namespace(ns) => {
            principal && env.expanded_name(n).is_some_and(|(n_ns, _)| n_ns == *ns)
        }
        NodeTest::Name(name) => {
            if !principal {
                return false;
            }
            match kind {
                Kind::Namespace(prefix, _) => name.0.is_none() && prefix == Some(name.1),
                _ => env.expanded_name(n) == Some(*name),
            }
        }
    }
}

// ---------- conversions ----------

pub fn boolean(v: &Value) -> bool {
    match v {
        Value::NodeSet(n) => !n.is_empty(),
        Value::Tree(_) => true,
        Value::Boolean(b) => *b,
        Value::Number(x) => *x != 0.0 && !x.is_nan(),
        Value::String(s) => !s.is_empty(),
    }
}

pub fn to_number(v: &Value, env: &Env) -> f64 {
    match v {
        Value::Number(x) => *x,
        Value::Boolean(b) => f64::from(u8::from(*b)),
        Value::String(s) => number::from_string(s),
        Value::NodeSet(_) | Value::Tree(_) => number::from_string(&to_string(v, env)),
    }
}

/// [`to_string`], borrowing where the value already is a string or a node's
/// text.
pub fn to_str<'v>(v: &'v Value, env: &'v Env) -> std::borrow::Cow<'v, str> {
    use std::borrow::Cow;
    match v {
        Value::String(s) => Cow::Borrowed(s),
        Value::Number(x) => Cow::Owned(number::to_string(*x)),
        Value::Boolean(b) => Cow::Borrowed(if *b { "true" } else { "false" }),
        Value::NodeSet(n) => n
            .first()
            .map_or(Cow::Borrowed(""), |&first| env.string_value_cow(first)),
        Value::Tree(root) => env.string_value_cow(*root),
    }
}

pub fn to_string(v: &Value, env: &Env) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(x) => number::to_string(*x),
        Value::Boolean(b) => if *b { "true" } else { "false" }.to_string(),
        Value::NodeSet(n) => n
            .first()
            .map_or(String::new(), |&first| env.string_value(first)),
        Value::Tree(root) => env.string_value(*root),
    }
}

fn compare(op: CompareOp, a: &Value, b: &Value, env: &Env) -> bool {
    use Value::*;
    // libxml2's comparisons take a fragment as the node-set of its root.
    if matches!(a, Tree(_)) || matches!(b, Tree(_)) {
        let as_set = |v: &Value| match v {
            Tree(root) => NodeSet(vec![*root]),
            v => v.clone(),
        };
        return compare(op, &as_set(a), &as_set(b), env);
    }
    match (a, b) {
        (NodeSet(x), NodeSet(y)) => {
            if matches!(op, CompareOp::Eq | CompareOp::Ne) {
                // Hash one side's values, as libxml2's xmlXPathEqualNodeSets
                // does, instead of comparing every pair.
                if x.is_empty() || y.is_empty() {
                    return false;
                }
                if op == CompareOp::Eq {
                    // With a small side, a direct scan beats hashing the other
                    // (`@cat = preceding-sibling::r/@cat` compares one value
                    // with thousands).
                    let (small, big) = if x.len() <= y.len() { (x, y) } else { (y, x) };
                    if small.len() <= 8 {
                        let values: Vec<_> = small.iter().map(|&n| eq_key(env, n)).collect();
                        return big.iter().any(|&n| {
                            let v = eq_key(env, n);
                            values.contains(&v)
                        });
                    }
                    let set: std::collections::HashSet<(u32, std::borrow::Cow<'_, str>)> =
                        small.iter().map(|&n| eq_key(env, n)).collect();
                    return big.iter().any(|&n| set.contains(&eq_key(env, n)));
                }
                let ys: std::collections::HashSet<(u32, std::borrow::Cow<'_, str>)> =
                    y.iter().map(|&n| eq_key(env, n)).collect();
                // Some pair differs unless every value on both sides is one and the same.
                if ys.len() > 1 {
                    return true;
                }
                let only = ys.iter().next().expect("y is not empty");
                x.iter().any(|&n| eq_key(env, n) != *only)
            } else {
                let ys: Vec<f64> = y
                    .iter()
                    .map(|&n| number::from_string(&env.string_value(n)))
                    .collect();
                x.iter().any(|&n| {
                    let p = number::from_string(&env.string_value(n));
                    ys.iter().any(|&q| compare_numbers(op, p, q))
                })
            }
        }
        (NodeSet(x), other) => compare_nodes_with(op, x, other, env, false),
        (other, NodeSet(y)) => compare_nodes_with(op, y, other, env, true),
        _ => compare_atomic(op, a, b, env),
    }
}

/// xmlXPathStringHash: the first two bytes of a string.
fn str_hash(s: &str) -> u32 {
    let b = s.as_bytes();
    match b {
        [] => 0,
        [x] => u32::from(*x),
        [x, y, ..] => u32::from(*x) + (u32::from(*y) << 8),
    }
}

/// What libxml2's `=` and `!=` compare a node by: xmlXPathNodeValHash and
/// the string value. Two nodes are equal when both match. The hash is the
/// value's first two bytes, except for a document node, where libxml2
/// hashes its first element child (or first child) instead, and an HTML
/// document node, which hashes as 0. So a fragment like `<a>1</a><b>2</b>`
/// isn't equal to "12".
fn eq_key<'e>(env: &'e Env, n: NodeRef) -> (u32, std::borrow::Cow<'e, str>) {
    let value = env.string_value_cow(n);
    let doc = env.doc(n);
    let hash = if n.ns == 0 && matches!(doc.kind(n.node), NodeKind::Root) {
        if doc.html {
            0
        } else {
            let children: Vec<_> = doc.children(n.node).collect();
            let first = children
                .iter()
                .find(|&&c| matches!(doc.kind(c), NodeKind::Element { .. }))
                .or(children.first());
            match first {
                Some(&c) => match doc.kind(c) {
                    NodeKind::Element { .. }
                    | NodeKind::Text(_)
                    | NodeKind::Comment(_)
                    | NodeKind::Pi { .. } => str_hash(&doc.string_value(c)),
                    _ => 0,
                },
                None => 0,
            }
        }
    } else {
        str_hash(&value)
    };
    (hash, value)
}

/// A node-set against a non-node-set value. `swapped`: the node-set was on
/// the right of the operator.
fn compare_nodes_with(
    op: CompareOp,
    nodes: &[NodeRef],
    other: &Value,
    env: &Env,
    swapped: bool,
) -> bool {
    let ordered = |p: f64, q: f64| {
        if swapped {
            compare_numbers(op, q, p)
        } else {
            compare_numbers(op, p, q)
        }
    };
    match other {
        Value::Boolean(b) => {
            let set = !nodes.is_empty();
            let (l, r) = if swapped { (*b, set) } else { (set, *b) };
            compare_atomic(op, &Value::Boolean(l), &Value::Boolean(r), env)
        }
        Value::Number(q) => nodes
            .iter()
            .any(|&n| ordered(number::from_string(&env.string_value_cow(n)), *q)),
        Value::String(s) => match op {
            // xmlXPathEqualNodeSetString compares value hashes first.
            CompareOp::Eq => {
                let key = (str_hash(s), std::borrow::Cow::Borrowed(s.as_str()));
                nodes.iter().any(|&n| eq_key(env, n) == key)
            }
            CompareOp::Ne => {
                let key = (str_hash(s), std::borrow::Cow::Borrowed(s.as_str()));
                nodes.iter().any(|&n| eq_key(env, n) != key)
            }
            _ => {
                let q = number::from_string(s);
                nodes
                    .iter()
                    .any(|&n| ordered(number::from_string(&env.string_value(n)), q))
            }
        },
        Value::NodeSet(_) | Value::Tree(_) => unreachable!("handled by compare"),
    }
}

fn compare_atomic(op: CompareOp, a: &Value, b: &Value, env: &Env) -> bool {
    match op {
        CompareOp::Eq | CompareOp::Ne => {
            let equal = if matches!(a, Value::Boolean(_)) || matches!(b, Value::Boolean(_)) {
                boolean(a) == boolean(b)
            } else if matches!(a, Value::Number(_)) || matches!(b, Value::Number(_)) {
                to_number(a, env) == to_number(b, env)
            } else {
                to_string(a, env) == to_string(b, env)
            };
            equal == (op == CompareOp::Eq)
        }
        _ => compare_numbers(op, to_number(a, env), to_number(b, env)),
    }
}

fn compare_numbers(op: CompareOp, a: f64, b: f64) -> bool {
    match op {
        CompareOp::Eq => a == b,
        CompareOp::Ne => a != b,
        CompareOp::Lt => a < b,
        CompareOp::Le => a <= b,
        CompareOp::Gt => a > b,
        CompareOp::Ge => a >= b,
    }
}

// ---------- functions ----------

fn arity(f: CoreFn, n: usize, min: usize, max: usize) -> R<()> {
    if n < min || n > max {
        return Err(XPathError::runtime(format!(
            "Invalid number of arguments to {f:?}"
        )));
    }
    Ok(())
}

fn call(f: &Function, args: &[Expr], env: &Env, focus: &Focus) -> R<Value> {
    let mut values = Vec::with_capacity(args.len());
    for a in args {
        values.push(evaluate(a, env, focus)?);
    }
    match f {
        Function::Core(core) => core_function(*core, values, env, focus),
        Function::Other(name) => env
            .funcs
            .call(*name, values, env, focus)
            .unwrap_or_else(|| {
                let ns = name.0.map_or("", |n| env.names.str(n));
                Err(XPathError::runtime(format!(
                    "Unregistered function {{{ns}}}{}",
                    env.names.str(name.1)
                )))
            }),
    }
}

/// The first node of an optional node-set argument, defaulting to the
/// context node. An empty node-set gives `None`.
fn node_arg(f: CoreFn, args: &[Value], focus: &Focus) -> R<Option<NodeRef>> {
    match args.first() {
        None => Ok(Some(focus.node)),
        Some(v) => match v.nodes_or_tree() {
            Some(n) => Ok(n.first().copied()),
            None => Err(XPathError::type_error(format!("{f:?} needs a node-set"))),
        },
    }
}

fn string_arg(args: &[Value], env: &Env, focus: &Focus) -> String {
    match args.first() {
        Some(v) => to_string(v, env),
        None => env.string_value(focus.node),
    }
}

fn core_function(f: CoreFn, args: Vec<Value>, env: &Env, focus: &Focus) -> R<Value> {
    use CoreFn::*;
    let n = args.len();
    let s = |i: usize| to_str(&args[i], env);
    Ok(match f {
        Last => {
            arity(f, n, 0, 0)?;
            Value::Number(focus.size as f64)
        }
        Position => {
            arity(f, n, 0, 0)?;
            Value::Number(focus.position as f64)
        }
        Count => {
            arity(f, n, 1, 1)?;
            match args[0].nodes_or_tree() {
                Some(nodes) => Value::Number(nodes.len() as f64),
                None => return Err(XPathError::type_error("count() needs a node-set")),
            }
        }
        Id => {
            arity(f, n, 1, 1)?;
            let tokens: Vec<std::string::String> = match args[0].nodes_or_tree() {
                Some(nodes) => nodes
                    .iter()
                    .flat_map(|&n| {
                        env.string_value(n)
                            .split_ascii_whitespace()
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .collect(),
                None => to_string(&args[0], env)
                    .split_ascii_whitespace()
                    .map(str::to_owned)
                    .collect(),
            };
            Value::NodeSet(find_ids(&tokens, focus.node, env))
        }
        LocalName | NamespaceUri | Name => {
            arity(f, n, 0, 1)?;
            Value::String(match node_arg(f, &args, focus)? {
                None => std::string::String::new(),
                Some(node) => match f {
                    LocalName => env.local_name(node),
                    NamespaceUri => env.namespace_uri(node),
                    _ => env.qualified_name(node),
                },
            })
        }
        String => {
            arity(f, n, 0, 1)?;
            Value::String(string_arg(&args, env, focus))
        }
        Concat => {
            arity(f, n, 2, usize::MAX)?;
            let joined: std::string::String = (0..n).map(s).collect();
            // Doubling a string in a recursive template is cheap in calls
            // and expensive in bytes.
            env.funcs.tick(joined.len() as u64 / 64)?;
            Value::String(joined)
        }
        StartsWith => {
            arity(f, n, 2, 2)?;
            Value::Boolean(s(0).starts_with(&*s(1)))
        }
        Contains => {
            arity(f, n, 2, 2)?;
            Value::Boolean(s(0).contains(&*s(1)))
        }
        SubstringBefore => {
            arity(f, n, 2, 2)?;
            let (a, b) = (s(0), s(1));
            Value::String(
                a.find(&*b)
                    .map_or(std::string::String::new(), |i| a[..i].to_string()),
            )
        }
        SubstringAfter => {
            arity(f, n, 2, 2)?;
            let (a, b) = (s(0), s(1));
            Value::String(
                a.find(&*b)
                    .map_or(std::string::String::new(), |i| a[i + b.len()..].to_string()),
            )
        }
        Substring => {
            arity(f, n, 2, 3)?;
            let len = if n == 3 {
                Some(to_number(&args[2], env))
            } else {
                None
            };
            Value::String(substring(&s(0), to_number(&args[1], env), len))
        }
        StringLength => {
            arity(f, n, 0, 1)?;
            Value::Number(string_arg(&args, env, focus).chars().count() as f64)
        }
        NormalizeSpace => {
            arity(f, n, 0, 1)?;
            let text = string_arg(&args, env, focus);
            let words: Vec<&str> = text
                .split([' ', '\t', '\n', '\r'])
                .filter(|w| !w.is_empty())
                .collect();
            Value::String(words.join(" "))
        }
        Translate => {
            arity(f, n, 3, 3)?;
            let (from, to): (Vec<char>, Vec<char>) =
                (s(1).chars().collect(), s(2).chars().collect());
            let out = s(0)
                .chars()
                .filter_map(|c| match from.iter().position(|&x| x == c) {
                    Some(i) => to.get(i).copied(),
                    None => Some(c),
                })
                .collect();
            Value::String(out)
        }
        Boolean => {
            arity(f, n, 1, 1)?;
            Value::Boolean(boolean(&args[0]))
        }
        Not => {
            arity(f, n, 1, 1)?;
            Value::Boolean(!boolean(&args[0]))
        }
        True | False => {
            arity(f, n, 0, 0)?;
            Value::Boolean(f == True)
        }
        Lang => {
            arity(f, n, 1, 1)?;
            Value::Boolean(lang_matches(&s(0), focus.node, env))
        }
        Number => {
            arity(f, n, 0, 1)?;
            Value::Number(match args.first() {
                Some(v) => to_number(v, env),
                None => number::from_string(&env.string_value(focus.node)),
            })
        }
        Sum => {
            arity(f, n, 1, 1)?;
            match args[0].nodes_or_tree() {
                Some(nodes) => Value::Number(
                    nodes
                        .iter()
                        .map(|&n| number::from_string(&env.string_value(n)))
                        .sum(),
                ),
                None => return Err(XPathError::type_error("sum() needs a node-set")),
            }
        }
        Floor => {
            arity(f, n, 1, 1)?;
            Value::Number(to_number(&args[0], env).floor())
        }
        Ceiling => {
            arity(f, n, 1, 1)?;
            Value::Number(to_number(&args[0], env).ceil())
        }
        Round => {
            arity(f, n, 1, 1)?;
            Value::Number(round(to_number(&args[0], env)))
        }
    })
}

/// libxml2's `round()`: values in [-0.5, 0.5) keep their sign as zero.
pub fn round(x: f64) -> f64 {
    if (-0.5..0.5).contains(&x) {
        return x * 0.0;
    }
    let r = x.floor();
    if x - r >= 0.5 { r + 1.0 } else { r }
}

/// libxml2's `substring()` (xmlXPathSubstringFunction): positions are
/// rounded like `round()`, and NaN or infinite arguments are handled by the
/// same comparisons libxml2 makes.
// The negated comparisons are deliberate: `!(x < y)` is true for NaN, which
// is how libxml2 sends NaN down the same branch as a too-large value.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn substring(s: &str, start: f64, len: Option<f64>) -> String {
    const INT_MAX: f64 = i32::MAX as f64;
    let mut i: i64 = 1;
    let mut j: i64 = i32::MAX as i64;
    if !(start < INT_MAX) {
        i = i32::MAX as i64;
    } else if start >= 1.0 {
        i = start as i64;
        if start - start.floor() >= 0.5 {
            i += 1;
        }
    }
    if let Some(le) = len {
        let mut rin = start.floor();
        if start - rin >= 0.5 {
            rin += 1.0;
        }
        let mut rle = le.floor();
        if le - rle >= 0.5 {
            rle += 1.0;
        }
        let end = rin + rle;
        if !(end >= 1.0) {
            j = 1;
        } else if end < INT_MAX {
            j = end as i64;
        }
    }
    if i >= j {
        return String::new();
    }
    s.chars()
        .skip((i - 1) as usize)
        .take((j - i) as usize)
        .collect()
}

/// `lang()`: the nearest `xml:lang` on the context node or an ancestor
/// matches the argument, ignoring case, up to a `-` suffix.
fn lang_matches(want: &str, node: NodeRef, env: &Env) -> bool {
    let (xml, lang) = (crate::tree::NamePool::XML_NS, crate::tree::NamePool::LANG);
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n.ns == 0 {
            let doc = env.doc(n);
            for a in doc.attributes(n.node) {
                if doc.name(a).map(|q| q.expanded()) == Some((Some(xml), lang)) {
                    let value = doc.value(a).unwrap_or("");
                    let head = value.get(..want.len());
                    return head.is_some_and(|h| h.eq_ignore_ascii_case(want))
                        && matches!(value.as_bytes().get(want.len()), None | Some(b'-'));
                }
            }
        }
        cur = env.parent(n);
    }
    false
}

/// Elements whose ID is one of `tokens`, in document order: the ID values
/// the parser registered (DTD-declared ID attributes and xml:id).
fn find_ids(tokens: &[String], context: NodeRef, env: &Env) -> Vec<NodeRef> {
    let doc = env.doc(context);
    let mut out: Vec<NodeRef> = tokens
        .iter()
        .filter_map(|t| doc.element_by_id(t))
        .map(|n| env.at(context, n))
        .collect();
    sort_dedup(&mut out);
    out
}
