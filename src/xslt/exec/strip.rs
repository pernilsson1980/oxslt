//! Applies xsl:strip-space and xsl:preserve-space to source documents.

use super::*;

/// Applies `xsl:strip-space` and `xsl:preserve-space` to the source: drops
/// whitespace-only text nodes whose parent matches a strip rule, unless an
/// `xml:space="preserve"` applies.
pub(super) fn strip_space(doc: Document, ss: &Stylesheet, names: &mut NamePool) -> Document {
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
    stripped.html = doc.html;
    stripped
}
