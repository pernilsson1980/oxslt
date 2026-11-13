//! The document tree: every node of a document lives in one `Vec`, linked by
//! [`NodeId`] indices instead of pointers.
//!
//! Nodes are stored in document order: an element is followed by its
//! attributes, then its children. So comparing two `NodeId`s of the same
//! document compares their XPath document order. Text, attribute values,
//! comments and processing-instruction data live in one string buffer per
//! document, referenced by [`Str`] ranges.
//!
//! Names are interned in a [`NamePool`] shared by every document and the
//! stylesheet of a transform, so name comparisons are integer comparisons.

use std::collections::{HashMap, HashSet};

/// A fast hasher for keys made of interned symbols ([`Sym`]) and node
/// indices: an Fx-style multiply-rotate.
///
/// Only for those keys. Symbols and node IDs are numbered 0, 1, 2, … as
/// they're created, so a document can't choose their bit patterns, and a
/// multiplicative hash spreads consecutive numbers without collisions.
/// Strings are attacker-controlled; maps keyed by them keep std's keyed
/// SipHash, so a document can't make them collide (hash flooding).
#[derive(Debug, Default, Clone, Copy)]
pub struct SymHasher(u64);

impl SymHasher {
    #[inline]
    fn add(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl std::hash::Hasher for SymHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut b = [0u8; 8];
            b[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(b));
        }
    }
    #[inline]
    fn write_u8(&mut self, x: u8) {
        self.add(u64::from(x));
    }
    #[inline]
    fn write_u32(&mut self, x: u32) {
        self.add(u64::from(x));
    }
    #[inline]
    fn write_u64(&mut self, x: u64) {
        self.add(x);
    }
    #[inline]
    fn write_usize(&mut self, x: usize) {
        self.add(x as u64);
    }
    #[inline]
    fn write_isize(&mut self, x: isize) {
        self.add(x as u64);
    }
}

/// A `HashMap` keyed by symbols or node indices; see [`SymHasher`].
pub type SymMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<SymHasher>>;

/// An interned string: a name, a namespace URI or a prefix.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Sym(u32);

/// Interns names for a whole transform.
#[derive(Debug)]
pub struct NamePool {
    names: Vec<Box<str>>,
    index: HashMap<Box<str>, Sym>,
    /// A front cache for `intern`: recently interned symbols, in slots
    /// chosen by a fast unkeyed hash, each hit confirmed by comparing the
    /// string. Most names repeat, and this skips `index`'s SipHash for them.
    /// Colliding names only cause misses, which fall back to `index`, so a
    /// document can't make interning slower than without the cache.
    recent: Box<[u32; RECENT]>,
}

/// Slots in `NamePool::recent`; a power of two.
const RECENT: usize = 512;
/// An empty slot in `NamePool::recent`.
const NO_SYM: u32 = u32::MAX;

/// The slot in `NamePool::recent` for `s`.
#[inline]
fn recent_slot(s: &str) -> usize {
    use std::hash::Hasher;
    let mut h = SymHasher::default();
    h.write(s.as_bytes());
    // The top bits are the best mixed.
    (h.finish() >> 55) as usize & (RECENT - 1)
}

impl Default for NamePool {
    fn default() -> Self {
        Self::new()
    }
}

impl NamePool {
    /// The `xml` prefix.
    pub const XML: Sym = Sym(0);
    /// The namespace URI the `xml` prefix is bound to.
    pub const XML_NS: Sym = Sym(1);
    /// The empty string.
    pub const EMPTY: Sym = Sym(2);
    /// `xmlns`.
    pub const XMLNS: Sym = Sym(3);
    /// `space`, as in `xml:space`.
    pub const SPACE: Sym = Sym(4);
    /// `base`, as in `xml:base`.
    pub const BASE: Sym = Sym(5);
    /// `lang`, as in `xml:lang`.
    pub const LANG: Sym = Sym(6);

    /// A pool that already holds the constants above. Every element has the
    /// `xml` prefix and its namespace URI in scope: the XPath namespace axis
    /// needs them even in documents that never mention them.
    pub fn new() -> Self {
        let mut pool = NamePool {
            names: Vec::new(),
            index: HashMap::new(),
            recent: Box::new([NO_SYM; RECENT]),
        };
        for (s, sym) in [
            ("xml", Self::XML),
            ("http://www.w3.org/XML/1998/namespace", Self::XML_NS),
            ("", Self::EMPTY),
            ("xmlns", Self::XMLNS),
            ("space", Self::SPACE),
            ("base", Self::BASE),
            ("lang", Self::LANG),
        ] {
            let got = pool.intern(s);
            debug_assert_eq!(got, sym);
        }
        pool
    }

