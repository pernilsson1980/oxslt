//! xsl:number's counting: which nodes count and at which levels (single,
//! multiple, any), as libxslt's numbers.c finds them. The formatting is in
//! xslt::number.

use super::*;

impl<'a> Transform<'a> {
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
    pub(super) fn count_numbers(
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
}
