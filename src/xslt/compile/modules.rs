//! Loading stylesheet modules: a module's top level, xsl:include and
//! xsl:import, and the namespace exclusions and aliases that apply within
//! a module.

use super::*;

impl Compiler<'_> {
    /// Namespace URIs excluded from literal result elements at `el`: the XSLT
    /// namespace, plus `exclude-result-prefixes` and `extension-element-prefixes`
    /// on `el`, its ancestors and the `xsl:stylesheet` element.
    pub(super) fn excluded(&self, d: &Doc, el: NodeId) -> HashSet<Sym> {
        self.excluded_uris(d, el, true)
    }

    /// The excluded URIs; `with_extensions` adds the extension namespaces'
    /// URIs (for an element's own declarations).
    pub(super) fn excluded_uris(&self, d: &Doc, el: NodeId, with_extensions: bool) -> HashSet<Sym> {
        let mut out = HashSet::from([self.xslt]);
        out.extend(self.leaked_exclusions.iter().copied());
        let attrs: &[&str] = if with_extensions {
            &["exclude-result-prefixes", "extension-element-prefixes"]
        } else {
            &["exclude-result-prefixes"]
        };
        let mut cur = Some(el);
        while let Some(e) = cur {
            if matches!(d.doc.kind(e), NodeKind::Element { .. }) {
                let scope = self.in_scope(d.doc, e);
                let is_xslt = self.is_xslt(d.doc, e);
                for &attr in attrs {
                    let v = if is_xslt {
                        self.attr(d.doc, e, attr)
                    } else {
                        self.xsl_attr(d.doc, e, attr)
                    };
                    for p in v.unwrap_or("").split_ascii_whitespace() {
                        let uri = if p == "#default" {
                            scope.iter().find(|(q, _)| q.is_none()).map(|&(_, u)| u)
                        } else {
                            self.resolve_prefix(&scope, p)
                        };
                        out.extend(uri);
                    }
                }
            }
            cur = d.doc.parent(e);
        }
        out
    }

    /// `xsl:namespace-alias` as libxslt applies it (xsltGetNamespace,
    /// xsltCopyNamespaceList): the URI is replaced, the stylesheet's prefix
    /// is kept. An alias to `#default` with no default namespace means none.
    pub(super) fn alias(&self, ns: Option<Sym>, prefix: Option<Sym>) -> (Option<Sym>, Option<Sym>) {
        match ns.and_then(|u| self.aliases.get(&u)) {
            Some(&(_, uri)) => {
                if self.names.str(uri).is_empty() {
                    (None, None)
                } else {
                    (Some(uri), prefix)
                }
            }
            None => (ns, prefix),
        }
    }

    /// Compiles one stylesheet document, into the current module.
    pub(super) fn stylesheet(
        &mut self,
        doc: &Document,
        file: &Path,
    ) -> Result<(), StylesheetError> {
        let display = file.file_name().map_or_else(
            || file.display().to_string(),
            |f| f.to_string_lossy().into_owned(),
        );
        let root = doc
            .children(doc.root())
            .find(|&c| matches!(doc.kind(c), NodeKind::Element { .. }));
        let Some(root) = root else {
            return Err(StylesheetError::Parse("no document element".into()));
        };
        let version = if self.is_xslt(doc, root) {
            self.attr(doc, root, "version")
        } else {
            self.xsl_attr(doc, root, "version")
        };
        let forwards_compatible = version.is_some_and(|v| v.trim() != "1.0");
        let url: std::rc::Rc<str> = doc
            .url
            .clone()
            .unwrap_or_else(|| file.to_string_lossy().into_owned())
            .into();
        let xml_base = Some(NamePool::BASE);
        let has_xml_base = xml_base.is_some()
            && doc.all_nodes().any(|n| {
                doc.name(n).is_some_and(|q| {
                    q.ns == Some(self.xml)
                        && Some(q.local) == xml_base
                        && matches!(doc.kind(n), NodeKind::Attribute { .. })
                })
            });
        let d = Doc {
            doc,
            file: display.into(),
            path: file.to_path_buf(),
            forwards_compatible,
            url,
            has_xml_base,
            scope_memo: Default::default(),
            space_memo: Default::default(),
        };

        // The module's own exclusions join the import unit's list
        // (xsltParseStylesheetExcludePrefix on the document element).
        let own = if self.is_xslt(doc, root) {
            self.attr(doc, root, "exclude-result-prefixes")
        } else {
            self.xsl_attr(doc, root, "exclude-result-prefixes")
        };
        if let Some(prefixes) = own {
            let scope = self.in_scope(doc, root);
            for p in prefixes.split_ascii_whitespace() {
                let uri = if p == "#default" {
                    scope.iter().find(|(q, _)| q.is_none()).map(|&(_, u)| u)
                } else {
                    self.resolve_prefix(&scope, p)
                };
                self.leaked_exclusions.extend(uri);
            }
        }

        if !self.is_xslt(doc, root) {
            // A simplified stylesheet: the literal result element is the
            // body of a template matching "/".
            if version.is_none() {
                self.errors.push(self.error(
                    &d,
                    root,
                    "The document element isn't xsl:stylesheet and has no xsl:version",
                ));
                return Ok(());
            }
            match self.literal_element(&d, root) {
                Ok(instr) => {
                    let expr = Expr::Path(xpath::ast::Path {
                        start: xpath::ast::PathStart::Root,
                        steps: vec![],
                    });
                    let matches = pattern::from_expr(expr, false).ok();
                    self.add_template(
                        Template {
                            matches,
                            name: None,
                            mode: None,
                            params: vec![],
                            body: vec![instr],
                            location: self.location(&d, root),
                            precedence: 0,
                        },
                        None,
                    );
                }
                Err(e) => self.errors.push(e),
            }
            return Ok(());
        }
        let local = self.local(doc, root).unwrap_or("");
        if local != "stylesheet" && local != "transform" {
            self.errors.push(self.error(
                &d,
                root,
                format!("Unexpected document element xsl:{local}"),
            ));
            return Ok(());
        }

        // Aliases apply to the whole stylesheet, wherever they're declared.
        for child in doc.children(root) {
            if self.is_xslt_named(doc, child, "namespace-alias")
                && let Err(e) = self.namespace_alias(&d, child)
            {
                self.errors.push(e);
            }
        }
        // Imports come first. An included file's imports become imports of
        // the including module, after the ones it already has, as in
        // libxslt.
        let children: Vec<NodeId> = doc.children(root).collect();
        let (imports, rest): (Vec<NodeId>, Vec<NodeId>) = children
            .iter()
            .partition(|&&c| self.is_xslt_named(doc, c, "import"));
        let mut seen = false;
        for &child in &children {
            if imports.contains(&child) {
                if seen {
                    self.errors.push(self.error(
                        &d,
                        child,
                        "xsl:import must come before any other top-level element",
                    ));
                } else if let Err(e) = self.import(&d, child) {
                    self.errors.push(e);
                }
            } else if matches!(doc.kind(child), NodeKind::Element { .. }) {
                seen = true;
            }
        }
        for child in rest {
            if let Err(e) = self.top_level(&d, child) {
                self.errors.push(e);
            }
        }
        Ok(())
    }

    pub(super) fn include(&mut self, d: &Doc, el: NodeId) -> R<()> {
        self.load_module(d, el, true)
    }

    /// `xsl:import`: a module of its own, with lower precedence than this
    /// one.
    fn import(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let saved = self.precedence;
        let module = self.modules.len();
        self.modules.push(Vec::new());
        self.modules[saved].push(module);
        self.precedence = module;
        let saved_exclusions = std::mem::take(&mut self.leaked_exclusions);
        let result = self.load_module(d, el, false);
        self.leaked_exclusions = saved_exclusions;
        self.precedence = saved;
        result
    }

    fn load_module(&mut self, d: &Doc, el: NodeId, included: bool) -> R<()> {
        let what = if included { "inclusion" } else { "import" };
        let href = self.required(d, el, "href")?;
        // Through the loader, so catalogs can map a URL to a local file.
        let (bytes, path, by_path) = self
            .loader
            .read_document_how(href, &Some(d.path.clone()))
            .map_err(|e| self.error(d, el, format!("Failed to load '{href}': {e}")))?;
        if self.including.contains(&path) {
            return Err(self.error(d, el, format!("Recursive {what} of '{href}'")));
        }
        let parsed = parse_with(
            &bytes,
            self.names,
            self.options,
            self.loader,
            &Some(path.clone()),
        )
        .map_err(|e| self.error(d, el, format!("Failed to parse '{href}': {e}")))?;
        self.warnings.extend(parsed.warnings);
        let mut doc = parsed.document;
        let base = self.location(d, el).base;
        // The URL as requested, so relative ones stay relative; but a
        // module found in a --path directory is named by the path found, as
        // libxml2 names it, and its own includes resolve from there.
        doc.url = Some(if by_path {
            path.to_string_lossy().into_owned()
        } else {
            crate::uri::build(href, Some(&base))
        });
        self.including.push(path.clone());
        let result = self.stylesheet(&doc, &path);
        self.including.pop();
        match result {
            Ok(()) => Ok(()),
            Err(StylesheetError::Parse(m)) => {
                Err(self.error(d, el, format!("Failed to parse '{href}': {m}")))
            }
            Err(StylesheetError::Compile(mut errs)) => {
                let first = errs.remove(0);
                self.errors.extend(errs);
                Err(first)
            }
        }
    }
}
