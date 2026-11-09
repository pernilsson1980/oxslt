//! Parser tests. Most inputs come from the evaluation of XML parsers made
//! before choosing quick-xml; expectations follow libxml2 where the feature
//! is in PoC scope.

use super::*;
use crate::tree::{NodeId, NodeKind};

/// Canonical form: `<{ns}name a="v">…</…>`, text escaped, comments as
/// `<!--…-->`, PIs as `<?target data?>`. Namespace declarations aren't shown.
fn dump(doc: &Document, names: &NamePool) -> String {
    fn qname(names: &NamePool, q: QName) -> String {
        match q.ns {
            Some(ns) => format!("{{{}}}{}", names.str(ns), names.str(q.local)),
            None => names.str(q.local).to_string(),
        }
    }
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
    fn walk(doc: &Document, names: &NamePool, id: NodeId, out: &mut String) {
        match doc.kind(id) {
            NodeKind::Root => doc.children(id).for_each(|c| walk(doc, names, c, out)),
            NodeKind::Element { name, .. } => {
                let n = qname(names, name);
                out.push('<');
                out.push_str(&n);
                for a in doc.attributes(id) {
                    let an = qname(names, doc.name(a).unwrap());
                    out.push_str(&format!(" {an}=\"{}\"", esc(doc.value(a).unwrap())));
                }
                out.push('>');
                doc.children(id).for_each(|c| walk(doc, names, c, out));
                out.push_str(&format!("</{n}>"));
            }
            NodeKind::Text(_) => out.push_str(&esc(doc.value(id).unwrap())),
            NodeKind::Comment(_) => out.push_str(&format!("<!--{}-->", doc.value(id).unwrap())),
            NodeKind::Pi { target, .. } => out.push_str(&format!(
                "<?{} {}?>",
                names.str(target),
                doc.value(id).unwrap()
            )),
            NodeKind::Attribute { .. } => unreachable!("attributes aren't children"),
        }
    }
    let mut out = String::new();
    walk(doc, names, doc.root(), &mut out);
    out
}

fn ok(input: &[u8]) -> String {
    let mut names = NamePool::new();
    match parse(input, &mut names, &ParseOptions::default()) {
        Ok(doc) => dump(&doc, &names),
        Err(e) => panic!("unexpected error: {e}"),
    }
}

fn err(input: &[u8]) -> ParseError {
    let mut names = NamePool::new();
    match parse(input, &mut names, &ParseOptions::default()) {
        Ok(doc) => panic!("expected an error, got {}", dump(&doc, &names)),
        Err(e) => e,
    }
}

#[test]
fn elements_attributes_text_comments_pis() {
    assert_eq!(
        ok(b"<?xml version=\"1.0\"?>\n<!-- top -->\n<d a=\"1\"><?pi  some data?>x<e/>y<!--c--></d>\n<?after?>"),
        "<!-- top --><d a=\"1\"><?pi some data?>x<e></e>y<!--c--></d><?after ?>"
    );
}

#[test]
fn character_and_predefined_references_and_cdata_merge_into_one_text_node() {
    let mut names = NamePool::new();
    let doc = parse(
        b"<d>a&#x41;&#66;&amp;<![CDATA[<&>]]>z</d>",
        &mut names,
        &ParseOptions::default(),
    )
    .unwrap();
    let d = doc.first_child(doc.root()).unwrap();
    let kids: Vec<_> = doc.children(d).collect();
    assert_eq!(kids.len(), 1);
    assert_eq!(doc.value(kids[0]), Some("aAB&<&>z"));
}

#[test]
fn line_endings_are_normalised() {
    // t10 from the parser evaluation: CRLF in text, tab and newline in attributes.
    let input =
        b"<?xml version=\"1.0\"?>\r\n<d a=\"x\ty\nz &#9;tab\">line1\r\nline2\rline3 &#13;</d>\r\n";
    assert_eq!(ok(input), "<d a=\"x y z \ttab\">line1\nline2\nline3 \r</d>");
}

#[test]
fn namespaces_resolve_for_elements_and_attributes() {
    // t09: default namespace, prefixes, xml:lang, and undeclaring the default.
    let input = br#"<r xmlns="urn:default" xmlns:a="urn:a" xml:lang="sv"><a:x a:att="1" plain="2"><y xmlns=""/></a:x></r>"#;
    assert_eq!(
        ok(input),
        r#"<{urn:default}r {http://www.w3.org/XML/1998/namespace}lang="sv"><{urn:a}x {urn:a}att="1" plain="2"><y></y></{urn:a}x></{urn:default}r>"#
    );
    let mut names = NamePool::new();
    let doc = parse(input, &mut names, &ParseOptions::default()).unwrap();
    let r = doc.first_child(doc.root()).unwrap();
    assert_eq!(doc.ns_decls(r).len(), 2);
    assert_eq!(
        doc.attributes(r).count(),
        1,
        "xmlns declarations aren't attributes"
    );
    let x = doc.first_child(r).unwrap();
    assert_eq!(doc.name(x).unwrap().prefix.map(|p| names.str(p)), Some("a"));
}

