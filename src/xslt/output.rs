//! Building a result tree: the transform's output, or a result tree fragment.
//!
//! An element stays pending until its first child or its end, because
//! `xsl:attribute` can still add attributes (a later one with the same name
//! replaces an earlier one). When the element is written, namespace
//! declarations are fixed up: bindings an output ancestor already declares are
//! dropped, and the element's and its attributes' namespaces are declared if
//! missing.

use super::ir::Binding;
use crate::tree::{Builder, Document, NamePool, NsDecl, QName, Sym, TextKind, TooLarge};
use crate::xpath::ast::Name;

#[derive(Debug)]
pub enum OutputError {
    /// `xsl:attribute` after the element's children: a runtime error.
    AttributeNotAllowed,
    /// `xsl:attribute` with no element to go to: libxslt ignores it.
    NoElement,
    TooLarge,
}

impl From<TooLarge> for OutputError {
    fn from(_: TooLarge) -> Self {
        OutputError::TooLarge
    }
}

struct Pending {
    name: QName,
    namespaces: Vec<Binding>,
    attributes: Vec<(QName, String)>,
    /// Attributes copied without a value (HTML's `<p foo>`), by expanded
    /// name.
    valueless: Vec<(Option<Sym>, Sym)>,
    /// Namespace nodes added after the element's own name is declared: a
    /// literal result element's inherited namespaces.
    late_namespaces: Vec<Binding>,
    /// Declarations made for `xsl:attribute` (xsltGetSpecialNamespace).
    /// libxslt makes them after the element's own, so they come last.
    attribute_decls: Vec<Binding>,
}

pub struct ResultTree {
    builder: Builder,
    pending: Option<Pending>,
    /// Bindings declared on each open output element, outermost first.
    scopes: Vec<Vec<Binding>>,
    /// Names of the open output elements, outermost first.
    open: Vec<QName>,
    /// `cdata-section-elements`: text added directly to these becomes CDATA.
    /// Empty unless the output method is xml (or not yet known).
    cdata: std::rc::Rc<Vec<Name>>,
}

impl ResultTree {
    pub fn new(cdata: std::rc::Rc<Vec<Name>>) -> Self {
        ResultTree {
            builder: Builder::new(),
            pending: None,
            scopes: Vec::new(),
            open: Vec::new(),
            cdata,
        }
    }

    /// Opens an element. Namespaces and attributes may be added until its
    /// first child.
    pub fn start_element(&mut self, name: QName, names: &mut NamePool) -> Result<(), OutputError> {
        self.flush(names)?;
        self.pending = Some(Pending {
            name,
            namespaces: Vec::new(),
            attributes: Vec::new(),
            valueless: Vec::new(),
            late_namespaces: Vec::new(),
            attribute_decls: Vec::new(),
        });
        Ok(())
    }

    /// Adds a namespace node to the pending element. Ignored if there's no
    /// pending element.
    pub fn add_namespace(&mut self, binding: Binding) {
        if let Some(p) = &mut self.pending
            && !p.namespaces.iter().any(|(q, _)| *q == binding.0)
        {
            p.namespaces.push(binding);
        }
    }

    /// Adds a namespace node that libxslt adds after the element's own name:
    /// an inherited namespace of a literal result element.
    pub fn add_late_namespace(&mut self, binding: Binding) {
        if let Some(p) = &mut self.pending {
            p.late_namespaces.push(binding);
        }
    }

    /// Adds an attribute to the pending element; one with the same expanded
    /// name is replaced in place.
    pub fn add_attribute(&mut self, name: QName, value: String) -> Result<(), OutputError> {
        let Some(p) = &mut self.pending else {
            return Err(if self.open.is_empty() {
                OutputError::NoElement
            } else {
                OutputError::AttributeNotAllowed
            });
        };
        match p
            .attributes
            .iter_mut()
            .find(|(q, _)| q.expanded() == name.expanded())
        {
            Some(existing) => *existing = (name, value),
            None => p.attributes.push((name, value)),
        }
        p.valueless.retain(|n| *n != name.expanded());
        Ok(())
    }

    /// Adds a copy of an attribute that has no value at all.
    pub fn add_valueless_attribute(&mut self, name: QName) -> Result<(), OutputError> {
        self.add_attribute(name, String::new())?;
        if let Some(p) = &mut self.pending {
            p.valueless.push(name.expanded());
        }
        Ok(())
    }

    /// Adds text. `unescaped` is `disable-output-escaping="yes"`. Text added
    /// directly to a `cdata-section-elements` element becomes CDATA instead,
    /// as libxslt's xsltCopyTextString decides.
    pub fn text(
        &mut self,
        s: &str,
        unescaped: bool,
        names: &mut NamePool,
    ) -> Result<(), OutputError> {
        if s.is_empty() {
            return Ok(());
        }
        self.flush(names)?;
        let kind = if self
            .open
            .last()
            .is_some_and(|n| self.cdata.contains(&n.expanded()))
        {
            TextKind::Cdata
        } else if unescaped {
            TextKind::Unescaped
        } else {
            TextKind::Normal
        };
        self.builder.text_of_kind(s, kind, 0)?;
        Ok(())
    }

