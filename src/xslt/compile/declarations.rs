//! Top-level elements: templates, global variables and parameters, keys,
//! attribute sets, xsl:output, decimal formats, space rules and namespace
//! aliases.

use super::*;

impl Compiler<'_> {
    pub(super) fn namespace_alias(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let scope = self.in_scope(d.doc, el);
        let lookup = |c: &Self, p: &str| -> Option<(Option<Sym>, Sym)> {
            if p == "#default" {
                let uri = scope
                    .iter()
                    .find(|(q, _)| q.is_none())
                    .map_or_else(|| c.names.get("").unwrap_or(c.xslt), |&(_, u)| u);
                Some((None, uri))
            } else {
                let uri = c.resolve_prefix(&scope, p)?;
                let prefix = scope
                    .iter()
                    .find(|(q, u)| *u == uri && q.is_some_and(|q| c.names.str(q) == p))
                    .and_then(|&(q, _)| q);
                Some((prefix, uri))
            }
        };
        let from = self.required(d, el, "stylesheet-prefix")?;
        let to = self.required(d, el, "result-prefix")?;
        let (_, from_uri) = lookup(self, from)
            .ok_or_else(|| self.error(d, el, format!("Undefined namespace prefix {from}")))?;
        let (to_prefix, to_uri) = if to == "#default" && !scope.iter().any(|(q, _)| q.is_none()) {
            (None, self.names.intern(""))
        } else {
            lookup(self, to)
                .ok_or_else(|| self.error(d, el, format!("Undefined namespace prefix {to}")))?
        };
        self.aliases.insert(from_uri, (to_prefix, to_uri));
        Ok(())
    }

    pub(super) fn top_level(&mut self, d: &Doc, el: NodeId) -> R<()> {
        if !matches!(d.doc.kind(el), NodeKind::Element { .. }) {
            if let NodeKind::Text(_) = d.doc.kind(el)
                && !d.doc.value(el).unwrap_or("").trim().is_empty()
            {
                return Err(self.error(
                    d,
                    el,
                    "Text isn't allowed at the top level of a stylesheet",
                ));
            }
            return Ok(());
        }
        if !self.is_xslt(d.doc, el) {
            // Top-level elements in other namespaces are ignored; with no
            // namespace, libxslt rejects them.
            if d.doc.name(el).is_some_and(|q| q.ns.is_none()) {
                let name = d.doc.name(el).map(|q| self.names.str(q.local).to_string());
                return Err(self.error(
                    d,
                    el,
                    format!(
                        "Found a top-level element {} with null namespace URI",
                        name.unwrap_or_default()
                    ),
                ));
            }
            return Ok(());
        }
        self.order += 1;
        let local = self.local(d.doc, el).unwrap_or("").to_string();
        match local.as_str() {
            "template" => self.template(d, el),
            "variable" | "param" => {
                if local == "variable"
                    && let Some(name) = self.attr(d.doc, el, "name")
                {
                    let name = self.qname(d, el, name)?;
                    self.ss.global_variable_names.insert(name);
                    // libxslt's xsltRegisterGlobalVariable: two xsl:variables
                    // of one name in a stylesheet and its includes are an
                    // error; params, or a param and a variable, aren't.
                    if !self.global_variables.insert((self.precedence, name)) {
                        let shown = self.attr(d.doc, el, "name").unwrap_or("").to_string();
                        return Err(self.error(
                            d,
                            el,
                            format!("redefinition of global variable {shown}"),
                        ));
                    }
                }
                let v = self.variable(d, el)?;
                self.ss.globals.push(v);
                self.global_modules.push(self.precedence);
                Ok(())
            }
            "output" => self.output(d, el),
            "strip-space" | "preserve-space" => self.space(d, el, local == "strip-space"),
            "include" => self.include(d, el),
            "attribute-set" => self.attribute_set(d, el),
            "decimal-format" => self.decimal_format(d, el),
            "namespace-alias" => Ok(()),
            "import" => Ok(()), // compiled first, by stylesheet()
            "key" => self.key(d, el),
            // Even in forwards-compatible mode: libxslt 1.1.35's
            // xsltParseStylesheetTop counts an unknown top-level XSLT element
            // as an error, unlike an unknown instruction in a template.
            _ => Err(self.error(
                d,
                el,
                format!("xsltParseStylesheetTop: unknown {local} element"),
            )),
        }
    }

    pub(super) fn add_template(&mut self, mut t: Template, priority: Option<f64>) {
        t.precedence = self.precedence;
        let index = self.ss.templates.len();
        if let Some(p) = &t.matches {
            let rules = self.ss.rules.entry(t.mode).or_default();
            for (alternative, alt) in p.alternatives.iter().enumerate() {
                rules.push(Rule {
                    template: index,
                    alternative,
                    priority: priority.unwrap_or(alt.default_priority),
                    order: self.order,
                    precedence: self.precedence,
                });
            }
        }
        self.ss.templates.push(t);
    }

    fn template(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let matches = match self.attr(d.doc, el, "match") {
            Some(m) => {
                let expr = self.pattern_xpath(d, el, m)?;
                Some(
                    pattern::from_expr(expr, false)
                        .map_err(|e| self.error(d, el, format!("{} '{m}'", e.0)))?,
                )
            }
            None => None,
        };
        let name = match self.attr(d.doc, el, "name") {
            Some(n) => Some(self.qname(d, el, n)?),
            None => None,
        };
        if matches.is_none() && name.is_none() {
            return Err(self.error(d, el, "xsl:template needs a match or name attribute"));
        }
        let mode = match self.attr(d.doc, el, "mode") {
            Some(m) => Some(self.qname(d, el, m)?),
            None => None,
        };
        let priority = match self.attr(d.doc, el, "priority") {
            Some(p) => {
                let n = xpath::number::from_string(p);
                if n.is_nan() {
                    return Err(self.error(d, el, format!("Invalid priority '{p}'")));
                }
                Some(n)
            }
            None => None,
        };
        let mut params = Vec::new();
        let mut rest = Vec::new();
        let mut in_params = true;
        for child in d.doc.children(el) {
            if in_params && self.is_xslt_named(d.doc, child, "param") {
                params.push(self.variable(d, child)?);
                continue;
            }
            if matches!(d.doc.kind(child), NodeKind::Element { .. })
                || !self.ignorable_text(d, child, el)
            {
                in_params = false;
            }
            rest.push(child);
        }
        let body = self.nodes(d, el, &rest)?;
        let location = self.location(d, el);
        self.add_template(
            Template {
                matches,
                name,
                mode,
                params,
                body,
                location,
                precedence: 0,
            },
            priority,
        );
        Ok(())
    }

    pub(super) fn variable(&mut self, d: &Doc, el: NodeId) -> R<Variable> {
        let name = self.required(d, el, "name")?;
        let name = self.qname(d, el, name)?;
        let value = match self.attr(d.doc, el, "select") {
            Some(s) => VarValue::Select(self.xpath(d, el, s)?),
            None => {
                let body = self.body(d, el)?;
                if body.is_empty() {
                    VarValue::Empty
                } else {
                    VarValue::Body(body)
                }
            }
        };
        Ok(Variable {
            name,
            value,
            location: self.location(d, el),
        })
    }

    /// `xsl:decimal-format`, as libxslt's xsltParseStylesheetDecimalFormat:
    /// an unnamed one changes the default format; a second one with the same
    /// name is ignored.
    fn decimal_format(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let mut format = match self.attr(d.doc, el, "name") {
            Some(n) => {
                let name = self.qname(d, el, n)?;
                if self.ss.decimal_formats.contains_key(&name) {
                    return Ok(());
                }
                Some((name, crate::xslt::number_format::DecimalFormat::default()))
            }
            None => None,
        };
        // libxslt's format-number uses only the main stylesheet's unnamed
        // format (style->decimalFormat): an imported one is ignored, even in
        // the imported module's own templates. A named format is looked up
        // through the imports, and its first declaration wins.
        if format.is_none() && self.precedence != 0 {
            return Ok(());
        }
        let target = match &mut format {
            Some((_, f)) => f,
            None => &mut self.ss.decimal_format,
        };
        let doc = d.doc;
        let xslt = self.xslt;
        let names = &*self.names;
        let get = |local: &str| {
            doc.attributes(el).find_map(|a| {
                let q = doc.name(a)?;
                (q.ns.is_none() && names.str(q.local) == local && Some(q.ns) != Some(Some(xslt)))
                    .then(|| doc.value(a).unwrap_or("").to_string())
            })
        };
        for (attr, field) in [
            ("decimal-separator", &mut target.decimal_point),
            ("grouping-separator", &mut target.grouping),
            ("infinity", &mut target.infinity),
            ("minus-sign", &mut target.minus_sign),
            ("NaN", &mut target.nan),
            ("percent", &mut target.percent),
            ("per-mille", &mut target.per_mille),
            ("zero-digit", &mut target.zero_digit),
            ("digit", &mut target.digit),
            ("pattern-separator", &mut target.pattern_separator),
        ] {
            if let Some(v) = get(attr) {
                *field = v;
            }
        }
        if let Some((name, f)) = format {
            self.ss.decimal_formats.insert(name, f);
        }
        Ok(())
    }

    fn output(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let doc = d.doc;
        if let Some(m) = self.attr(doc, el, "method") {
            self.ss.output.method = match m.trim() {
                "xml" => Method::Xml,
                "html" => Method::Html,
                "text" => Method::Text,
                // A QName method is an extension; treat it as xml.
                other if other.contains(':') => Method::Xml,
                other => {
                    return Err(self.error(d, el, format!("Unsupported output method '{other}'")));
                }
            };
        }
        let text = |c: &Self, name: &str| c.attr(doc, el, name).map(str::to_string);
        let flag = |c: &Self, name: &str| c.attr(doc, el, name).map(|v| v.trim() == "yes");
        // Each attribute overrides an earlier xsl:output's, independently.
        let o = Output {
            version: text(self, "version"),
            encoding: text(self, "encoding"),
            doctype_public: text(self, "doctype-public"),
            doctype_system: text(self, "doctype-system"),
            media_type: text(self, "media-type"),
            omit_xml_declaration: flag(self, "omit-xml-declaration"),
            standalone: flag(self, "standalone"),
            indent: flag(self, "indent"),
            ..Output::default()
        };
        // libxslt (the fix for bug #339570) expands an unprefixed name with the
        // default namespace in effect on xsl:output, unlike other QNames.
        let cdata_attr = self.attr(doc, el, "cdata-section-elements");
        let mut cdata = self.qname_list(d, el, cdata_attr)?;
        let default_ns = self
            .in_scope(doc, el)
            .into_iter()
            .find(|(p, _)| p.is_none())
            .map(|(_, u)| u);
        if let Some(uri) = default_ns.filter(|&u| !self.names.str(u).is_empty()) {
            for (raw, name) in cdata_attr
                .unwrap_or("")
                .split_ascii_whitespace()
                .zip(cdata.iter_mut())
            {
                if !raw.contains(':') {
                    name.0 = Some(uri);
                }
            }
        }
        let out = &mut self.ss.output;
        out.version = o.version.or(out.version.take());
        out.encoding = o.encoding.or(out.encoding.take());
        out.doctype_public = o.doctype_public.or(out.doctype_public.take());
        out.doctype_system = o.doctype_system.or(out.doctype_system.take());
        out.media_type = o.media_type.or(out.media_type.take());
        out.omit_xml_declaration = o.omit_xml_declaration.or(out.omit_xml_declaration);
        out.standalone = o.standalone.or(out.standalone);
        out.indent = o.indent.or(out.indent);
        let module = self.precedence;
        self.cdata_modules
            .extend(cdata.into_iter().map(|name| (module, name)));
        Ok(())
    }

    fn space(&mut self, d: &Doc, el: NodeId, strip: bool) -> R<()> {
        let elements = self.required(d, el, "elements")?;
        for token in elements.split_ascii_whitespace() {
            let test = if token == "*" {
                SpaceTest::Any
            } else if let Some(prefix) = token.strip_suffix(":*") {
                let scope = self.in_scope(d.doc, el);
                let uri = self.resolve_prefix(&scope, prefix).ok_or_else(|| {
                    self.error(d, el, format!("Undefined namespace prefix {prefix}"))
                })?;
                SpaceTest::Namespace(Some(uri))
            } else {
                SpaceTest::Name(self.qname(d, el, token)?)
            };
            self.ss.space.push(SpaceRule {
                strip,
                test,
                order: self.order,
                precedence: self.precedence,
            });
        }
        Ok(())
    }

    fn key(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let name = self.required(d, el, "name")?;
        let name = self.qname(d, el, name)?;
        let m = self.required(d, el, "match")?;
        let expr = self.pattern_xpath(d, el, m)?;
        let matches = pattern::from_expr(expr, false)
            .map_err(|e| self.error(d, el, format!("{} '{m}'", e.0)))?;
        let use_text = self.required(d, el, "use")?;
        let use_ = self.xpath(d, el, use_text)?;
        // libxslt compiles `use` with XML_XPATH_NOVAR too.
        if pattern::contains_variable(&use_) {
            return Err(self.error(
                d,
                el,
                format!("xsl:key : XPath compilation failed '{use_text}'"),
            ));
        }
        let location = self.location(d, el);
        self.ss.keys.entry(name).or_default().push(KeyDef {
            matches,
            use_,
            location,
            precedence: self.precedence,
            order: self.order,
        });
        Ok(())
    }

    fn attribute_set(&mut self, d: &Doc, el: NodeId) -> R<()> {
        let name = self.required(d, el, "name")?;
        let name = self.qname(d, el, name)?;
        let uses = self.attr(d.doc, el, "use-attribute-sets");
        let uses = self.qname_list(d, el, uses)?;
        let mut attributes = Vec::new();
        for child in d.doc.children(el) {
            if self.is_xslt_named(d.doc, child, "attribute") {
                attributes.push(self.instruction(d, child)?);
            } else if matches!(d.doc.kind(child), NodeKind::Element { .. }) {
                return Err(self.error(
                    d,
                    child,
                    "Only xsl:attribute is allowed in xsl:attribute-set",
                ));
            }
        }
        // Several sets with the same name in one module merge, later
        // attributes winning. Modules merge in assign_precedence.
        let module = self.precedence;
        let defs = self.module_sets.entry(name).or_default();
        let set = match defs.iter_mut().find(|(m, _)| *m == module) {
            Some((_, set)) => set,
            None => {
                defs.push((
                    module,
                    AttributeSet {
                        uses: vec![],
                        attributes: vec![],
                    },
                ));
                &mut defs.last_mut().expect("just pushed").1
            }
        };
        set.uses.extend(uses);
        set.attributes.extend(attributes);
        Ok(())
    }
}
