//! XInclude 1.0, as libxml2 2.9.14 processes it for `xsltproc --xinclude`
//! (`xmlXIncludeProcessFlags` with xsltproc's parser options).
//!
//! libxml2 edits the tree in place; oxslt rebuilds the document, copying
//! every node and putting each `xi:include`'s replacement in its place. What
//! stays the same:
//!
//! - Includes are found as libxml2's first phase finds them: not inside
//!   another include, and an `include` with an `include` child or several
//!   `fallback`s is an error that leaves it an ordinary element.
//! - An included document is parsed with the same options and has its own
//!   includes processed first. Recursion through the URLs being processed
//!   is an error.
//! - The replacement is the document's top-level nodes (not its DTD), or
//!   the nodes an `xpointer` selects (the shorthand ID form, `element()`,
//!   `xpointer()` and `xmlns()`). `parse="text"` includes the file's text,
//!   unchanged, in its `encoding`.
//! - A failed include uses its `fallback`; without one it stays as it is and
//!   the whole processing reports an error, which makes xsltproc skip the
//!   input (exit 6).
//! - libxml2 leaves XINCLUDE_START and XINCLUDE_END marker nodes in the
//!   tree. XPath never sees them, but they keep the text on either side of
//!   an include apart, so the rebuilt document breaks text there too.
//! - The xml:base fixup: when the included file's URL, relative to the
//!   include's base, contains a '/', each top-level included element gets an
//!   `xml:base` attribute.
//! - IDs of included elements, and unparsed entities of included documents,
//!   carry over.

use std::collections::HashMap;

use crate::catalog::ResourceLoader;
use crate::parse::{ParseOptions, parse_with};
use crate::tree::{Builder, Document, NamePool, NodeId, NodeKind, QName, Sym};
use crate::xpath::{self, Empty, Env, Focus, NodeRef, Value};

pub const NS: &str = "http://www.w3.org/2001/XInclude";
pub const OLD_NS: &str = "http://www.w3.org/2003/XInclude";

/// The result of processing a document's XIncludes.
#[derive(Debug)]
pub struct Processed {
    pub document: Document,
    /// XInclude errors. Any error makes `xmlXIncludeProcessFlags` fail, and
    /// xsltproc then skips the input.
    pub errors: Vec<String>,
    /// Other messages: parser warnings and errors in included files, which
    /// only make that include fall back.
    pub warnings: Vec<String>,
}

/// Processes `doc`'s XIncludes.
pub fn process(
    doc: Document,
    names: &mut NamePool,
    loader: &ResourceLoader,
    options: &ParseOptions,
) -> Processed {
    let mut ctx = Ctx {
        names,
        loader,
        options,
        errors: Vec::new(),
        warnings: Vec::new(),
        urls: Vec::new(),
        texts: HashMap::new(),
        legacy: false,
    };
    let document = ctx.process_doc(doc);
    Processed {
        document,
        errors: ctx.errors,
        warnings: ctx.warnings,
    }
}

struct Ctx<'a> {
    names: &'a mut NamePool,
    loader: &'a ResourceLoader,
    options: &'a ParseOptions,
    errors: Vec<String>,
    warnings: Vec<String>,
    /// URLs of the documents being processed, for recursion checks.
    urls: Vec<String>,
    /// Text includes already read, by URL.
    texts: HashMap<String, String>,
    /// The old 2003 namespace was seen: href may carry a fragment.
    legacy: bool,
}

/// What an include becomes.
enum Replacement {
    /// Nodes of a document, copied; `base` is the xml:base fixup.
    Nodes {
        doc: Box<Document>,
        nodes: Vec<NodeRef>,
        base: Option<String>,
    },
    /// The current document's own nodes (a local reference).
    Local {
        nodes: Vec<NodeId>,
        base: Option<String>,
    },
    Text(String),
    /// A fallback's children, processed for includes in turn.
    Fallback(NodeId),
}

