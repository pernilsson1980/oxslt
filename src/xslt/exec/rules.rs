//! Template rule selection: the rule index by node kind and name (NodeKey,
//! could_match), pattern matching, and applying and calling rules.

use super::*;

/// A node's kind and name, as far as choosing template rules goes. Root,
/// namespace and processing-instruction nodes are `Other`, which every rule
/// is a candidate for.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(super) enum NodeKey {
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

pub(super) fn match_steps(
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

impl<'a> Transform<'a> {
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
    pub(super) fn find_template(
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

    pub(super) fn apply(
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
    pub(super) fn call_rule(
        &mut self,
        index: usize,
        focus: &Focus,
        params: &[(Name, Value)],
    ) -> R<()> {
        let saved = self.current_rule.replace(index);
        let result = self.call(index, focus, params);
        self.current_rule = saved;
        result
    }

    /// Whether `node` matches any alternative of `p`.
    pub(super) fn pattern_matches(&mut self, p: &Pattern, node: NodeRef) -> R<bool> {
        for alt in &p.alternatives {
            if self.matches(alt, node)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
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
            let p = crate::xslt::pattern::from_expr(expr, false).unwrap();
            let alt = &p.alternatives[0];
            let got: Vec<bool> = keys.iter().map(|&k| could_match(alt, k)).collect();
            assert_eq!(got, want.to_vec(), "{pattern}");
        }
    }
}
