//! Writes a result tree as bytes, as xsltproc does: libxslt 1.1.35's
//! `xsltSaveResultTo` and the choice of method and DOCTYPE in
//! `xsltApplyStylesheetInternal`, with libxml2 2.9.14's node writers.
//!
//! Characters an output encoding can't represent become numeric character
//! references. `indent="yes"` for xml, `cdata-section-elements` and
//! `disable-output-escaping` come in later steps.

pub mod html;

use crate::tree::{Document, NamePool, NodeId, NodeKind, QName, TextKind};
use crate::xslt::ir::{Method, Output};

/// Whether libxslt would treat the result as HTML without `method="html"`:
/// the first element is an `html` in no namespace (any letter case), with
/// no element or non-blank text before it.
fn implicit_html(doc: &Document, names: &NamePool) -> bool {
    for c in doc.children(doc.root()) {
        match doc.kind(c) {
            NodeKind::Element { name, .. } => {
                return name.ns.is_none() && names.str(name.local).eq_ignore_ascii_case("html");
            }
            NodeKind::Text(_)
                if !doc
                    .value(c)
                    .unwrap_or("")
                    .chars()
                    .all(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r')) =>
            {
                return false;
            }
            _ => {}
        }
    }
    false
}

fn root_qname(doc: &Document, names: &NamePool) -> Option<String> {
    doc.children(doc.root()).find_map(|c| match doc.kind(c) {
        NodeKind::Element { name, .. } => Some(qname(names, name)),
        _ => None,
    })
}

pub fn serialize(doc: Document, names: &mut NamePool, output: &Output) -> Vec<u8> {
    // An empty result writes nothing, not even the XML declaration.
    if doc.first_child(doc.root()).is_none() {
        return Vec::new();
    }
    let method = match output.method {
        Method::Unspecified if implicit_html(&doc, names) => Method::Html,
        Method::Unspecified => Method::Xml,
        m => m,
    };
    let mut s = String::new();
    match method {
        Method::Text => text_content(&doc, doc.root(), &mut s),
        Method::Html => {
            let doctype = html_doctype(&doc, names, output);
            let doc =
                html::set_meta_encoding(doc, names, output.encoding.as_deref().unwrap_or("UTF-8"));
            html::write(
                &doc,
                names,
                doctype.as_ref(),
                output.indent.unwrap_or(true),
                &mut s,
            );
        }
        Method::Xml | Method::Unspecified => xml_document(&doc, names, output, &mut s),
    }
    encode(&s, output.encoding.as_deref(), method)
}

/// The DOCTYPE libxslt gives an HTML result.
fn html_doctype(doc: &Document, names: &NamePool, output: &Output) -> Option<html::Doctype> {
    let (public, system) = (output.doctype_public.clone(), output.doctype_system.clone());
    let explicit = output.method == Method::Html;
    // An explicit html method names the DOCTYPE "html"; a default one uses
    // the root element's name as written.
    let name = if explicit {
        "html".to_string()
    } else {
        root_qname(doc, names).unwrap_or_else(|| "html".into())
    };
    if public.is_some() || system.is_some() {
        return Some(html::Doctype {
            name,
            public,
            system,
        });
    }
    let version = output.version.as_deref()?;
    let (p, s) = html::html_version_ids(version);
    match (p, s) {
        (None, None) if explicit => Some(html::default_doctype()),
        (None, None) => None,
        _ => Some(html::Doctype {
            name,
            public: p.map(str::to_string),
            system: s.map(str::to_string),
        }),
    }
}

/// Transcodes the output. Characters the encoding can't represent become
/// `&#N;` references, as libxml2's output buffer writes them for every
/// method. An unknown encoding name leaves the output in UTF-8.
fn encode(s: &str, encoding: Option<&str>, _method: Method) -> Vec<u8> {
    match encoding.and_then(crate::encoding::lookup) {
        Some(charset) => charset.encode(s, true),
        None => s.as_bytes().to_vec(),
    }
}

fn text_content(doc: &Document, id: NodeId, out: &mut String) {
    for c in doc.children(id) {
        match doc.kind(c) {
            NodeKind::Text(_) => out.push_str(doc.value(c).unwrap_or("")),
            NodeKind::Element { .. } => text_content(doc, c, out),
            _ => {}
        }
    }
}

/// Appends `q` as written, prefix and all.
pub(crate) fn push_qname(names: &NamePool, q: QName, s: &mut String) {
    if let Some(p) = q.prefix {
        s.push_str(names.str(p));
        s.push(':');
    }
    s.push_str(names.str(q.local));
}

fn qname(names: &NamePool, q: QName) -> String {
    match q.prefix {
        Some(p) => format!("{}:{}", names.str(p), names.str(q.local)),
        None => names.str(q.local).to_string(),
    }
}

fn xml_document(doc: &Document, names: &NamePool, output: &Output, s: &mut String) {
    // libxslt's indent has three states; only an explicit "no" turns off the
    // newlines after a DOCTYPE, after a top-level comment and at the end.
    let newlines = output.indent != Some(false);
    let hex_non_ascii = output.encoding.is_none();
    if !output.omit_xml_declaration.unwrap_or(false) {
        s.push_str("<?xml version=\"");
        s.push_str(output.version.as_deref().unwrap_or("1.0"));
        s.push('"');
        if let Some(e) = &output.encoding {
            s.push_str(&format!(" encoding=\"{e}\""));
        }
        if let Some(sa) = output.standalone {
            s.push_str(if sa {
                " standalone=\"yes\""
            } else {
                " standalone=\"no\""
            });
        }
        s.push_str("?>\n");
    }
    // xmlDtdDumpOutput: a public ID is always followed by a quoted system
    // ID, which is "" when there is none.
    if (output.doctype_public.is_some() || output.doctype_system.is_some())
        && let Some(root) = root_qname(doc, names)
    {
        s.push_str("<!DOCTYPE ");
        s.push_str(&root);
        match (&output.doctype_public, &output.doctype_system) {
            (Some(p), sys) => {
                s.push_str(" PUBLIC ");
                html::quoted(p, s);
                s.push(' ');
                html::quoted(sys.as_deref().unwrap_or(""), s);
            }
            (None, Some(sys)) => {
                s.push_str(" SYSTEM ");
                html::quoted(sys, s);
            }
            (None, None) => {}
        }
        s.push('>');
        if newlines {
            s.push('\n');
        }
    }
    // libxml2 writes a document whose DOCTYPE names XHTML 1.0 with its
    // XHTML serializer (xmlIsXHTML, xhtmlNodeDumpOutput). libxslt makes the
    // DOCTYPE only when there's a root element, as above.
    let xhtml = root_qname(doc, names).is_some() && is_xhtml(output);
    let charset = output.encoding.as_deref().unwrap_or("UTF-8");
    let children: Vec<NodeId> = doc.children(doc.root()).collect();
    for (i, &c) in children.iter().enumerate() {
        let f = Fmt {
            hex_non_ascii,
            format: output.indent == Some(true),
            level: 0,
            root: true,
            xhtml: xhtml.then_some(charset),
        };
        node(doc, names, c, f, s);
        if newlines && matches!(doc.kind(c), NodeKind::Comment(_)) && i + 1 < children.len() {
            s.push('\n');
        }
    }
    if newlines {
        s.push('\n');
    }
}

/// How xmlNodeDumpOutput writes one node. `format` is `indent="yes"`;
/// `level` is the depth below the subtree root, which gets no indentation.
/// `xhtml` is the charset for the XHTML serializer's Content-Type meta, when
/// writing XHTML.
#[derive(Copy, Clone)]
struct Fmt<'a> {
    hex_non_ascii: bool,
    format: bool,
    level: usize,
    root: bool,
    xhtml: Option<&'a str>,
}