impl Ctx<'_> {
    fn xi_sym(&self, uri: &str) -> Option<Sym> {
        self.names.get(uri)
    }

    /// Whether `doc` has any element in an XInclude namespace.
    fn has_xinclude(&self, doc: &Document) -> bool {
        let syms: Vec<Sym> = [NS, OLD_NS].iter().filter_map(|u| self.xi_sym(u)).collect();
        !syms.is_empty()
            && doc.all_nodes().any(|n| {
                matches!(doc.kind(n), NodeKind::Element { name, .. } if name.ns.is_some_and(|ns| syms.contains(&ns)))
            })
    }

    /// The XInclude local name of `n`, if it's an element in either
    /// XInclude namespace; notes the legacy namespace.
    fn xi_name(&mut self, doc: &Document, n: NodeId) -> Option<&'static str> {
        let NodeKind::Element { name, .. } = doc.kind(n) else {
            return None;
        };
        let ns = self.names.str(name.ns?);
        if ns == OLD_NS {
            self.legacy = true;
        } else if ns != NS {
            return None;
        }
        Some(match self.names.str(name.local) {
            "include" => "include",
            "fallback" => "fallback",
            _ => "other",
        })
    }

    fn error(&mut self, doc: &Document, n: NodeId, message: impl AsRef<str>) {
        let file = doc.url.as_deref().unwrap_or("");
        let line = doc.line(n);
        self.errors.push(format!(
            "{file}:{line}: element include: XInclude error : {}",
            message.as_ref()
        ));
    }

    /// xmlXIncludeGetProp: `{XInclude}name`, then (legacy) the old
    /// namespace, then any attribute with that local name.
    fn prop(&self, doc: &Document, el: NodeId, local: &str) -> Option<String> {
        let find = |ns: Option<&str>| {
            doc.attributes(el).find_map(|a| {
                let q = doc.name(a)?;
                let ns_ok = match ns {
                    Some(u) => q.ns.is_some_and(|s| self.names.str(s) == u),
                    None => true,
                };
                (ns_ok && self.names.str(q.local) == local)
                    .then(|| doc.value(a).unwrap_or("").to_string())
            })
        };
        find(Some(NS))
            .or_else(|| {
                if self.legacy {
                    find(Some(OLD_NS))
                } else {
                    None
                }
            })
            .or_else(|| find(None))
    }

    /// Processes one document's includes (xmlXIncludeDoProcess on its root).
    fn process_doc(&mut self, doc: Document) -> Document {
        if !self.has_xinclude(&doc) {
            return doc;
        }
        let pushed = doc.url.clone();
        if let Some(u) = &pushed {
            self.urls.push(u.clone());
        }
        let mut b = Builder::new();
        if doc.has_doctype {
            b.set_has_doctype();
        }
        for (name, uri) in doc.unparsed_entities() {
            b.add_unparsed_entity(name, uri);
        }
        let ids = ids_by_element(&doc);
        self.walk_children(&doc, doc.root(), &mut b, &ids);
        if pushed.is_some() {
            self.urls.pop();
        }
        let mut new = b.finish();
        new.url = doc.url.clone();
        new.fragment = doc.fragment;
        new.html = doc.html;
        new
    }

    fn walk_children(&mut self, doc: &Document, parent: NodeId, b: &mut Builder, ids: &Ids) {
        let children: Vec<NodeId> = doc.children(parent).collect();
        for c in children {
            self.walk(doc, c, b, ids);
        }
    }

    fn walk(&mut self, doc: &Document, n: NodeId, b: &mut Builder, ids: &Ids) {
        match self.xi_name(doc, n) {
            Some("include") if self.valid_include(doc, n) => {
                self.include(doc, n, b, ids);
                return;
            }
            Some("fallback") => {
                let in_include = doc
                    .parent(n)
                    .is_some_and(|p| self.xi_name(doc, p) == Some("include"));
                if !in_include {
                    self.error(doc, n, "fallback is not the child of an 'include'");
                }
            }
            _ => {}
        }
        copy_node(doc, n, b, ids, None, &mut |ctx_doc, child, b| {
            self.walk(ctx_doc, child, b, ids)
        });
    }

    /// xmlXIncludeTestNode for an include: no include child, at most one
    /// fallback. An invalid one is an error and an ordinary element.
    fn valid_include(&mut self, doc: &Document, n: NodeId) -> bool {
        let mut fallbacks = 0;
        let children: Vec<NodeId> = doc.children(n).collect();
        for c in children {
            match self.xi_name(doc, c) {
                Some("include") => {
                    self.error(doc, n, "include has an 'include' child");
                    return false;
                }
                Some("fallback") => fallbacks += 1,
                _ => {}
            }
        }
        if fallbacks > 1 {
            self.error(doc, n, "include has multiple fallback children");
            return false;
        }
        true
    }

    /// One include: what xmlXIncludeAddNode checks, then
    /// xmlXIncludeLoadNode and xmlXIncludeIncludeNode.
    fn include(&mut self, doc: &Document, el: NodeId, b: &mut Builder, ids: &Ids) {
        match self.load(doc, el) {
            Some(replacement) => {
                // libxml2 refuses a replacement that would give the
                // document several root elements.
                let at_top = doc.parent(el) == Some(doc.root());
                if at_top && replacement_elements(&replacement, doc) > 1 {
                    self.error(
                        doc,
                        el,
                        "XInclude error: would result in multiple root nodes",
                    );
                    copy_subtree(doc, el, b, ids);
                    return;
                }
                b.break_text();
                self.emit(doc, replacement, b, ids);
                b.break_text();
            }
            // Not replaced: the include stays as it was.
            None => copy_subtree(doc, el, b, ids),
        }
    }

    fn emit(&mut self, doc: &Document, replacement: Replacement, b: &mut Builder, ids: &Ids) {
        match replacement {
            Replacement::Nodes {
                doc: src,
                nodes,
                base,
            } => {
                // xmlXIncludeMergeEntities: the included document's
                // unparsed entities, where the including one lacks them.
                for (name, uri) in src.unparsed_entities() {
                    if doc.unparsed_entity_uri(name).is_none() {
                        b.add_unparsed_entity(name, uri);
                    }
                }
                let src_ids = ids_by_element(&src);
                for n in nodes {
                    copy_included(
                        &src,
                        n.node,
                        b,
                        &src_ids,
                        base.as_deref(),
                        doc.url.as_deref(),
                    );
                }
            }
            Replacement::Local { nodes, base } => {
                for n in nodes {
                    copy_included(doc, n, b, ids, base.as_deref(), doc.url.as_deref());
                }
            }
            Replacement::Text(t) => {
                let _ = b.text(&t, 0);
            }
            Replacement::Fallback(fb) => self.walk_children(doc, fb, b, ids),
        }
    }

    /// The replacement for an include, or None if it isn't replaced.
    fn load(&mut self, doc: &Document, el: NodeId) -> Option<Replacement> {
        let href = self.prop(doc, el, "href").unwrap_or_default();
        let xml = match self.prop(doc, el, "parse").as_deref() {
            None | Some("xml") => true,
            Some("text") => false,
            Some(other) => {
                self.error(doc, el, format!("invalid value {other} for 'parse'"));
                return None;
            }
        };
        let xml_base = Some((crate::tree::NamePool::XML_NS, crate::tree::NamePool::BASE));
        let base = doc.base_uri(el, xml_base);
        let uri = crate::uri::build(&href, base.as_deref().or(doc.url.as_deref()));
        let mut fragment = self.prop(doc, el, "xpointer");
        let (url, uri_fragment) = match uri.split_once('#') {
            Some((u, f)) => (u.to_string(), Some(f.to_string())),
            None => (uri.clone(), None),
        };
        if let Some(f) = uri_fragment {
            if self.legacy {
                fragment.get_or_insert(f);
            } else {
                self.error(
                    doc,
                    el,
                    format!("Invalid fragment identifier in URI {uri} use the xpointer attribute"),
                );
                return None;
            }
        }
        let local = doc.url.as_deref() == Some(url.as_str());
        let no_fragment = fragment.as_deref().is_none_or(str::is_empty);
        if local && xml && no_fragment {
            self.error(
                doc,
                el,
                format!("detected a local recursion with no xpointer in {url}"),
            );
            return None;
        }
        if !local && xml && self.urls.contains(&url) {
            self.error(doc, el, format!("detected a recursion in {url}"));
            return None;
        }

        let loaded = if xml {
            self.load_doc(doc, el, &url, fragment.as_deref(), base.as_deref())
        } else {
            self.load_text(doc, el, &url)
        };
        if loaded.is_some() {
            return loaded;
        }
        // A fallback, if there is one.
        let fallback = doc
            .children(el)
            .find(|&c| self.xi_name(doc, c) == Some("fallback"));
        match fallback {
            Some(fb) => Some(Replacement::Fallback(fb)),
            None => {
                self.error(
                    doc,
                    el,
                    format!("could not load {uri}, and no fallback was found"),
                );
                None
            }
        }
    }

    /// xmlXIncludeLoadDoc.
    fn load_doc(
        &mut self,
        doc: &Document,
        el: NodeId,
        url: &str,
        fragment: Option<&str>,
        include_base: Option<&str>,
    ) -> Option<Replacement> {
        let local = url.is_empty() || url.starts_with('#') || doc.url.as_deref() == Some(url);
        if local {
            let nodes = match fragment {
                None => doc.children(doc.root()).collect(),
                Some(f) => {
                    let found = self.xpointer(doc, el, doc, 0, f)?;
                    found.into_iter().map(|n| n.node).collect()
                }
            };
            let base = self.base_fixup(doc, el, url, include_base);
            return Some(Replacement::Local { nodes, base });
        }
        // Parse it, through the catalogs, with the same options.
        let path = self.loader.locate(None, url, &None)?;
        let bytes = crate::catalog::read_file(&path).ok()?;
        let parsed = match parse_with(
            &bytes,
            self.names,
            self.options,
            self.loader,
            &Some(path.clone()),
        ) {
            Ok(p) => p,
            Err(e) => {
                self.warnings.push(format!(
                    "{}:{}: parser error : {}",
                    path.display(),
                    e.line,
                    e.message
                ));
                return None;
            }
        };
        for w in &parsed.warnings {
            self.warnings.push(format!("warning: {w}"));
        }
        let mut included = parsed.document;
        // The document's URL is where it was found (a catalog may map it).
        let found = path.to_string_lossy().into_owned();
        let doc_url = if crate::catalog::local_path(url).as_deref() == Some(path.as_path()) {
            url.to_string()
        } else {
            found
        };
        included.url = Some(doc_url.clone());
        let included = self.process_doc(included);
        let nodes: Vec<NodeRef> = match fragment {
            None => included
                .children(included.root())
                .map(|n| NodeRef::new(0, n))
                .collect(),
            Some(f) => self.xpointer(doc, el, &included, 0, f)?,
        };
        let base = self.base_fixup(doc, el, &doc_url, include_base);
        Some(Replacement::Nodes {
            doc: Box::new(included),
            nodes,
            base,
        })
    }

    /// The xml:base value for the included top-level elements, if the
    /// fixup applies: the include's own xml:base, or the URL relative to
    /// the include's base when that contains a '/'.
    fn base_fixup(
        &mut self,
        doc: &Document,
        el: NodeId,
        url: &str,
        include_base: Option<&str>,
    ) -> Option<String> {
        let own = doc.attributes(el).find_map(|a| {
            let q = doc.name(a)?;
            (q.ns == Some(NamePool::XML_NS) && q.local == NamePool::BASE)
                .then(|| doc.value(a).unwrap_or("").to_string())
        });
        if own.is_some() {
            return own;
        }
        match crate::uri::build_relative(url, include_base) {
            None => {
                self.error(doc, el, format!("trying to build relative URI from {url}"));
                None
            }
            Some(rel) => rel.contains('/').then_some(rel),
        }
    }

    /// xmlXIncludeLoadTxt.
    fn load_text(&mut self, doc: &Document, el: NodeId, url: &str) -> Option<Replacement> {
        if url.is_empty() {
            self.error(doc, el, "text serialization of document not available");
            return None;
        }
        if let Some(t) = self.texts.get(url) {
            return Some(Replacement::Text(t.clone()));
        }
        let encoding = self.prop_plain(doc, el, "encoding");
        let path = self.loader.locate(None, url, &None)?;
        let bytes = crate::catalog::read_file(&path).ok()?;
        let text = match encoding.as_deref() {
            None => String::from_utf8(bytes).ok(),
            Some(e) => match crate::encoding::lookup(e) {
                Some(cs) => cs.decode(&bytes).map(|t| t.into_owned()),
                None => {
                    self.error(doc, el, format!("encoding {e} not supported"));
                    return None;
                }
            },
        };
        let Some(text) = text.filter(|t| t.chars().all(is_xml_char)) else {
            self.error(doc, el, format!("{url} contains invalid char"));
            return None;
        };
        self.texts.insert(url.to_string(), text.clone());
        Some(Replacement::Text(text))
    }

    /// xmlGetProp: any attribute with that local name (for `encoding`).
    fn prop_plain(&self, doc: &Document, el: NodeId, local: &str) -> Option<String> {
        doc.attributes(el).find_map(|a| {
            let q = doc.name(a)?;
            (self.names.str(q.local) == local).then(|| doc.value(a).unwrap_or("").to_string())
        })
    }

    /// xmlXPtrEval on `target` (document index `doc_index` in the Env):
    /// the selected nodes, or None (with an error where libxml2 gives one)
    /// if it fails or selects nothing.
    fn xpointer(
        &mut self,
        doc: &Document,
        el: NodeId,
        target: &Document,
        doc_index: u32,
        pointer: &str,
    ) -> Option<Vec<NodeRef>> {
        let found = match self.eval_xpointer(target, doc_index, pointer) {
            Ok(nodes) => nodes,
            Err(()) => {
                self.error(doc, el, format!("XPointer evaluation failed: #{pointer}"));
                return None;
            }
        };
        if found.is_empty() {
            return None;
        }
        let mut kept = Vec::new();
        for n in found {
            if n.ns != 0 {
                self.error(doc, el, format!("XPointer selects a namespace: #{pointer}"));
            } else if matches!(target.kind(n.node), NodeKind::Attribute { .. }) {
                self.error(
                    doc,
                    el,
                    format!("XPointer selects an attribute: #{pointer}"),
                );
            } else {
                kept.push(n);
            }
        }
        Some(kept)
    }

    fn eval_xpointer(
        &mut self,
        target: &Document,
        doc_index: u32,
        pointer: &str,
    ) -> Result<Vec<NodeRef>, ()> {
        let p = pointer.trim_start_matches([' ', '\t', '\n', '\r']);
        let root = NodeRef::new(doc_index, target.root());
        if p.starts_with('/') {
            return child_sequence(target, vec![root], p).ok_or(());
        }
        let name_end = p
            .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')))
            .unwrap_or(p.len());
        if name_end == 0 {
            return Err(());
        }
        if !p[name_end..].starts_with('(') {
            // A bare name (an ID) or a child sequence from an ID.
            let (name, rest) = p.split_at(name_end);
            let start = target
                .element_by_id(name)
                .map(|n| NodeRef::new(doc_index, n));
            let nodes = child_sequence(target, start.into_iter().collect(), rest).ok_or(())?;
            return Ok(nodes);
        }
        // A full XPointer: parts tried in turn until one selects nodes.
        let mut namespaces: Vec<(String, String)> = Vec::new();
        let mut rest = p;
        while !rest.is_empty() {
            let end = rest
                .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')))
                .unwrap_or(rest.len());
            let scheme = &rest[..end];
            if scheme.is_empty() || !rest[end..].starts_with('(') {
                return Err(());
            }
            let (body, after) = xpointer_part(&rest[end + 1..]).ok_or(())?;
            match scheme {
                "xpointer" => {
                    let ns = namespaces.clone();
                    let names = &mut *self.names;
                    let syms: Vec<(String, Sym)> = ns
                        .iter()
                        .map(|(p, u)| (p.clone(), names.intern(u)))
                        .collect();
                    let resolve = move |pre: &str| {
                        if pre == "xml" {
                            return Some(NamePool::XML_NS);
                        }
                        syms.iter().find(|(q, _)| q == pre).map(|&(_, u)| u)
                    };
                    let expr = xpath::compile(&body, self.names, &resolve).map_err(|_| ())?;
                    let docs = std::slice::from_ref(target);
                    let env = Env {
                        docs,
                        names: self.names,
                        vars: &Empty,
                        funcs: &Empty,
                    };
                    let focus = Focus::single(NodeRef::new(0, target.root()));
                    match xpath::evaluate(&expr, &env, &focus) {
                        Ok(Value::NodeSet(nodes)) if !nodes.is_empty() => {
                            return Ok(nodes
                                .into_iter()
                                .map(|n| NodeRef {
                                    doc: doc_index,
                                    ..n
                                })
                                .collect());
                        }
                        Ok(_) => {}
                        Err(_) => return Err(()),
                    }
                }
                "element" => {
                    let nodes = if body.starts_with('/') {
                        child_sequence(target, vec![root], &body)
                    } else {
                        let end = body.find('/').unwrap_or(body.len());
                        let start = target
                            .element_by_id(&body[..end])
                            .map(|n| NodeRef::new(doc_index, n));
                        child_sequence(target, start.into_iter().collect(), &body[end..])
                    };
                    match nodes {
                        Some(n) if !n.is_empty() => return Ok(n),
                        Some(_) => {}
                        None => return Err(()),
                    }
                }
                "xmlns" => {
                    let (prefix, uri) = body.split_once('=').ok_or(())?;
                    namespaces.push((prefix.trim().to_string(), uri.trim_start().to_string()));
                }
                _ => {} // An unknown scheme: try the next part.
            }
            rest = after.trim_start_matches([' ', '\t', '\n', '\r']);
        }
        Ok(Vec::new())
    }
}