    /// xsltCopyTextString with an empty string: a new empty text node,
    /// unless it would merge with a text node before it.
    pub fn empty_text(&mut self, names: &mut NamePool) -> Result<(), OutputError> {
        self.flush(names)?;
        if self.builder.current_text_len() == 0 && !self.builder.last_child_is_text() {
            self.builder.text_node("", 0)?;
        }
        Ok(())
    }

    pub fn comment(&mut self, s: &str, names: &mut NamePool) -> Result<(), OutputError> {
        self.flush(names)?;
        self.builder.comment(s, 0)?;
        Ok(())
    }

    pub fn pi(&mut self, target: Sym, data: &str, names: &mut NamePool) -> Result<(), OutputError> {
        self.flush(names)?;
        self.builder.pi(target, data, 0)?;
        Ok(())
    }

    pub fn end_element(&mut self, names: &mut NamePool) -> Result<(), OutputError> {
        self.flush(names)?;
        self.builder.end_element();
        self.scopes.pop();
        self.open.pop();
        Ok(())
    }

    pub fn finish(mut self, names: &mut NamePool) -> Result<Document, OutputError> {
        self.flush(names)?;
        Ok(self.builder.finish())
    }

    /// The URI `prefix` is bound to in the output at the pending element,
    /// counting `decls` about to be made on it.
    fn bound(&self, decls: &[Binding], prefix: Option<Sym>) -> Option<Sym> {
        if let Some(&(_, u)) = decls.iter().find(|(p, _)| *p == prefix) {
            return Some(u);
        }
        self.scopes
            .iter()
            .rev()
            .flatten()
            .find(|(p, _)| *p == prefix)
            .map(|&(_, u)| u)
    }

    /// The declarations the pending element makes for its namespace nodes
    /// and its own name, and its name with the prefix it will use. These are
    /// what libxslt has put on the element by the time attributes are added.
    fn element_decls(&self, p: &Pending, names: &mut NamePool) -> (Vec<Binding>, QName) {
        let empty = NamePool::EMPTY;
        let xml_ns = NamePool::XML_NS;
        let xml_prefix = NamePool::XML;
        let is_unbound = |u: Option<Sym>| u.is_none() || u == Some(empty);

        let mut decls: Vec<Binding> = Vec::new();
        for &(prefix, uri) in &p.namespaces {
            if prefix == Some(xml_prefix) || uri == xml_ns {
                continue;
            }
            if decls.iter().any(|(q, _)| *q == prefix) {
                continue;
            }
            let current = self.bound(&decls, prefix);
            let redundant =
                current == Some(uri) || (prefix.is_none() && uri == empty && is_unbound(current));
            if !redundant {
                decls.push((prefix, uri));
            }
        }

        // The element's own namespace.
        let mut name = p.name;
        match name.ns {
            Some(uri) if uri != empty => {
                if self.bound(&decls, name.prefix) != Some(uri) {
                    if decls.iter().any(|(q, _)| *q == name.prefix) {
                        // The prefix is taken by another URI on this element.
                        let prefix = self.fresh_prefix(&decls, names);
                        name.prefix = Some(prefix);
                    }
                    decls.push((name.prefix, uri));
                }
            }
            _ => {
                name.ns = None;
                name.prefix = None;
                if !is_unbound(self.bound(&decls, None)) {
                    decls.push((None, empty));
                }
            }
        }
        // Inherited namespaces come after the element's own name, each only
        // if its prefix isn't bound to the same URI already.
        for &(prefix, uri) in &p.late_namespaces {
            if prefix == Some(xml_prefix) || uri == xml_ns || uri == empty {
                continue;
            }
            if !decls.iter().any(|(q, _)| *q == prefix) && self.bound(&decls, prefix) != Some(uri) {
                decls.push((prefix, uri));
            }
        }
        (decls, name)
    }