/// xmlIsXHTML: the DOCTYPE's public or system ID is XHTML 1.0's.
fn is_xhtml(output: &Output) -> bool {
    const PUBLIC: [&str; 3] = [
        "-//W3C//DTD XHTML 1.0 Strict//EN",
        "-//W3C//DTD XHTML 1.0 Frameset//EN",
        "-//W3C//DTD XHTML 1.0 Transitional//EN",
    ];
    const SYSTEM: [&str; 3] = [
        "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd",
        "http://www.w3.org/TR/xhtml1/DTD/xhtml1-frameset.dtd",
        "http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd",
    ];
    output
        .doctype_public
        .as_deref()
        .is_some_and(|p| PUBLIC.contains(&p))
        || output
            .doctype_system
            .as_deref()
            .is_some_and(|s| SYSTEM.contains(&s))
}

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";

/// xhtmlIsEmpty: an XHTML element written as `<br />` when it has no
/// children.
fn xhtml_is_empty(doc: &Document, names: &NamePool, id: NodeId, name: QName) -> bool {
    if name.ns.is_some_and(|ns| names.str(ns) != XHTML_NS) || doc.first_child(id).is_some() {
        return false;
    }
    matches!(
        names.str(name.local),
        "area"
            | "br"
            | "base"
            | "basefont"
            | "col"
            | "frame"
            | "hr"
            | "img"
            | "input"
            | "isindex"
            | "link"
            | "meta"
            | "param"
    )
}