    pub fn intern(&mut self, s: &str) -> Sym {
        let slot = recent_slot(s);
        let cached = self.recent[slot];
        if cached != NO_SYM && *self.names[cached as usize] == *s {
            return Sym(cached);
        }
        if let Some(&sym) = self.index.get(s) {
            self.recent[slot] = sym.0;
            return sym;
        }
        // More than 4 billion distinct names can't come from a real document;
        // the node limit is hit long before.
        let sym = Sym(u32::try_from(self.names.len()).expect("name pool overflow"));
        self.names.push(s.into());
        self.index.insert(s.into(), sym);
        self.recent[slot] = sym.0;
        sym
    }

    /// The symbol for `s`, if it has been interned.
    pub fn get(&self, s: &str) -> Option<Sym> {
        self.index.get(s).copied()
    }

    pub fn str(&self, sym: Sym) -> &str {
        &self.names[sym.0 as usize]
    }
}

/// An element or attribute name. Two names are the same XPath name when their
/// [`QName::expanded`] forms are equal; the prefix is kept only for output.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct QName {
    pub ns: Option<Sym>,
    pub local: Sym,
    pub prefix: Option<Sym>,
}

impl QName {
    pub fn expanded(&self) -> (Option<Sym>, Sym) {
        (self.ns, self.local)
    }
}

/// A node of one document. Ordering follows document order.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct NodeId(u32);

impl NodeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A range in a document's string buffer.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Str {
    start: u32,
    len: u32,
}

/// A namespace declaration on an element: `xmlns` (prefix `None`) or `xmlns:p`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct NsDecl {
    pub prefix: Option<Sym>,
    pub uri: Sym,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum NodeKind {
    Root,
    Element {
        name: QName,
        /// Attributes are the nodes right after the element.
        attr_count: u32,
        /// A range in [`Document::ns_decls`].
        ns_start: u32,
        ns_count: u32,
    },
    Attribute {
        name: QName,
        value: Str,
    },
    Text(Str),
    Comment(Str),
    Pi {
        target: Sym,
        data: Str,
    },
}

#[derive(Copy, Clone, Debug)]
struct Node {
    kind: NodeKind,
    parent: Option<NodeId>,
    first_child: Option<NodeId>,
    last_child: Option<NodeId>,
    prev_sibling: Option<NodeId>,
    next_sibling: Option<NodeId>,
    /// Source line, for error messages.
    line: u32,
}

/// How a result tree's text node is written. To XPath every kind is
/// ordinary text; only the serializer tells them apart.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum TextKind {
    #[default]
    Normal,
    /// `disable-output-escaping="yes"`: written as it is.
    Unescaped,
    /// Text of a `cdata-section-elements` element: written as `<![CDATA[…]]>`.
    Cdata,
}

#[derive(Debug)]
pub struct Document {
    nodes: Vec<Node>,
    text: String,
    ns_decls: Vec<NsDecl>,
    /// Text nodes that aren't [`TextKind::Normal`]. Only result trees have any.
    text_kinds: HashMap<NodeId, TextKind>,
    /// ID values (attributes the DTD declares as ID, and xml:id) to their
    /// elements. The first element with a value wins, as in libxml2.
    ids: HashMap<String, NodeId>,
    /// Unparsed entities the DTD declares, by name, to their URI.
    unparsed_entities: HashMap<String, String>,
    /// Attributes with no value at all, as HTML's `<p foo>` gives (libxml2
    /// leaves them without children). XPath sees "", but libxml2's HTML
    /// serializer writes just the name, and copies keep that.
    valueless: HashSet<NodeId>,
    /// The input had a `<!DOCTYPE>`. The PoC doesn't process DTDs.
    pub has_doctype: bool,
    /// The document's URL as libxml2 records it (`doc->URL`): the path as
    /// given for the input, or as `uri::build` resolved it. None for result
    /// tree fragments.
    pub url: Option<String>,
    /// A result tree fragment: a variable's or parameter's content.
    pub fragment: bool,
    /// Parsed with `--html` (libxml2's XML_HTML_DOCUMENT_NODE), which XPath
    /// equality treats differently (`xpath::eval::node_hash`).
    pub html: bool,
}

