//! The html output method, following libxml2 2.9.14's `HTMLtree.c`
//! (`htmlNodeDumpFormatOutput`, `htmlAttrDumpOutput`, `htmlSetMetaEncoding`)
//! and libxslt 1.1.35's choice of DOCTYPE. Comments name the libxml2 rule
//! each piece copies.

use crate::tree::{Builder, Document, NamePool, NodeId, NodeKind, NsDecl, QName};

/// libxml2 2.9.14's html40ElementTable (HTMLparser.c): name, empty,
/// saveEndTag, isinline. Sorted by name for binary search.
const ELEMENTS: &[(&str, bool, bool, bool)] = &[
    ("a", false, false, true),
    ("abbr", false, false, true),
    ("acronym", false, false, true),
    ("address", false, false, false),
    ("applet", false, false, true),
    ("area", true, true, false),
    ("b", false, false, true),
    ("base", true, true, false),
    ("basefont", true, true, true),
    ("bdo", false, false, true),
    ("big", false, false, true),
    ("blockquote", false, false, false),
    ("body", false, false, false),
    ("br", true, true, true),
    ("button", false, false, true),
    ("caption", false, false, false),
    ("center", false, false, false),
    ("cite", false, false, true),
    ("code", false, false, true),
    ("col", true, true, false),
    ("colgroup", false, false, false),
    ("dd", false, false, false),
    ("del", false, false, true),
    ("dfn", false, false, true),
    ("dir", false, false, false),
    ("div", false, false, false),
    ("dl", false, false, false),
    ("dt", false, false, false),
    ("em", false, false, true),
    ("embed", false, false, true),
    ("fieldset", false, false, false),
    ("font", false, false, true),
    ("form", false, false, false),
    ("frame", true, true, false),
    ("frameset", false, false, false),
    ("h1", false, false, false),
    ("h2", false, false, false),
    ("h3", false, false, false),
    ("h4", false, false, false),
    ("h5", false, false, false),
    ("h6", false, false, false),
    ("head", false, false, false),
    ("hr", true, true, false),
    ("html", false, false, false),
    ("i", false, false, true),
    ("iframe", false, false, true),
    ("img", true, true, true),
    ("input", true, true, true),
    ("ins", false, false, true),
    ("isindex", true, true, false),
    ("kbd", false, false, true),
    ("label", false, false, true),
    ("legend", false, false, false),
    ("li", false, true, false),
    ("link", true, true, false),
    ("map", false, false, true),
    ("menu", false, false, false),
    ("meta", true, true, false),
    ("noframes", false, false, false),
    ("noscript", false, false, false),
    ("object", false, false, true),
    ("ol", false, false, false),
    ("optgroup", false, false, false),
    ("option", false, false, false),
    ("p", false, false, false),
    ("param", true, true, false),
    ("pre", false, false, false),
    ("q", false, false, true),
    ("s", false, false, true),
    ("samp", false, false, true),
    ("script", false, false, true),
    ("select", false, false, true),
    ("small", false, false, true),
    ("span", false, false, true),
    ("strike", false, false, true),
    ("strong", false, false, true),
    ("style", false, false, false),
    ("sub", false, false, true),
    ("sup", false, false, true),
    ("table", false, false, false),
    ("tbody", false, false, false),
    ("td", false, false, false),
    ("textarea", false, false, true),
    ("tfoot", false, false, false),
    ("th", false, false, false),
    ("thead", false, false, false),
    ("title", false, false, false),
    ("tr", false, false, false),
    ("tt", false, false, true),
    ("u", false, false, true),
    ("ul", false, false, false),
    ("var", false, false, true),
];

#[derive(Copy, Clone)]
struct Info {
    /// A void element: written as `<br>`, children dropped.
    empty: bool,
    /// With no children, the end tag is left out (`<li>`).
    save_end_tag: bool,
    inline: bool,
}

/// `htmlTagLookup`: case-insensitive.
fn lookup(name: &str) -> Option<Info> {
    let lower = name.bytes().map(|b| b.to_ascii_lowercase());
    ELEMENTS
        .binary_search_by(|(n, ..)| n.bytes().cmp(lower.clone()))
        .ok()
        .map(|i| {
            let (_, empty, save_end_tag, inline) = ELEMENTS[i];
            Info {
                empty,
                save_end_tag,
                inline,
            }
        })
}

