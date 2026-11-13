//! The intermediate tree, as libxml2's SAX2 handler builds it (it can attach
//! an element to one finished earlier, so it can't be built in document
//! order), and the `Document` built from it at the end.

use super::*;

pub(super) enum Kind {
    Element {
        name: String,
        attrs: Vec<(String, Option<Vec<u8>>)>,
    },
    Text(Vec<u8>),
    Cdata(Vec<u8>),
    Comment(Vec<u8>),
    Pi {
        target: String,
        data: Vec<u8>,
    },
    /// The DOCTYPE (`xmlCreateIntSubset`), first among the document's
    /// children; XPath doesn't see it.
    Dtd,
}

pub(super) struct Node {
    pub(super) kind: Kind,
    pub(super) children: Vec<usize>,
    line: u32,
}

#[derive(Default)]
pub(super) struct Tree {
    pub(super) nodes: Vec<Node>,
    /// The document's children.
    top: Vec<usize>,
    /// ctxt->node: the open elements (nodePush/nodePop).
    open: Vec<usize>,
    /// The DOCTYPE's public ID, which `areBlanks` looks at.
    pub(super) external_id: Option<Vec<u8>>,
    has_dtd: bool,
}

impl Tree {
    fn add(&mut self, kind: Kind, line: u32) -> usize {
        self.nodes.push(Node {
            kind,
            children: Vec::new(),
            line,
        });
        self.nodes.len() - 1
    }

    pub(super) fn current(&self) -> Option<usize> {
        self.open.last().copied()
    }

    fn last_child(&self, n: usize) -> Option<usize> {
        self.nodes[n].children.last().copied()
    }

    /// xmlSAX2StartElement's placement: the first element is the
    /// document's; with no open element, a new one goes into the
    /// document's first child if that's an element, else after the
    /// document's other children (xmlAddSibling of a comment or DTD).
    pub(super) fn start_element(
        &mut self,
        name: String,
        attrs: Vec<(String, Option<Vec<u8>>)>,
        line: u32,
    ) -> bool {
        if self.open.len() > MAX_DEPTH {
            return false;
        }
        let id = self.add(Kind::Element { name, attrs }, line);
        match self.current() {
            Some(parent) => self.nodes[parent].children.push(id),
            None => match self.top.first() {
                Some(&first) if matches!(self.nodes[first].kind, Kind::Element { .. }) => {
                    self.nodes[first].children.push(id)
                }
                _ => self.top.push(id),
            },
        }
        self.open.push(id);
        true
    }

    pub(super) fn end_element(&mut self) {
        self.open.pop();
    }

    /// xmlSAX2Text: appended to the open element, merged with a last child
    /// of the same type; dropped when no element is open.
    pub(super) fn text(&mut self, s: &[u8], cdata: bool) {
        let Some(parent) = self.current() else {
            return;
        };
        if let Some(last) = self.last_child(parent) {
            match &mut self.nodes[last].kind {
                Kind::Text(t) if !cdata => {
                    if t.len() + s.len() <= MAX_TEXT {
                        t.extend_from_slice(s);
                    }
                    return;
                }
                Kind::Cdata(t) if cdata => {
                    if t.len() + s.len() <= MAX_TEXT {
                        t.extend_from_slice(s);
                    }
                    return;
                }
                _ => {}
            }
        }
        let kind = if cdata {
            Kind::Cdata(s.to_vec())
        } else {
            Kind::Text(s.to_vec())
        };
        let id = self.add(kind, 0);
        self.nodes[parent].children.push(id);
    }

    /// xmlSAX2Comment and xmlSAX2ProcessingInstruction.
    pub(super) fn misc(&mut self, kind: Kind, line: u32) {
        let id = self.add(kind, line);
        match self.current() {
            Some(parent) => self.nodes[parent].children.push(id),
            None => self.top.push(id),
        }
    }

    /// xmlSAX2InternalSubset: the first DOCTYPE only, as the document's
    /// first child.
    pub(super) fn doctype(&mut self, external_id: Option<Vec<u8>>) {
        if self.has_dtd {
            return;
        }
        self.has_dtd = true;
        self.external_id = external_id;
        let id = self.add(Kind::Dtd, 0);
        self.top.insert(0, id);
    }

    pub(super) fn build(self, names: &mut NamePool) -> Document {
        let mut b = Builder::new();
        // Depth-first, with an explicit stack: (node, children emitted).
        let mut stack: Vec<(usize, usize)> = Vec::new();
        let mut top = self.top.iter();
        loop {
            let next = match stack.last_mut() {
                Some((n, i)) => {
                    let n = *n;
                    match self.nodes[n].children.get(*i) {
                        Some(&c) => {
                            *i += 1;
                            Some(c)
                        }
                        None => {
                            stack.pop();
                            b.end_element();
                            continue;
                        }
                    }
                }
                None => top.next().copied(),
            };
            let Some(id) = next else { break };
            let node = &self.nodes[id];
            // The tree can't be larger than the input, so TooLarge can't
            // happen before the parser's own input limit; ignore it.
            match &node.kind {
                Kind::Element { name, attrs } => {
                    let qname = QName {
                        ns: None,
                        local: names.intern(name),
                        prefix: None,
                    };
                    let _ = b.start_element(qname, node.line);
                    for (a, v) in attrs {
                        let an = QName {
                            ns: None,
                            local: names.intern(a),
                            prefix: None,
                        };
                        let _ = match v {
                            Some(v) => b.add_attribute(an, &lossy(v)),
                            None => b.add_valueless_attribute(an),
                        };
                    }
                    // xmlIsID for HTML documents, and xml:id.
                    for (a, v) in attrs {
                        if let Some(v) = v
                            && (a == "id" || a == "xml:id" || (a == "name" && name == "a"))
                        {
                            b.add_id(&lossy(v));
                        }
                    }
                    stack.push((id, 0));
                }
                Kind::Text(t) => {
                    let _ = b.text_of_kind(&lossy(t), TextKind::Normal, node.line);
                }
                Kind::Cdata(t) => {
                    let _ = b.text_of_kind(&lossy(t), TextKind::Cdata, node.line);
                }
                Kind::Comment(t) => {
                    let _ = b.comment(&lossy(t), node.line);
                }
                Kind::Pi { target, data } => {
                    let _ = b.pi(names.intern(target), &lossy(data), node.line);
                }
                Kind::Dtd => {}
            }
        }
        b.finish()
    }
}

/// libxml2 keeps whatever bytes it copied; a Rust string can't hold
/// invalid UTF-8.
pub(super) fn lossy(v: &[u8]) -> String {
    String::from_utf8_lossy(v).into_owned()
}