    /// The prefix for an `xsl:attribute` in namespace `uri`, chosen as
    /// libxslt's xsltGetSpecialNamespace does, declaring it on the pending
    /// element if needed. `requested` is the name's prefix; without one (or
    /// with `xmlns`) libxslt asks for `ns_1`. Returns None if there's no
    /// pending element.
    pub fn attribute_prefix(
        &mut self,
        uri: Sym,
        requested: Option<Sym>,
        names: &mut NamePool,
    ) -> Option<Sym> {
        let xml_prefix = NamePool::XML;
        let xmlns = NamePool::XMLNS;
        let p = match requested {
            Some(r) if r != xmlns => r,
            _ => names.intern("ns_1"),
        };
        if p == xml_prefix {
            return Some(xml_prefix);
        }
        let pending = self.pending.as_ref()?;
        let (mut on_element, _) = self.element_decls(pending, names);
        on_element.extend(pending.attribute_decls.iter().copied());

        // The element itself declares the prefix: same URI, or occupied.
        if let Some(&(_, u)) = on_element.iter().find(|(q, _)| *q == Some(p)) {
            if u == uri {
                return Some(p);
            }
            return Some(self.prefix_by_href_or_new(&on_element, uri, p, names));
        }
        if let Some(parent) = self.open.last() {
            if parent.ns == Some(uri) && parent.prefix == Some(p) {
                return Some(p);
            }
            if let Some(u) = self.bound(&[], Some(p)) {
                if u == uri {
                    return Some(p);
                }
                // The prefix means something else above. libxslt redeclares
                // it here, unless an attribute already uses it.
                let used = pending
                    .attributes
                    .iter()
                    .any(|(q, _)| q.ns.is_some() && q.prefix == Some(p));
                if used {
                    return Some(self.prefix_by_href_or_new(&on_element, uri, p, names));
                }
            }
        }
        self.pending
            .as_mut()
            .expect("checked above")
            .attribute_decls
            .push((Some(p), uri));
        Some(p)
    }

    /// xmlSearchNsByHref at the pending element, or else a new prefix
    /// `P_1`, `P_2`, … declared on it (libxslt's declare_new_prefix).
    fn prefix_by_href_or_new(
        &mut self,
        on_element: &[Binding],
        uri: Sym,
        p: Sym,
        names: &mut NamePool,
    ) -> Sym {
        let found = on_element
            .iter()
            .chain(self.scopes.iter().rev().flatten())
            .find(|(pfx, u)| {
                *u == uri && pfx.is_some() && self.bound(on_element, *pfx) == Some(uri)
            })
            .and_then(|&(pfx, _)| pfx);
        if let Some(f) = found {
            return f;
        }
        let base = names.str(p).to_string();
        let fresh = (1..)
            .map(|i| names.intern(&format!("{base}_{i}")))
            .find(|&c| self.bound(on_element, Some(c)).is_none())
            .expect("an unused prefix exists");
        self.pending
            .as_mut()
            .expect("pending element")
            .attribute_decls
            .push((Some(fresh), uri));
        fresh
    }

    fn flush(&mut self, names: &mut NamePool) -> Result<(), OutputError> {
        let Some(p) = self.pending.take() else {
            return Ok(());
        };
        let empty = NamePool::EMPTY;
        let xml_ns = NamePool::XML_NS;
        let xml_prefix = NamePool::XML;
        let (mut decls, name) = self.element_decls(&p, names);
        for &(prefix, uri) in &p.attribute_decls {
            if !decls.iter().any(|(q, _)| *q == prefix) && self.bound(&decls, prefix) != Some(uri) {
                decls.push((prefix, uri));
            }
        }

        // Attributes in a namespace need a prefix bound to it.
        let valueless = p.valueless;
        let mut attributes = p.attributes;
        for (q, _) in &mut attributes {
            let Some(uri) = q.ns.filter(|&u| u != empty) else {
                q.ns = None;
                q.prefix = None;
                continue;
            };
            if uri == xml_ns {
                q.prefix = Some(xml_prefix);
                continue;
            }
            if q.prefix.is_some() && self.bound(&decls, q.prefix) == Some(uri) {
                continue;
            }
            let existing = decls
                .iter()
                .chain(self.scopes.iter().rev().flatten())
                .find(|(pfx, u)| {
                    *u == uri && pfx.is_some() && self.bound(&decls, *pfx) == Some(uri)
                })
                .map(|&(pfx, _)| pfx);
            if let Some(pfx) = existing {
                q.prefix = pfx;
                continue;
            }
            let prefix = match q.prefix {
                Some(pfx) if self.bound(&decls, Some(pfx)).is_none() => pfx,
                _ => self.fresh_prefix(&decls, names),
            };
            q.prefix = Some(prefix);
            decls.push((Some(prefix), uri));
        }

        self.builder.start_element(name, 0)?;
        for &(prefix, uri) in &decls {
            self.builder.add_ns_decl(NsDecl { prefix, uri });
        }
        for (q, v) in &attributes {
            self.builder
                .add_attribute_maybe_valueless(*q, v, valueless.contains(&q.expanded()))?;
        }
        self.scopes.push(decls);
        self.open.push(name);
        Ok(())
    }

    /// A prefix not bound at the pending element: `ns_1`, `ns_2`, …
    fn fresh_prefix(&self, decls: &[Binding], names: &mut NamePool) -> Sym {
        (1..)
            .map(|i| names.intern(&format!("ns_{i}")))
            .find(|&p| self.bound(decls, Some(p)).is_none())
            .expect("an unused prefix exists")
    }
}