impl Document {
    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Every node, in document order.
    pub fn all_nodes(&self) -> impl Iterator<Item = NodeId> + use<> {
        (0..self.nodes.len() as u32).map(NodeId)
    }

    fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    pub fn kind(&self, id: NodeId) -> NodeKind {
        self.node(id).kind
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).first_child
    }

    pub fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).last_child
    }

    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).next_sibling
    }

    pub fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).prev_sibling
    }

    /// The element with this ID, for XPath's `id()`.
    pub fn element_by_id(&self, id: &str) -> Option<NodeId> {
        self.ids.get(id).copied()
    }

    /// Every ID and its element.
    pub fn ids(&self) -> impl Iterator<Item = (&str, NodeId)> {
        self.ids.iter().map(|(v, &n)| (v.as_str(), n))
    }

    /// Every unparsed entity: name and URI.
    pub fn unparsed_entities(&self) -> impl Iterator<Item = (&str, &str)> {
        self.unparsed_entities
            .iter()
            .map(|(n, u)| (n.as_str(), u.as_str()))
    }

    /// An unparsed entity's URI, for XSLT's `unparsed-entity-uri()`.
    pub fn unparsed_entity_uri(&self, name: &str) -> Option<&str> {
        self.unparsed_entities.get(name).map(String::as_str)
    }

    /// An attribute without a value (see `valueless`).
    pub fn is_valueless(&self, id: NodeId) -> bool {
        self.valueless.contains(&id)
    }

    pub fn text_kind(&self, id: NodeId) -> TextKind {
        self.text_kinds.get(&id).copied().unwrap_or_default()
    }

    pub fn line(&self, id: NodeId) -> u32 {
        self.node(id).line
    }

    pub fn str(&self, s: Str) -> &str {
        &self.text[s.start as usize..(s.start + s.len) as usize]
    }

    /// The name of an element or attribute.
    pub fn name(&self, id: NodeId) -> Option<QName> {
        match self.kind(id) {
            NodeKind::Element { name, .. } | NodeKind::Attribute { name, .. } => Some(name),
            _ => None,
        }
    }

    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            doc: self,
            next: self.first_child(id),
        }
    }

    pub fn attributes(&self, id: NodeId) -> impl Iterator<Item = NodeId> + use<> {
        let count = match self.kind(id) {
            NodeKind::Element { attr_count, .. } => attr_count,
            _ => 0,
        };
        (id.0 + 1..id.0 + 1 + count).map(NodeId)
    }

    /// The namespace declarations made on an element (not those inherited).
    pub fn ns_decls(&self, id: NodeId) -> &[NsDecl] {
        match self.kind(id) {
            NodeKind::Element {
                ns_start, ns_count, ..
            } => &self.ns_decls[ns_start as usize..(ns_start + ns_count) as usize],
            _ => &[],
        }
    }

    /// The node's own text: an attribute's value, a text node's text, a
    /// comment's content or a processing instruction's data.
    pub fn value(&self, id: NodeId) -> Option<&str> {
        match self.kind(id) {
            NodeKind::Attribute { value: s, .. }
            | NodeKind::Text(s)
            | NodeKind::Comment(s)
            | NodeKind::Pi { data: s, .. } => Some(self.str(s)),
            NodeKind::Root | NodeKind::Element { .. } => None,
        }
    }

    /// The XPath string-value: for the root and elements, the concatenation of
    /// all descendant text nodes in document order.
    /// xmlNodeGetBase: the base URI of a node, from the `xml:base`
    /// attributes on it and its ancestors, resolved against the document's
    /// URL. `xml_base` is the interned name of `xml:base`.
    pub fn base_uri(&self, id: NodeId, xml_base: Option<(Sym, Sym)>) -> Option<String> {
        let mut base: Option<String> = None;
        if let Some((xml, local)) = xml_base {
            let mut cur = Some(id);
            while let Some(n) = cur {
                if matches!(self.kind(n), NodeKind::Element { .. })
                    && let Some(a) = self.attributes(n).find(|&a| {
                        self.name(a)
                            .is_some_and(|q| q.ns == Some(xml) && q.local == local)
                    })
                {
                    let value = self.value(a).unwrap_or("");
                    base = Some(match base {
                        Some(inner) => crate::uri::build(&inner, Some(value)),
                        None => value.to_string(),
                    });
                    let b = base.as_deref().unwrap_or("");
                    if b.starts_with("http://") || b.starts_with("ftp://") || b.starts_with("urn:")
                    {
                        return base;
                    }
                }
                cur = self.parent(n);
            }
        }
        match (base, &self.url) {
            (None, url) => url.clone(),
            (Some(b), Some(url)) => Some(crate::uri::build(&b, Some(url))),
            (Some(b), None) => Some(b),
        }
    }

    pub fn string_value(&self, id: NodeId) -> String {
        if let Some(v) = self.value(id) {
            return v.to_string();
        }
        let mut out = String::new();
        let mut stack: Vec<NodeId> = self.children(id).collect();
        stack.reverse();
        while let Some(n) = stack.pop() {
            match self.kind(n) {
                NodeKind::Text(s) => out.push_str(self.str(s)),
                NodeKind::Element { .. } => {
                    let start = stack.len();
                    stack.extend(self.children(n));
                    stack[start..].reverse();
                }
                _ => {}
            }
        }
        out
    }
}