/// An XPointer part's body up to its balanced ')', with '^' escapes
/// removed, and what follows it.
fn xpointer_part(s: &str) -> Option<(String, &str)> {
    let mut level = 1;
    let mut out = String::new();
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            ')' => {
                level -= 1;
                if level == 0 {
                    return Some((out, &s[i + 1..]));
                }
            }
            '(' => level += 1,
            '^' => {
                if let Some(&(_, n)) = chars.peek()
                    && matches!(n, '(' | ')' | '^')
                {
                    out.push(n);
                    chars.next();
                    continue;
                }
            }
            _ => {}
        }
        out.push(c);
    }
    None
}

/// An XPointer child sequence (`/1/3`) from each start node: the nth
/// element child each time. None if the syntax is wrong.
fn child_sequence(doc: &Document, mut nodes: Vec<NodeRef>, seq: &str) -> Option<Vec<NodeRef>> {
    let mut rest = seq;
    while let Some(r) = rest.strip_prefix('/') {
        let end = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
        let n: usize = r[..end].parse().unwrap_or(0);
        nodes = nodes
            .into_iter()
            .filter_map(|cur| {
                let child = doc
                    .children(cur.node)
                    .filter(|&c| matches!(doc.kind(c), NodeKind::Element { .. }))
                    .nth(n.checked_sub(1)?)?;
                Some(NodeRef { node: child, ..cur })
            })
            .collect();
        rest = &r[end..];
    }
    rest.trim().is_empty().then_some(nodes)
}

fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

type Ids = HashMap<NodeId, Vec<String>>;

/// A document's IDs, by element.
fn ids_by_element(doc: &Document) -> Ids {
    let mut ids: Ids = HashMap::new();
    for (value, el) in doc.ids() {
        ids.entry(el).or_default().push(value.to_string());
    }
    ids
}

/// How many elements a replacement has at its top level.
fn replacement_elements(r: &Replacement, doc: &Document) -> usize {
    let is_el = |d: &Document, n: NodeId| matches!(d.kind(n), NodeKind::Element { .. });
    match r {
        Replacement::Nodes {
            doc: src, nodes, ..
        } => nodes.iter().filter(|n| is_el(src, n.node)).count(),
        Replacement::Local { nodes, .. } => nodes.iter().filter(|&&n| is_el(doc, n)).count(),
        Replacement::Text(_) => 0,
        Replacement::Fallback(fb) => doc.children(*fb).filter(|&c| is_el(doc, c)).count(),
    }
}

/// Copies one node; an element's children go through `children`, so the
/// caller decides whether includes inside are processed. `base` replaces
/// or adds the element's xml:base (the fixup).
fn copy_node(
    doc: &Document,
    n: NodeId,
    b: &mut Builder,
    ids: &Ids,
    base: Option<&str>,
    children: &mut dyn FnMut(&Document, NodeId, &mut Builder),
) {
    match doc.kind(n) {
        NodeKind::Element { name, .. } => {
            let _ = b.start_element(name, doc.line(n));
            for d in doc.ns_decls(n) {
                b.add_ns_decl(*d);
            }
            let xml_base = QName {
                ns: Some(NamePool::XML_NS),
                local: NamePool::BASE,
                prefix: Some(NamePool::XML),
            };
            let mut base_done = base.is_none();
            for a in doc.attributes(n) {
                let q = doc.name(a).unwrap();
                match base {
                    Some(v) if q.expanded() == xml_base.expanded() => {
                        let _ = b.add_attribute(q, v);
                        base_done = true;
                    }
                    _ => {
                        let _ = b.add_attribute(q, doc.value(a).unwrap_or(""));
                    }
                }
            }
            if !base_done && let Some(v) = base {
                let _ = b.add_attribute(xml_base, v);
            }
            for value in ids.get(&n).into_iter().flatten() {
                b.add_id(value);
            }
            let kids: Vec<NodeId> = doc.children(n).collect();
            for c in kids {
                children(doc, c, b);
            }
            b.end_element();
        }
        NodeKind::Text(_) => {
            let _ = b.text_of_kind(doc.value(n).unwrap_or(""), doc.text_kind(n), doc.line(n));
        }
        NodeKind::Comment(_) => {
            let _ = b.comment(doc.value(n).unwrap_or(""), doc.line(n));
        }
        NodeKind::Pi { target, .. } => {
            let _ = b.pi(target, doc.value(n).unwrap_or(""), doc.line(n));
        }
        NodeKind::Root => {
            let kids: Vec<NodeId> = doc.children(n).collect();
            for c in kids {
                children(doc, c, b);
            }
        }
        NodeKind::Attribute { .. } => {}
    }
}