/// The XHTML serializer's Content-Type check: a `head` under the root
/// `html` without a `meta http-equiv="Content-Type"` gets one.
fn xhtml_needs_meta(doc: &Document, names: &NamePool, id: NodeId, name: QName) -> bool {
    let Some(parent) = doc.parent(id) else {
        return false;
    };
    let parent_is_root_html = doc.parent(parent) == Some(doc.root())
        && doc
            .name(parent)
            .is_some_and(|q| names.str(q.local) == "html");
    if names.str(name.local) != "head" || !parent_is_root_html {
        return false;
    }
    !doc.children(id).any(|c| {
        doc.name(c).is_some_and(|q| names.str(q.local) == "meta")
            && doc.attributes(c).any(|a| {
                names.str(doc.name(a).unwrap().local) == "http-equiv"
                    && doc
                        .value(a)
                        .unwrap_or("")
                        .eq_ignore_ascii_case("Content-Type")
            })
    })
}

/// xhtmlAttrListDumpOutput: attributes as in XML, but an empty boolean
/// attribute takes its name as value, and `id` (from `name`, on some
/// elements) and `lang`/`xml:lang` are added when only one of a pair is
/// there.
fn xhtml_attributes(doc: &Document, names: &NamePool, id: NodeId, f: Fmt, s: &mut String) {
    let mut name_attr = None;
    let mut has_id = false;
    let mut lang = None;
    let mut xml_lang = None;
    for a in doc.attributes(id) {
        let q = doc.name(a).unwrap();
        let local = names.str(q.local);
        let mut value = doc.value(a).unwrap_or("");
        if q.ns.is_none() && local == "id" {
            has_id = true;
        } else if q.ns.is_none() && local == "name" {
            name_attr = Some(value);
        } else if q.ns.is_none() && local == "lang" {
            lang = Some(value);
        } else if q.ns.is_some()
            && local == "lang"
            && q.prefix.is_some_and(|p| names.str(p) == "xml")
        {
            xml_lang = Some(value);
        } else if q.ns.is_none() && value.is_empty() && html::is_boolean(local) {
            value = local;
        }
        s.push(' ');
        push_qname(names, q, s);
        s.push_str("=\"");
        escape_attr(value, f.hex_non_ascii, s);
        s.push('"');
    }
    let element = names.str(doc.name(id).unwrap().local);
    if let Some(v) = name_attr
        && !has_id
        && matches!(
            element,
            "a" | "p" | "div" | "img" | "map" | "applet" | "form" | "frame" | "iframe"
        )
    {
        s.push_str(" id=\"");
        escape_attr(v, f.hex_non_ascii, s);
        s.push('"');
    }
    match (lang, xml_lang) {
        (Some(v), None) => {
            s.push_str(" xml:lang=\"");
            escape_attr(v, f.hex_non_ascii, s);
            s.push('"');
        }
        (None, Some(v)) => {
            s.push_str(" lang=\"");
            escape_attr(v, f.hex_non_ascii, s);
            s.push('"');
        }
        _ => {}
    }
}

fn push_xhtml_meta(charset: &str, f: Fmt, s: &mut String) {
    if f.format {
        s.push('\n');
        s.extend(std::iter::repeat_n(' ', 2 * (f.level + 1).min(30)));
    }
    s.push_str("<meta http-equiv=\"Content-Type\" content=\"text/html; charset=");
    s.push_str(charset);
    s.push_str("\" />");
}

impl Fmt<'_> {
    /// libxml2 indents two spaces per level, up to 60.
    fn indent(&self, s: &mut String) {
        if self.format && !self.root {
            s.extend(std::iter::repeat_n(' ', 2 * self.level.min(30)));
        }
    }
}