#[derive(Debug)]
pub struct Children<'a> {
    doc: &'a Document,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let id = self.next?;
        self.next = self.doc.next_sibling(id);
        Some(id)
    }
}

/// Too many nodes or too much text for 32-bit indices.
#[derive(Debug)]
pub struct TooLarge;

impl std::fmt::Display for TooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("document too large: too many nodes or too much text")
    }
}

impl std::error::Error for TooLarge {}

/// Builds a [`Document`] in document order. Used by the parser.
#[derive(Debug)]
pub struct Builder {
    doc: Document,
    open: Vec<NodeId>,
    /// Set by `break_text`: the next text starts a node of its own.
    text_break: bool,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Self {
        let root = Node {
            kind: NodeKind::Root,
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            line: 0,
        };
        Builder {
            doc: Document {
                nodes: vec![root],
                text: String::new(),
                ns_decls: Vec::new(),
                text_kinds: HashMap::new(),
                ids: HashMap::new(),
                unparsed_entities: HashMap::new(),
                valueless: HashSet::new(),
                has_doctype: false,
                url: None,
                fragment: false,
                html: false,
            },
            open: vec![NodeId(0)],
            text_break: false,
        }
    }

    pub fn set_has_doctype(&mut self) {
        self.doc.has_doctype = true;
    }

    /// Registers the current element under an ID value, unless the value is
    /// already taken.
    pub fn add_id(&mut self, value: &str) {
        let el = self.current();
        self.doc.ids.entry(value.to_string()).or_insert(el);
    }

    pub fn add_unparsed_entity(&mut self, name: &str, uri: &str) {
        self.doc
            .unparsed_entities
            .insert(name.to_string(), uri.to_string());
    }

    /// Depth of open elements, not counting the root.
    pub fn depth(&self) -> usize {
        self.open.len() - 1
    }

    fn current(&self) -> NodeId {
        *self.open.last().expect("root is always open")
    }

    fn push_str(&mut self, s: &str) -> Result<Str, TooLarge> {
        let start = u32::try_from(self.doc.text.len()).map_err(|_| TooLarge)?;
        let len = u32::try_from(s.len()).map_err(|_| TooLarge)?;
        start.checked_add(len).ok_or(TooLarge)?;
        self.doc.text.push_str(s);
        Ok(Str { start, len })
    }

    fn new_node(&mut self, kind: NodeKind, parent: NodeId, line: u32) -> Result<NodeId, TooLarge> {
        let id = NodeId(u32::try_from(self.doc.nodes.len()).map_err(|_| TooLarge)?);
        self.doc.nodes.push(Node {
            kind,
            parent: Some(parent),
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            line,
        });
        Ok(id)
    }

    /// Adds a node as the last child of the current element.
    fn append_child(&mut self, kind: NodeKind, line: u32) -> Result<NodeId, TooLarge> {
        let parent = self.current();
        let id = self.new_node(kind, parent, line)?;
        let prev = self.doc.nodes[parent.index()].last_child;
        self.doc.nodes[id.index()].prev_sibling = prev;
        match prev {
            Some(p) => self.doc.nodes[p.index()].next_sibling = Some(id),
            None => self.doc.nodes[parent.index()].first_child = Some(id),
        }
        self.doc.nodes[parent.index()].last_child = Some(id);
        Ok(id)
    }