/// libxml2's htmlBooleanAttrs.
fn is_boolean(name: &str) -> bool {
    const BOOLEAN: &[&str] = &[
        "checked", "compact", "declare", "defer", "disabled", "ismap", "multiple", "nohref",
        "noresize", "noshade", "nowrap", "readonly", "selected",
    ];
    BOOLEAN.iter().any(|b| b.eq_ignore_ascii_case(name))
}

/// The DOCTYPE of an HTML result: name, public ID, system ID.
pub struct Doctype {
    pub name: String,
    pub public: Option<String>,
    pub system: Option<String>,
}

/// libxslt's xsltHTMLVersions: `xsl:output version` to (public, system).
pub fn html_version_ids(version: &str) -> (Option<&'static str>, Option<&'static str>) {
    const VERSIONS: &[(&str, Option<&str>, Option<&str>)] = &[
        ("5", None, Some("about:legacy-compat")),
        (
            "4.01frame",
            Some("-//W3C//DTD HTML 4.01 Frameset//EN"),
            Some("http://www.w3.org/TR/1999/REC-html401-19991224/frameset.dtd"),
        ),
        (
            "4.01strict",
            Some("-//W3C//DTD HTML 4.01//EN"),
            Some("http://www.w3.org/TR/1999/REC-html401-19991224/strict.dtd"),
        ),
        (
            "4.01trans",
            Some("-//W3C//DTD HTML 4.01 Transitional//EN"),
            Some("http://www.w3.org/TR/1999/REC-html401-19991224/loose.dtd"),
        ),
        (
            "4.01",
            Some("-//W3C//DTD HTML 4.01 Transitional//EN"),
            Some("http://www.w3.org/TR/1999/REC-html401-19991224/loose.dtd"),
        ),
        (
            "4.0strict",
            Some("-//W3C//DTD HTML 4.01//EN"),
            Some("http://www.w3.org/TR/html4/strict.dtd"),
        ),
        (
            "4.0trans",
            Some("-//W3C//DTD HTML 4.01 Transitional//EN"),
            Some("http://www.w3.org/TR/html4/loose.dtd"),
        ),
        (
            "4.0frame",
            Some("-//W3C//DTD HTML 4.01 Frameset//EN"),
            Some("http://www.w3.org/TR/html4/frameset.dtd"),
        ),
        (
            "4.0",
            Some("-//W3C//DTD HTML 4.01 Transitional//EN"),
            Some("http://www.w3.org/TR/html4/loose.dtd"),
        ),
        ("3.2", Some("-//W3C//DTD HTML 3.2//EN"), None),
    ];
    VERSIONS
        .iter()
        .find(|(v, ..)| v.eq_ignore_ascii_case(version))
        .map_or((None, None), |&(_, p, s)| (p, s))
}

/// libxml2's htmlNewDoc with no IDs: an HTML 4.0 Transitional DOCTYPE.
pub fn default_doctype() -> Doctype {
    Doctype {
        name: "html".into(),
        public: Some("-//W3C//DTD HTML 4.0 Transitional//EN".into()),
        system: Some("http://www.w3.org/TR/REC-html40/loose.dtd".into()),
    }
}