/// Copies a subtree as it is.
fn copy_subtree(doc: &Document, n: NodeId, b: &mut Builder, ids: &Ids) {
    copy_node(doc, n, b, ids, None, &mut |d, c, b| {
        copy_subtree(d, c, b, ids)
    });
}

/// Copies an included node (xmlXIncludeCopyNode): a document node is its
/// children; a top-level element gets the xml:base fixup, combined with
/// its own xml:base as libxml2 does.
fn copy_included(
    src: &Document,
    n: NodeId,
    b: &mut Builder,
    ids: &Ids,
    base: Option<&str>,
    target_url: Option<&str>,
) {
    if matches!(src.kind(n), NodeKind::Root) {
        let kids: Vec<NodeId> = src.children(n).collect();
        for c in kids {
            copy_included(src, c, b, ids, base, target_url);
        }
        return;
    }
    let fixed = match (base, src.kind(n)) {
        (Some(base), NodeKind::Element { .. }) => {
            let own = src.attributes(n).find_map(|a| {
                let q = src.name(a)?;
                (q.ns == Some(NamePool::XML_NS) && q.local == NamePool::BASE)
                    .then(|| src.value(a).unwrap_or("").to_string())
            });
            // xmlNodeGetBase of the unattached copy: its own xml:base
            // against the including document's URL, or that URL.
            match own {
                None => Some(base.to_string()),
                Some(v) => {
                    let current = match target_url {
                        Some(u) => crate::uri::build(&v, Some(u)),
                        None => v.clone(),
                    };
                    if Some(current.as_str()) == target_url {
                        Some(base.to_string())
                    } else {
                        Some(crate::uri::build(&v, Some(base)))
                    }
                }
            }
        }
        _ => None,
    };
    copy_node(src, n, b, ids, fixed.as_deref(), &mut |d, c, b| {
        copy_subtree(d, c, b, ids)
    });
}
