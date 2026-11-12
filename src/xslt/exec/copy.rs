//! Copying source nodes into the result: xsl:copy and xsl:copy-of, with
//! namespace nodes as libxslt copies them.

use super::*;

impl<'a> Transform<'a> {
    /// `xsl:copy`: a shallow copy of the context node.
    pub(super) fn copy(&mut self, focus: &Focus, sets: &[Name], body: &Body) -> R<()> {
        let n = focus.node;
        if n.ns > 0 {
            let binding = self.namespace_node(n);
            self.out.last_mut().unwrap().add_namespace(binding);
            return Ok(());
        }
        let doc = &self.docs[n.doc as usize];
        match doc.kind(n.node) {
            NodeKind::Root => self.body(body, focus),
            NodeKind::Element { name, .. } => {
                let decls: Vec<Binding> = doc
                    .ns_decls(n.node)
                    .iter()
                    .map(|d| (d.prefix, d.uri))
                    .collect();
                let (out, names) = self.out();
                out.start_element(name, names).map_err(Self::output_err)?;
                for b in decls {
                    out.add_namespace(b);
                }
                self.attribute_sets(sets, focus)?;
                self.body(body, focus)?;
                let (out, names) = self.out();
                out.end_element(names).map_err(Self::output_err)
            }
            _ => self.copy_of(n, false),
        }
    }

    fn namespace_node(&self, n: NodeRef) -> Binding {
        let vars = Vars {
            locals: &[],
            globals: &self.globals,
            lazy: None,
        };
        let funcs = XsltFunctions {
            current: n,
            budget: &self.budget,
            ids: &self.ids,
            keys: &self.keys,
            loads: &self.loads,
            made: &self.made,
            serial: 0,
            token: self.token,
            base: "",
            namespaces: &[],
            ss: self.ss,
            warnings: &self.warnings,
            wanted_globals: &self.wanted_globals,
            dyn_exprs: &self.dyn_exprs,
        };
        let env = Env {
            docs: &self.docs,
            names: self.names,
            vars: &vars,
            funcs: &funcs,
        };
        env.namespaces(n)[n.ns as usize - 1]
    }

    /// `xsl:copy-of` of one node. `top`: the node itself was selected, so an
    /// element copies all its in-scope namespaces, not just its own declarations.
    pub(super) fn copy_of(&mut self, n: NodeRef, top: bool) -> R<()> {
        self.tick(1)?;
        if n.ns > 0 {
            let binding = self.namespace_node(n);
            self.out.last_mut().unwrap().add_namespace(binding);
            return Ok(());
        }
        let d = n.doc as usize;
        match self.docs[d].kind(n.node) {
            NodeKind::Root => {
                let children: Vec<NodeId> = self.docs[d].children(n.node).collect();
                for c in children {
                    self.copy_of(NodeRef::new(n.doc, c), false)?;
                }
                Ok(())
            }
            NodeKind::Element { name, .. } => {
                let bindings: Vec<Binding> = if top {
                    // xmlGetNsList's order: the element's own declarations,
                    // then each ancestor's new ones.
                    let doc = &self.docs[d];
                    let mut seen = Vec::new();
                    let mut list = Vec::new();
                    let mut cur = Some(n.node);
                    while let Some(e) = cur {
                        for x in doc.ns_decls(e) {
                            if !seen.contains(&x.prefix) {
                                seen.push(x.prefix);
                                list.push((x.prefix, x.uri));
                            }
                        }
                        cur = doc.parent(e);
                    }
                    list
                } else {
                    self.docs[d]
                        .ns_decls(n.node)
                        .iter()
                        .map(|x| (x.prefix, x.uri))
                        .collect()
                };
                let attrs: Vec<(QName, String, bool)> = self.docs[d]
                    .attributes(n.node)
                    .map(|a| {
                        (
                            self.docs[d].name(a).unwrap(),
                            self.docs[d].value(a).unwrap_or("").to_string(),
                            self.docs[d].is_valueless(a),
                        )
                    })
                    .collect();
                let children: Vec<NodeId> = self.docs[d].children(n.node).collect();
                let (out, names) = self.out();
                out.start_element(name, names).map_err(Self::output_err)?;
                for b in bindings {
                    out.add_namespace(b);
                }
                for (q, v, valueless) in attrs {
                    if valueless {
                        out.add_valueless_attribute(q).map_err(Self::output_err)?;
                    } else {
                        out.add_attribute(q, v).map_err(Self::output_err)?;
                    }
                }
                for c in children {
                    self.copy_of(NodeRef::new(n.doc, c), false)?;
                }
                let (out, names) = self.out();
                out.end_element(names).map_err(Self::output_err)
            }
            NodeKind::Attribute { name, .. } => {
                let value = self.docs[d].value(n.node).unwrap_or("").to_string();
                let out = self.out.last_mut().unwrap();
                let added = if self.docs[d].is_valueless(n.node) {
                    out.add_valueless_attribute(name)
                } else {
                    out.add_attribute(name, value)
                };
                if added.is_err() {
                    self.report_plain(
                        "runtime error: Attribute nodes must be added before any child nodes to an element.",
                    );
                }
                Ok(())
            }
            // The source documents and the output are separate fields, so
            // text is copied straight from one to the other.
            NodeKind::Text(_) => {
                let doc = &self.docs[d];
                let s = doc.value(n.node).unwrap_or("");
                // Unescaped text in a fragment stays unescaped when copied.
                let unescaped = doc.text_kind(n.node) == crate::tree::TextKind::Unescaped;
                let out = self.out.last_mut().expect("an output is open");
                out.text(s, unescaped, self.names).map_err(Self::output_err)
            }
            NodeKind::Comment(_) => {
                let s = self.docs[d].value(n.node).unwrap_or("");
                let out = self.out.last_mut().expect("an output is open");
                out.comment(s, self.names).map_err(Self::output_err)
            }
            NodeKind::Pi { target, .. } => {
                let s = self.docs[d].value(n.node).unwrap_or("");
                let out = self.out.last_mut().expect("an output is open");
                out.pi(target, s, self.names).map_err(Self::output_err)
            }
        }
    }
}