/// `htmlSetMetaEncoding`: makes sure `<head>` starts with a content-type
/// `<meta>` naming `encoding`. Returns a copy of the document with it added
/// or updated, or the document itself if there's no `<head>`.
pub fn set_meta_encoding(doc: Document, names: &mut NamePool, encoding: &str) -> Document {
    if encoding.eq_ignore_ascii_case("html") {
        return doc;
    }
    let content = format!("text/html; charset={encoding}");
    let is = |names: &NamePool, id: NodeId, want: &str| {
        matches!(doc.kind(id), NodeKind::Element { .. })
            && doc
                .name(id)
                .is_some_and(|q| names.str(q.local).eq_ignore_ascii_case(want))
    };

    // Find <head> (or a content-type <meta>) under the root, as libxml2 does.
    let mut head = None;
    let mut meta_parent = None;
    for c in doc.children(doc.root()) {
        if is(names, c, "html") {
            for h in doc.children(c) {
                if is(names, h, "head") {
                    head = Some(h);
                    break;
                }
                if is(names, h, "meta") {
                    // libxml2 treats the meta's parent as the head.
                    head = Some(c);
                    meta_parent = Some(c);
                    break;
                }
            }
            break;
        }
        if is(names, c, "head") {
            head = Some(c);
            break;
        }
        if is(names, c, "meta") {
            head = Some(doc.root());
            meta_parent = Some(doc.root());
            break;
        }
    }
    let scan = head.or(meta_parent);
    let Some(scan) = scan else { return doc };

    // An existing <meta http-equiv="Content-Type" content="…">.
    let mut existing = None;
    for m in doc.children(scan) {
        if !is(names, m, "meta") {
            continue;
        }
        let mut http = false;
        let mut value = None;
        for a in doc.attributes(m) {
            let n = names.str(doc.name(a).unwrap().local);
            let v = doc.value(a).unwrap_or("");
            if n.eq_ignore_ascii_case("http-equiv") && v.eq_ignore_ascii_case("Content-Type") {
                http = true;
            } else if n.eq_ignore_ascii_case("content") {
                value = Some((a, v.to_string()));
            }
        }
        if let (true, Some(v)) = (http, value) {
            existing = Some((m, v));
            break;
        }
    }
    let edit = match existing {
        // Change it only if the encoding really differs.
        Some((_, (_, v)))
            if v.to_ascii_lowercase()
                .contains(&encoding.to_ascii_lowercase()) =>
        {
            return doc;
        }
        Some((_, (attr, _))) => Edit::Replace(attr),
        None => match head {
            Some(h) => Edit::Insert(h),
            None => return doc,
        },
    };
    let meta = QName {
        ns: None,
        local: names.intern("meta"),
        prefix: None,
    };
    let http_equiv = QName {
        ns: None,
        local: names.intern("http-equiv"),
        prefix: None,
    };
    let content_name = QName {
        ns: None,
        local: names.intern("content"),
        prefix: None,
    };
    let mut b = Builder::new();
    copy_children(
        &doc,
        doc.root(),
        &mut b,
        &edit,
        &content,
        (meta, http_equiv, content_name),
    );
    b.finish()
}

enum Edit {
    /// Insert a new meta as the first child of this element.
    Insert(NodeId),
    /// Replace the value of this `content` attribute.
    Replace(NodeId),
}

fn copy_children(
    doc: &Document,
    id: NodeId,
    b: &mut Builder,
    edit: &Edit,
    content: &str,
    names: (QName, QName, QName),
) {
    // The new meta becomes the first child of the head (which may be <html>,
    // or the document itself).
    if let Edit::Insert(h) = edit
        && *h == id
    {
        let _ = b.start_element(names.0, 0);
        let _ = b.add_attribute(names.1, "Content-Type");
        let _ = b.add_attribute(names.2, content);
        b.end_element();
    }
    for c in doc.children(id) {
        match doc.kind(c) {
            NodeKind::Element { name, .. } => {
                let _ = b.start_element(name, 0);
                for d in doc.ns_decls(c) {
                    b.add_ns_decl(NsDecl {
                        prefix: d.prefix,
                        uri: d.uri,
                    });
                }
                for a in doc.attributes(c) {
                    let value = match edit {
                        Edit::Replace(target) if *target == a => content,
                        _ => doc.value(a).unwrap_or(""),
                    };
                    let _ = b.add_attribute(doc.name(a).unwrap(), value);
                }
                copy_children(doc, c, b, edit, content, names);
                b.end_element();
            }
            NodeKind::Text(_) => {
                // Keep the text's kind: unescaped and CDATA text must survive the copy.
                let _ = b.text_of_kind(doc.value(c).unwrap_or(""), doc.text_kind(c), 0);
            }
            NodeKind::Comment(_) => {
                let _ = b.comment(doc.value(c).unwrap_or(""), 0);
            }
            NodeKind::Pi { target, .. } => {
                let _ = b.pi(target, doc.value(c).unwrap_or(""), 0);
            }
            NodeKind::Root | NodeKind::Attribute { .. } => {}
        }
    }
}

/// Writes an HTML document. `format` is libxslt's indent, which defaults to
/// on for HTML.
pub fn write(
    doc: &Document,
    names: &NamePool,
    doctype: Option<&Doctype>,
    format: bool,
    s: &mut String,
) {
    // htmlDtdDumpOutput
    if let Some(d) = doctype {
        s.push_str("<!DOCTYPE ");
        s.push_str(&d.name);
        if let Some(p) = &d.public {
            s.push_str(" PUBLIC ");
            quoted(p, s);
            if let Some(sys) = &d.system {
                s.push(' ');
                quoted(sys, s);
            }
        } else if let Some(sys) = d.system.as_deref().filter(|s| *s != "about:legacy-compat") {
            s.push_str(" SYSTEM ");
            quoted(sys, s);
        }
        s.push_str(">\n");
    }
    let w = Writer { doc, names, format };
    for c in doc.children(doc.root()) {
        w.node(c, None, s);
    }
    // Returning to the document node writes a newline.
    s.push('\n');
}