    /// Opens an element. Its namespace declarations and attributes must be
    /// added next, before any child.
    pub fn start_element(&mut self, name: QName, line: u32) -> Result<NodeId, TooLarge> {
        let ns_start = u32::try_from(self.doc.ns_decls.len()).map_err(|_| TooLarge)?;
        let kind = NodeKind::Element {
            name,
            attr_count: 0,
            ns_start,
            ns_count: 0,
        };
        let id = self.append_child(kind, line)?;
        self.open.push(id);
        Ok(id)
    }

    pub fn add_ns_decl(&mut self, decl: NsDecl) {
        self.doc.ns_decls.push(decl);
        let el = self.current();
        if let NodeKind::Element { ns_count, .. } = &mut self.doc.nodes[el.index()].kind {
            *ns_count += 1;
        }
    }

    pub fn add_attribute(&mut self, name: QName, value: &str) -> Result<(), TooLarge> {
        let el = self.current();
        debug_assert!(
            self.doc.first_child(el).is_none(),
            "attributes must come before children"
        );
        let value = self.push_str(value)?;
        let line = self.doc.nodes[el.index()].line;
        self.new_node(NodeKind::Attribute { name, value }, el, line)?;
        if let NodeKind::Element { attr_count, .. } = &mut self.doc.nodes[el.index()].kind {
            *attr_count += 1;
        }
        Ok(())
    }

    /// An attribute without a value: XPath sees "".
    pub fn add_valueless_attribute(&mut self, name: QName) -> Result<(), TooLarge> {
        self.add_attribute(name, "")?;
        let id = NodeId(self.doc.nodes.len() as u32 - 1);
        self.doc.valueless.insert(id);
        Ok(())
    }

    /// `add_attribute`, or `add_valueless_attribute` when `valueless`.
    pub fn add_attribute_maybe_valueless(
        &mut self,
        name: QName,
        value: &str,
        valueless: bool,
    ) -> Result<(), TooLarge> {
        if valueless {
            self.add_valueless_attribute(name)
        } else {
            self.add_attribute(name, value)
        }
    }

    pub fn end_element(&mut self) {
        debug_assert!(self.open.len() > 1, "end_element without an open element");
        if self.open.len() > 1 {
            self.open.pop();
        }
    }

    /// Adds text to the current element, merging it with a directly preceding
    /// text node: in the XPath data model adjacent text and CDATA are one node.
    pub fn text(&mut self, s: &str, line: u32) -> Result<(), TooLarge> {
        self.text_of_kind(s, TextKind::Normal, line)
    }

    /// Adds text of a given kind. It merges only with a preceding text node
    /// of the same kind, as libxslt's xsltCopyTextString does.
    pub fn text_of_kind(&mut self, s: &str, kind: TextKind, line: u32) -> Result<(), TooLarge> {
        if s.is_empty() {
            return Ok(());
        }
        let parent = self.current();
        if !std::mem::take(&mut self.text_break)
            && let Some(last) = self.doc.nodes[parent.index()].last_child
            && let NodeKind::Text(prev) = self.doc.nodes[last.index()].kind
            && (prev.start + prev.len) as usize == self.doc.text.len()
            && self.doc.text_kind(last) == kind
        {
            let added = self.push_str(s)?;
            if let NodeKind::Text(t) = &mut self.doc.nodes[last.index()].kind {
                t.len = t.len.checked_add(added.len).ok_or(TooLarge)?;
            }
            return Ok(());
        }
        let s = self.push_str(s)?;
        let id = self.append_child(NodeKind::Text(s), line)?;
        if kind != TextKind::Normal {
            self.doc.text_kinds.insert(id, kind);
        }
        Ok(())
    }

    /// Keeps the next text from merging with the text before it. libxml2
    /// leaves an entity-reference node where an undeclared entity was
    /// dropped, so the text around it is two nodes.
    pub fn break_text(&mut self) {
        self.text_break = true;
    }

    /// The length in bytes of the text node being built: the current
    /// element's last child, if it's text. 0 otherwise.
    pub fn current_text_len(&self) -> usize {
        let parent = self.current();
        match self.doc.nodes[parent.index()].last_child {
            Some(last) => match self.doc.nodes[last.index()].kind {
                NodeKind::Text(t) => t.len as usize,
                _ => 0,
            },
            None => 0,
        }
    }

    /// Whether the current element's last child is a text node.
    pub fn last_child_is_text(&self) -> bool {
        let parent = self.current();
        self.doc.nodes[parent.index()]
            .last_child
            .is_some_and(|l| matches!(self.doc.nodes[l.index()].kind, NodeKind::Text(_)))
    }

