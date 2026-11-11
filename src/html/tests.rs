use super::*;

/// The document as a compact string: elements as `name[attrs](children)`,
/// text quoted, CDATA as `C"…"`, comments as `#…`.
fn dump(src: &str) -> String {
    let mut names = NamePool::new();
    let doc = parse(src.as_bytes(), &mut names, "t.html").document;
    fn walk(doc: &Document, names: &NamePool, n: crate::tree::NodeId, out: &mut String) {
        for c in doc.children(n) {
            match doc.kind(c) {
                crate::tree::NodeKind::Element { name, .. } => {
                    out.push_str(names.str(name.local));
                    let attrs: Vec<String> = doc
                        .attributes(c)
                        .map(|a| {
                            let an = doc.name(a).unwrap();
                            format!("{}={}", names.str(an.local), doc.value(a).unwrap_or(""))
                        })
                        .collect();
                    if !attrs.is_empty() {
                        out.push_str(&format!("[{}]", attrs.join(",")));
                    }
                    out.push('(');
                    walk(doc, names, c, out);
                    out.push(')');
                }
                crate::tree::NodeKind::Text(_) => {
                    let k = if doc.text_kind(c) == TextKind::Cdata {
                        "C"
                    } else {
                        ""
                    };
                    out.push_str(&format!("{k}{:?}", doc.value(c).unwrap_or("")));
                }
                crate::tree::NodeKind::Comment(_) => {
                    out.push_str(&format!("#{}", doc.value(c).unwrap_or("")))
                }
                crate::tree::NodeKind::Pi { .. } => out.push_str("?pi"),
                _ => {}
            }
        }
    }
    let mut out = String::new();
    walk(&doc, &names, doc.root(), &mut out);
    out
}

#[test]
fn implied_elements() {
    assert_eq!(dump("hello"), r#"html(body(p("hello")))"#);
    assert_eq!(
        dump("<title>t</title><p>x"),
        r#"html(head(title("t"))body(p("x")))"#
    );
}

#[test]
fn auto_close() {
    assert_eq!(dump("<p>a<p>b"), r#"html(body(p("a")p("b")))"#);
    assert_eq!(
        dump("<ul><li>a<li>b</ul>"),
        r#"html(body(ul(li("a")li("b"))))"#
    );
}

#[test]
fn attributes_and_references() {
    assert_eq!(
        dump("<p CLASS=x checked title='a&amp;b&nbsp;' data=&foo;>"),
        "html(body(p[class=x,checked=checked,title=a&b\u{a0},data=&foo;]()))"
    );
}

#[test]
fn script_is_cdata() {
    assert_eq!(
        dump("<script>if (a<b) x();</script>"),
        r#"html(head(script(C"if (a<b) x();")))"#
    );
}

#[test]
fn element_prefix_dropped() {
    assert_eq!(dump("<p><svg:rect/>"), "html(body(p(rect())))");
}