struct Writer<'a> {
    doc: &'a Document,
    names: &'a NamePool,
    format: bool,
}

impl Writer<'_> {
    fn local(&self, q: QName) -> &str {
        self.names.str(q.local)
    }

    fn push_qname(&self, q: QName, s: &mut String) {
        super::push_qname(self.names, q, s);
    }

    fn info(&self, id: NodeId) -> Option<Info> {
        let q = self.doc.name(id)?;
        if q.ns.is_some() {
            None
        } else {
            lookup(self.local(q))
        }
    }

    fn is_text(&self, id: Option<NodeId>) -> bool {
        id.is_some_and(|i| matches!(self.doc.kind(i), NodeKind::Text(_)))
    }

    /// The newline libxml2 writes after an element when formatting: not
    /// before text, and not inside elements whose name starts with 'p'.
    fn newline_after(&self, id: NodeId, info: Option<Info>, parent: Option<&str>, s: &mut String) {
        let next = self.doc.next_sibling(id);
        if self.format
            && next.is_some()
            && info.is_some_and(|i| !i.inline)
            && !self.is_text(next)
            && parent.is_some_and(|p| !p.starts_with('p'))
        {
            s.push('\n');
        }
    }

    fn node(&self, id: NodeId, parent: Option<&str>, s: &mut String) {
        let doc = self.doc;
        match doc.kind(id) {
            NodeKind::Element { name, .. } => {
                let info = self.info(id);
                s.push('<');
                self.push_qname(name, s);
                for d in doc.ns_decls(id) {
                    match d.prefix {
                        Some(p) => s.push_str(&format!(" xmlns:{}=", self.names.str(p))),
                        None => s.push_str(" xmlns="),
                    }
                    quoted(self.names.str(d.uri), s);
                }
                for a in doc.attributes(id) {
                    self.attribute(a, name, s);
                }
                let first = doc.first_child(id);
                if info.is_some_and(|i| i.empty) {
                    s.push('>');
                } else if first.is_none() {
                    let local = self.local(name);
                    if info.is_some_and(|i| i.save_end_tag) && local != "html" && local != "body" {
                        s.push('>');
                    } else {
                        s.push_str("></");
                        self.push_qname(name, s);
                        s.push('>');
                    }
                } else {
                    let local = self.local(name).to_string();
                    let several = first != doc.last_child(id);
                    s.push('>');
                    if self.format
                        && info.is_some_and(|i| !i.inline)
                        && !self.is_text(first)
                        && several
                        && !local.starts_with('p')
                    {
                        s.push('\n');
                    }
                    for c in doc.children(id) {
                        self.node(c, Some(&local), s);
                    }
                    if self.format
                        && info.is_some_and(|i| !i.inline)
                        && !self.is_text(doc.last_child(id))
                        && several
                        && !local.starts_with('p')
                    {
                        s.push('\n');
                    }
                    s.push_str("</");
                    self.push_qname(name, s);
                    s.push('>');
                }
                self.newline_after(id, info, parent, s);
            }
            NodeKind::Text(_) => {
                let text = doc.value(id).unwrap_or("");
                let kind = doc.text_kind(id);
                // libxml2's HTML writer has no case for CDATA nodes: they vanish.
                if kind == crate::tree::TextKind::Cdata {
                    return;
                }
                let raw = kind == crate::tree::TextKind::Unescaped
                    || parent.is_some_and(|p| {
                        p.eq_ignore_ascii_case("script") || p.eq_ignore_ascii_case("style")
                    });
                if raw {
                    s.push_str(text);
                } else {
                    encode(text, false, s);
                }
            }
            NodeKind::Comment(_) => {
                let text = doc.value(id).unwrap_or("");
                if !text.is_empty() {
                    s.push_str("<!--");
                    s.push_str(text);
                    s.push_str("-->");
                }
            }
            NodeKind::Pi { target, .. } => {
                // HTML processing instructions end with '>', not '?>'.
                s.push_str("<?");
                s.push_str(self.names.str(target));
                s.push(' ');
                s.push_str(doc.value(id).unwrap_or(""));
                s.push('>');
            }
            NodeKind::Root | NodeKind::Attribute { .. } => {}
        }
    }

    fn attribute(&self, a: NodeId, element: QName, s: &mut String) {
        let q = self.doc.name(a).unwrap();
        s.push(' ');
        self.push_qname(q, s);
        let local = self.local(q);
        if is_boolean(local) {
            return;
        }
        let raw = self.doc.value(a).unwrap_or("");
        let mut value = String::with_capacity(raw.len() + 16);
        encode(raw, true, &mut value);
        s.push('=');
        let url = q.ns.is_none()
            && element.ns.is_none()
            && (["href", "action", "src"]
                .iter()
                .any(|n| n.eq_ignore_ascii_case(local))
                || (local.eq_ignore_ascii_case("name")
                    && self.local(element).eq_ignore_ascii_case("a")));
        if url {
            let trimmed = value.trim_start_matches([' ', '\t', '\n', '\r']);
            quoted(&uri_escape(trimmed, "@/:=?;#%&,+<>"), s);
        } else {
            quoted(&value, s);
        }
    }
}