    /// A text node even when `s` is empty, which only extension functions
    /// make (libexslt's text nodes in result tree fragments).
    pub fn text_node(&mut self, s: &str, line: u32) -> Result<NodeId, TooLarge> {
        let s = self.push_str(s)?;
        self.append_child(NodeKind::Text(s), line)
    }

    pub fn comment(&mut self, s: &str, line: u32) -> Result<(), TooLarge> {
        let s = self.push_str(s)?;
        self.append_child(NodeKind::Comment(s), line)?;
        Ok(())
    }

    pub fn pi(&mut self, target: Sym, data: &str, line: u32) -> Result<(), TooLarge> {
        let data = self.push_str(data)?;
        self.append_child(NodeKind::Pi { target, data }, line)?;
        Ok(())
    }

    pub fn finish(self) -> Document {
        self.doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The front cache never changes what `intern` returns, even with far
    /// more names than slots, so many share a slot.
    #[test]
    fn interning_with_the_front_cache() {
        let mut pool = NamePool::new();
        let names: Vec<String> = (0..5000).map(|i| format!("n{i}")).collect();
        let syms: Vec<Sym> = names.iter().map(|n| pool.intern(n)).collect();
        for round in 0..3 {
            for (i, n) in names.iter().enumerate().rev() {
                assert_eq!(pool.intern(n), syms[i], "{n} round {round}");
                assert_eq!(pool.get(n), Some(syms[i]));
                assert_eq!(pool.str(syms[i]), n);
            }
        }
        assert_eq!(pool.intern(""), NamePool::EMPTY);
        assert_eq!(pool.intern("space"), NamePool::SPACE);
        assert_eq!(pool.get("never"), None);
    }

    fn name(pool: &mut NamePool, local: &str) -> QName {
        QName {
            ns: None,
            local: pool.intern(local),
            prefix: None,
        }
    }

    #[test]
    fn interning_returns_the_same_symbol() {
        let mut pool = NamePool::new();
        let a = pool.intern("para");
        assert_eq!(a, pool.intern("para"));
        assert_ne!(a, pool.intern("title"));
        assert_eq!(pool.str(a), "para");
        assert_eq!(pool.get("title"), Some(pool.intern("title")));
        assert_eq!(pool.get("missing"), None);
    }

    #[test]
    fn document_order_and_navigation() {
        let mut pool = NamePool::new();
        let mut b = Builder::new();
        let doc_el = b.start_element(name(&mut pool, "doc"), 1).unwrap();
        b.add_attribute(name(&mut pool, "a"), "1").unwrap();
        b.text("x", 1).unwrap();
        let child = b.start_element(name(&mut pool, "c"), 2).unwrap();
        b.text("y", 2).unwrap();
        b.end_element();
        b.comment(" note ", 3).unwrap();
        b.end_element();
        let doc = b.finish();

        let attr = doc.attributes(doc_el).next().unwrap();
        assert!(
            doc_el < attr && attr < child,
            "element, then attributes, then children"
        );
        assert_eq!(doc.parent(attr), Some(doc_el));
        assert_eq!(doc.value(attr), Some("1"));
        assert_eq!(doc.children(doc.root()).collect::<Vec<_>>(), vec![doc_el]);
        assert_eq!(doc.children(doc_el).count(), 3);
        assert_eq!(doc.string_value(doc_el), "xy");
        assert_eq!(doc.string_value(doc.root()), "xy");
        assert_eq!(doc.line(child), 2);
        let last = doc.last_child(doc_el).unwrap();
        assert!(matches!(doc.kind(last), NodeKind::Comment(_)));
        assert_eq!(doc.prev_sibling(last), Some(child));
    }

    #[test]
    fn adjacent_text_merges_into_one_node() {
        let mut pool = NamePool::new();
        let mut b = Builder::new();
        let el = b.start_element(name(&mut pool, "d"), 1).unwrap();
        b.text("a", 1).unwrap();
        b.text("b", 1).unwrap();
        b.text("", 1).unwrap();
        b.text("c", 1).unwrap();
        b.end_element();
        let doc = b.finish();
        let kids: Vec<_> = doc.children(el).collect();
        assert_eq!(kids.len(), 1);
        assert_eq!(doc.value(kids[0]), Some("abc"));
    }
}
