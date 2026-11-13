//! What the parser knows about HTML elements and entities, from libxml2's
//! tables (tables.rs): auto-closing, end-tag priorities, entity values,
//! allowPCData, and how element names lose their prefix.

use super::*;

pub(super) fn element_info(name: &str) -> Option<(u8, u8, bool)> {
    tables::ELEMENTS
        .binary_search_by(|(n, ..)| n.cmp(&name))
        .ok()
        .map(|i| {
            let (_, start, end, empty) = tables::ELEMENTS[i];
            (start, end, empty)
        })
}

/// htmlCheckAutoClose: does the start tag `new` close an open `old`?
pub(super) fn check_auto_close(new: &str, old: &str) -> bool {
    tables::START_CLOSE
        .binary_search_by(|&(o, n)| o.cmp(old).then(n.cmp(new)))
        .is_ok()
}

/// htmlGetEndPriority.
pub(super) fn end_priority(name: &str) -> u32 {
    match name {
        "div" => 150,
        "td" | "th" => 160,
        "tr" => 170,
        "thead" | "tbody" | "tfoot" => 180,
        "table" => 190,
        "head" | "body" => 200,
        "html" => 220,
        _ => 100,
    }
}

/// htmlEntityLookup.
pub(super) fn entity(name: &[u8]) -> Option<u32> {
    tables::ENTITIES
        .iter()
        .find(|(n, _)| n.as_bytes() == name)
        .map(|&(_, v)| v)
}

/// allowPCData: elements whose blank text `areBlanks` keeps.
pub(super) const ALLOW_PCDATA: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "applet",
    "b",
    "bdo",
    "big",
    "blockquote",
    "body",
    "button",
    "caption",
    "center",
    "cite",
    "code",
    "dd",
    "del",
    "dfn",
    "div",
    "dt",
    "em",
    "font",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "i",
    "iframe",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "noframes",
    "noscript",
    "object",
    "p",
    "pre",
    "q",
    "s",
    "samp",
    "small",
    "span",
    "strike",
    "strong",
    "td",
    "th",
    "tt",
    "u",
    "var",
];

/// xmlSplitQName as xmlSAX2StartElement applies it to HTML element names:
/// the part after the first colon, unless the name starts with a colon or
/// ends with its only colon.
pub(super) fn local_name(name: &str) -> String {
    // (Its `xml:` exception is compiled out: XML_XML_NAMESPACE is defined.)
    if name.starts_with(':') {
        return name.to_string();
    }
    match name.split_once(':') {
        Some((_, "")) => name.to_string(),
        Some((_, local)) => local.to_string(),
        None => name.to_string(),
    }
}