/// `xmlEncodeEntitiesInternal` for an HTML document: escapes `<`, `>` and
/// `&`, writes other characters as they are. In attributes, `&{…}` and
/// `<!--…-->` are kept as written.
fn encode(t: &str, attr: bool, s: &mut String) {
    let mut i = 0;
    let b = t.as_bytes();
    while i < t.len() {
        let rest = &t[i..];
        let c = rest.chars().next().unwrap();
        if attr
            && rest.starts_with("<!--")
            && let Some(end) = rest.find("-->")
        {
            s.push_str(&rest[..end + 3]);
            i += end + 3;
            continue;
        }
        if attr
            && c == '&'
            && b.get(i + 1) == Some(&b'{')
            && let Some(end) = rest.find('}')
        {
            s.push_str(&rest[..=end]);
            i += end + 1;
            continue;
        }
        match c {
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '&' => s.push_str("&amp;"),
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\t' | '\r') => {
                s.push_str(&format!("&#{};", c as u32))
            }
            c => s.push(c),
        }
        i += c.len_utf8();
    }
}

/// `xmlURIEscapeStr`: percent-escapes every byte that isn't unreserved, `@`
/// or in `keep`.
fn uri_escape(t: &str, keep: &str) -> String {
    let mut out = String::with_capacity(t.len());
    for &byte in t.as_bytes() {
        let c = byte as char;
        let unreserved = c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c);
        if byte < 0x80 && (unreserved || c == '@' || keep.contains(c)) {
            out.push(c);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `xmlBufWriteQuotedString`: double quotes, or single quotes if the value
/// contains `"` but not `'`, or `&quot;` if it contains both.
pub fn quoted(v: &str, s: &mut String) {
    if v.contains('"') {
        if v.contains('\'') {
            s.push('"');
            s.push_str(&v.replace('"', "&quot;"));
            s.push('"');
        } else {
            s.push('\'');
            s.push_str(v);
            s.push('\'');
        }
    } else {
        s.push('"');
        s.push_str(v);
        s.push('"');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_table_is_sorted_and_complete() {
        assert!(ELEMENTS.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(ELEMENTS.len(), 92);
        assert!(lookup("BR").unwrap().empty);
        assert!(lookup("li").unwrap().save_end_tag && !lookup("li").unwrap().empty);
        assert!(lookup("span").unwrap().inline && !lookup("div").unwrap().inline);
        assert!(lookup("blink").is_none());
    }

    #[test]
    fn uri_escaping_and_quoting() {
        assert_eq!(
            uri_escape("a b.png?x=1&amp;y=2", "@/:=?;#%&,+<>"),
            "a%20b.png?x=1&amp;y=2"
        );
        assert_eq!(uri_escape("é", ""), "%C3%A9");
        let q = |v: &str| {
            let mut s = String::new();
            quoted(v, &mut s);
            s
        };
        assert_eq!(q("a"), "\"a\"");
        assert_eq!(q("say \"hi\""), "'say \"hi\"'");
        assert_eq!(q("it's \"x\""), "\"it's &quot;x&quot;\"");
    }
}