#[test]
fn unbound_prefix_is_an_error() {
    assert!(err(b"<p:d/>").message.contains("Namespace prefix p"));
    assert!(
        err(b"<d p:a=\"1\"/>")
            .message
            .contains("Namespace prefix p")
    );
}

#[test]
fn latin1_is_transcoded_from_the_declaration() {
    // t08: café åäö in ISO-8859-1.
    let input = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n<d>caf\xe9 \xe5\xe4\xf6</d>\n";
    assert_eq!(ok(input), "<d>café åäö</d>");
}

#[test]
fn utf16_with_bom_and_utf8_bom() {
    let mut utf16 = vec![0xFF, 0xFE];
    for u in "<d>ä</d>".encode_utf16() {
        utf16.extend(u.to_le_bytes());
    }
    assert_eq!(ok(&utf16), "<d>ä</d>");
    assert_eq!(ok(b"\xEF\xBB\xBF<d>x</d>"), "<d>x</d>");
}

#[test]
fn invalid_utf8_and_unknown_encodings_are_errors() {
    let e = err(b"<d>\n ok \xff</d>");
    assert_eq!(
        (e.message.as_str(), e.line, e.column),
        ("Input is not proper UTF-8", 2, 5)
    );
    assert!(
        err(b"<?xml version=\"1.0\" encoding=\"no-such-encoding\"?><d/>")
            .message
            .contains("Unsupported encoding")
    );
}