fn node(doc: &Document, names: &NamePool, id: NodeId, f: Fmt, s: &mut String) {
    let hex_non_ascii = f.hex_non_ascii;
    match doc.kind(id) {
        NodeKind::Element { name, .. } => {
            f.indent(s);
            s.push('<');
            push_qname(names, name, s);
            // xmlNsListDumpOutput writes the URI with xmlBufWriteQuotedString.
            for d in doc.ns_decls(id) {
                match d.prefix {
                    Some(p) => s.push_str(&format!(" xmlns:{}=", names.str(p))),
                    None => s.push_str(" xmlns="),
                }
                html::quoted(names.str(d.uri), s);
            }
            // XHTML: an unprefixed, undeclared html element gets the XHTML
            // namespace (XHTML 1.0, A.3.1.1).
            if f.xhtml.is_some()
                && name.ns.is_none()
                && names.str(name.local) == "html"
                && doc.ns_decls(id).is_empty()
            {
                s.push_str(" xmlns=\"http://www.w3.org/1999/xhtml\"");
            }
            if f.xhtml.is_some() {
                xhtml_attributes(doc, names, id, f, s);
            } else {
                for a in doc.attributes(id) {
                    s.push(' ');
                    push_qname(names, doc.name(a).unwrap(), s);
                    s.push_str("=\"");
                    escape_attr(doc.value(a).unwrap_or(""), hex_non_ascii, s);
                    s.push('"');
                }
            }
            let meta = f.xhtml.filter(|_| xhtml_needs_meta(doc, names, id, name));
            if doc.first_child(id).is_none() {
                match f.xhtml {
                    None => s.push_str("/>"),
                    // C.2: `<br />`; C.3: other empty elements in full.
                    Some(_)
                        if name.prefix.is_none()
                            && meta.is_none()
                            && xhtml_is_empty(doc, names, id, name) =>
                    {
                        s.push_str(" />")
                    }
                    Some(_) => {
                        s.push('>');
                        if let Some(charset) = meta {
                            push_xhtml_meta(charset, f, s);
                            if f.format {
                                s.push('\n');
                            }
                        }
                        s.push_str("</");
                        push_qname(names, name, s);
                        s.push('>');
                    }
                }
                return;
            }
            // Any text child turns formatting off for this element's subtree,
            // so mixed content is kept as it is.
            let format = f.format
                && !doc
                    .children(id)
                    .any(|c| matches!(doc.kind(c), NodeKind::Text(_)));
            s.push('>');
            if let Some(charset) = meta {
                push_xhtml_meta(charset, f, s);
            }
            if format {
                s.push('\n');
            }
            let child = Fmt {
                format,
                level: f.level + 1,
                root: false,
                ..f
            };
            for c in doc.children(id) {
                node(doc, names, c, child, s);
                if format {
                    s.push('\n');
                }
            }
            if format {
                Fmt {
                    level: f.level,
                    ..child
                }
                .indent(s);
            }
            s.push_str("</");
            push_qname(names, name, s);
            s.push('>');
        }
        NodeKind::Text(_) => {
            let text = doc.value(id).unwrap_or("");
            match doc.text_kind(id) {
                TextKind::Normal => escape_text(text, s),
                TextKind::Unescaped => s.push_str(text),
                TextKind::Cdata => cdata(text, s),
            }
        }
        NodeKind::Comment(_) => {
            // libxslt makes an empty comment with no content, which libxml2
            // doesn't write at all.
            let text = doc.value(id).unwrap_or("");
            // The indentation comes before libxml2 checks for content. The
            // XHTML serializer indents only elements.
            if f.xhtml.is_none() {
                f.indent(s);
            }
            if !text.is_empty() {
                s.push_str("<!--");
                s.push_str(text);
                s.push_str("-->");
            }
        }
        NodeKind::Pi { target, .. } => {
            if f.xhtml.is_none() {
                f.indent(s);
            }
            s.push_str("<?");
            s.push_str(names.str(target));
            let data = doc.value(id).unwrap_or("");
            if !data.is_empty() {
                s.push(' ');
                s.push_str(data);
            }
            s.push_str("?>");
        }
        NodeKind::Root | NodeKind::Attribute { .. } => {}
    }
}

/// libxml2's CDATA writer: a `]]>` in the text splits it into two sections,
/// the first ending after `]]`.
fn cdata(t: &str, s: &mut String) {
    if t.is_empty() {
        s.push_str("<![CDATA[]]>");
        return;
    }
    let mut rest = t;
    while let Some(i) = rest.find("]]>") {
        s.push_str("<![CDATA[");
        s.push_str(&rest[..i + 2]);
        s.push_str("]]>");
        rest = &rest[i + 2..];
    }
    if !rest.is_empty() {
        s.push_str("<![CDATA[");
        s.push_str(rest);
        s.push_str("]]>");
    }
}

fn escape_text(t: &str, s: &mut String) {
    for c in t.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '\r' => s.push_str("&#13;"),
            c => s.push(c),
        }
    }
}

/// libxml2's xmlBufAttrSerializeTxtContent. When the result document has no
/// encoding (no `encoding` on `xsl:output`), it writes non-ASCII characters
/// as hex references, though text keeps them as they are.
fn escape_attr(t: &str, hex_non_ascii: bool, s: &mut String) {
    for c in t.chars() {
        match c {
            c if hex_non_ascii && !c.is_ascii() => s.push_str(&format!("&#x{:X};", c as u32)),
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\n' => s.push_str("&#10;"),
            '\r' => s.push_str("&#13;"),
            '\t' => s.push_str("&#9;"),
            c => s.push(c),
        }
    }
}