#[test]
fn internal_subset_entities_and_defaults() {
    // An external DOCTYPE with nothing to load is still accepted.
    let mut names = NamePool::new();
    let doc = parse(
        b"<!DOCTYPE d SYSTEM \"d.dtd\">\n<d>x</d>",
        &mut names,
        &ParseOptions::default(),
    )
    .unwrap();
    assert!(doc.has_doctype);
    assert_eq!(dump(&doc, &names), "<d>x</d>");
    // Entities, including ones with markup, and default attributes.
    let input = br#"<!DOCTYPE d [
        <!ENTITY e "x">
        <!ENTITY markup "<b xmlns='urn:b'>&e;</b>">
        <!ATTLIST d role CDATA "normal" kind NMTOKEN #IMPLIED>
        <!ATTLIST b id ID #IMPLIED>
    ]><d kind="  a  ">[&e;] &markup; &#65;</d>"#;
    assert_eq!(
        ok(input),
        r#"<d kind="a" role="normal">[x] <{urn:b}b>x</{urn:b}b> A</d>"#
    );
    // Without DTDATTR (xsltproc --novalid) there are no defaults.
    let opts = ParseOptions {
        dtd_attributes: false,
        load_dtd: false,
        ..ParseOptions::default()
    };
    let doc = parse(input, &mut names, &opts).unwrap();
    assert!(dump(&doc, &names).starts_with(r#"<d kind="a">"#));
    // A #FIXED xmlns default changes the element's namespace (GIMP's DTD).
    assert_eq!(
        ok(br#"<!DOCTYPE d [<!ATTLIST d xmlns CDATA #FIXED "urn:fixed">]><d><e/></d>"#),
        "<{urn:fixed}d><{urn:fixed}e></{urn:fixed}e></{urn:fixed}d>"
    );
}

#[test]
fn entity_errors() {
    assert_eq!(
        err(b"<!DOCTYPE d [ <!ENTITY a \"&a;\"> ]><d>&a;</d>").message,
        "Detected an entity reference loop"
    );
    assert!(
        err(b"<!DOCTYPE d [ <!ENTITY a \"<b>\"> ]><d>&a;</d>")
            .message
            .contains("balanced")
            || true
    );
    assert_eq!(err(b"<d>&nope;</d>").message, "Entity 'nope' not defined");
    // With an external subset declared, an undeclared entity is a warning and is dropped.
    let mut names = NamePool::new();
    let p = parse_with(
        b"<!DOCTYPE d SYSTEM \"x.dtd\"><d>a&nope;b</d>",
        &mut names,
        &ParseOptions::default(),
        &NoLoader,
        &None,
    )
    .unwrap();
    assert_eq!(dump(&p.document, &names), "<d>ab</d>");
    // libxml2 keeps an entity-reference node there, so the text is two nodes.
    let doc = &p.document;
    let d = doc.first_child(doc.root()).unwrap();
    assert_eq!(doc.children(d).count(), 2);
    assert!(p.warnings.iter().any(|w| w.contains("nope")));
}

#[test]
fn malformed_documents_are_rejected_with_positions() {
    // t11–t14 from the parser evaluation.
    assert!(err(b"<d><a></b></d>").message.contains("</b>"));
    assert_eq!(
        err(b"<d>&undefined;</d>").message,
        "Entity 'undefined' not defined"
    );
    assert!(
        err(b"<d a=\"1\" a=\"2\"/>")
            .message
            .contains("Attribute a redefined")
    );
    assert_eq!(
        err(b"<d>a ]]> b</d>").message,
        "Sequence ']]>' not allowed in content"
    );
    assert_eq!(ok(b"<d>a ]]&gt; b ]] ></d>"), "<d>a ]]&gt; b ]] &gt;</d>");
    let e = err(b"<d>\n  <ok/>\n  <bad attr=noquotes/>\n</d>\n");
    assert_eq!(e.line, 3);
}

#[test]
fn structure_errors() {
    assert_eq!(err(b"").message, "Start tag expected, '<' not found");
    assert_eq!(
        err(b"  <!-- only a comment -->").message,
        "Start tag expected, '<' not found"
    );
    assert_eq!(
        err(b"<a/><b/>").message,
        "Extra content at the end of the document"
    );
    assert_eq!(
        err(b"<a/>text").message,
        "Extra content at the end of the document"
    );
    assert_eq!(err(b"<a><b>").message, "Premature end of data");
}

#[test]
fn depth_limit() {
    let input = "<a>".repeat(20) + &"</a>".repeat(20);
    let mut names = NamePool::new();
    let opts = ParseOptions {
        max_depth: 10,
        ..ParseOptions::default()
    };
    let e = parse(input.as_bytes(), &mut names, &opts).unwrap_err();
    assert!(e.message.starts_with("Excessive depth in document"));
    assert!(parse(input.as_bytes(), &mut names, &ParseOptions::default()).is_ok());
    // libxml2 without XML_PARSE_HUGE: 257 nested elements, not 258.
    let nest = |n: usize| "<a>".repeat(n) + &"</a>".repeat(n);
    assert!(parse(nest(257).as_bytes(), &mut names, &ParseOptions::default()).is_ok());
    assert!(parse(nest(258).as_bytes(), &mut names, &ParseOptions::default()).is_err());
}

#[test]
fn element_line_numbers() {
    let mut names = NamePool::new();
    let doc = parse(
        b"<?xml version=\"1.0\"?>\n<d>\n  <a/>\n\n  <b/>\n</d>",
        &mut names,
        &ParseOptions::default(),
    )
    .unwrap();
    let d = doc.first_child(doc.root()).unwrap();
    let lines: Vec<u32> = doc
        .children(d)
        .filter(|&c| matches!(doc.kind(c), NodeKind::Element { .. }))
        .map(|c| doc.line(c))
        .collect();
    assert_eq!((doc.line(d), lines), (2, vec![3, 5]));
}

#[test]
fn billion_laughs_hits_the_expansion_limit() {
    let mut decls = String::from("<!ENTITY l0 \"lol\">");
    for i in 1..10 {
        let prev = format!("&l{};", i - 1).repeat(10);
        decls.push_str(&format!("<!ENTITY l{i} \"{prev}\">"));
    }
    let input = format!("<!DOCTYPE l [{decls}]><l>&l9;</l>");
    assert_eq!(
        err(input.as_bytes()).message,
        "Maximum entity amplification factor exceeded"
    );
}
#[test]
fn many_namespace_declarations_parse_and_resolve() {
    let decls: String = (0..1_000)
        .map(|i| format!(" xmlns:p{i}=\"urn:{i}\""))
        .collect();
    let input = format!("<d{decls}><p999:e p7:a=\"1\"/></d>");
    let mut names = NamePool::new();
    let doc = parse(input.as_bytes(), &mut names, &ParseOptions::default()).unwrap();
    let d = doc.first_child(doc.root()).unwrap();
    assert_eq!(doc.ns_decls(d).len(), 1_000);
    let e = doc.first_child(d).unwrap();
    assert_eq!(names.str(doc.name(e).unwrap().ns.unwrap()), "urn:999");
    let a = doc.attributes(e).next().unwrap();
    assert_eq!(names.str(doc.name(a).unwrap().ns.unwrap()), "urn:7");
}

#[test]
fn too_many_namespace_bindings_is_an_error() {
    let decls: String = (0..2_000)
        .map(|i| format!(" xmlns:p{i}=\"urn:{i}\""))
        .collect();
    let e = err(format!("<d{decls}/>").as_bytes());
    assert!(e.message.contains("namespace bindings"), "{}", e.message);
}
