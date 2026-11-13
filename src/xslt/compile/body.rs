//! Template bodies: instructions, literal result elements, xsl:number,
//! sorts and parameters, xsl:fallback and extension elements (exsl:document,
//! saxon:output, xt:document).

use super::*;

/// The namespaces whose `document` element writes a file: EXSLT's and
/// James Clark's XT.
pub const DOCUMENT_ELEMENT_NAMESPACES: [&str; 2] =
    ["http://exslt.org/common", "http://www.jclark.com/xt"];

/// Saxon's namespace, for `saxon:output`, which libxslt writes as it writes
/// `exsl:document`.
const SAXON_NS: &str = "http://icl.com/saxon";

/// The output attributes xsltDocumentElem reads, in its order.
const DOCUMENT_SETTINGS: [&str; 10] = [
    "version",
    "encoding",
    "method",
    "doctype-system",
    "doctype-public",
    "standalone",
    "indent",
    "omit-xml-declaration",
    "cdata-section-elements",
    "append",
];

impl Compiler<'_> {
    pub(super) fn ignorable_text(&self, d: &Doc, node: NodeId, parent: NodeId) -> bool {
        let doc = d.doc;
        match doc.kind(node) {
            NodeKind::Text(_) => {
                doc.value(node)
                    .unwrap_or("")
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
                    && !self.is_xslt_named(doc, parent, "text")
                    && !self.preserves_space(d, parent)
            }
            NodeKind::Comment(_) | NodeKind::Pi { .. } => true,
            _ => false,
        }
    }

    pub(super) fn body(&mut self, d: &Doc, el: NodeId) -> R<Body> {
        let children: Vec<NodeId> = d.doc.children(el).collect();
        self.nodes(d, el, &children)
    }

    pub(super) fn nodes(&mut self, d: &Doc, parent: NodeId, nodes: &[NodeId]) -> R<Body> {
        let mut out = Vec::new();
        for &n in nodes {
            match d.doc.kind(n) {
                NodeKind::Text(_) => {
                    if !self.ignorable_text(d, n, parent) {
                        out.push(Instr::Text(d.doc.value(n).unwrap_or("").to_string()));
                    }
                }
                NodeKind::Element { .. } => {
                    if self.is_xslt(d.doc, n) {
                        if self.is_xslt_named(d.doc, n, "fallback") {
                            continue;
                        }
                        out.push(self.instruction(d, n)?);
                    } else if self.is_extension(d, n) {
                        match self.document_element(d, n)? {
                            Some(instr) => out.push(instr),
                            None => out.extend(self.fallback(d, n)?),
                        }
                    } else {
                        out.push(self.literal_element(d, n)?);
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Whether `el` is an extension element. libxslt (xsltCheckExtPrefix)
    /// compares the element's prefix, as written, with the declared
    /// extension prefixes; another prefix for the same URI doesn't count.
    fn is_extension(&self, d: &Doc, el: NodeId) -> bool {
        let Some(q) = d.doc.name(el) else {
            return false;
        };
        q.ns.is_some() && self.extension_prefixes(d, el).contains(&q.prefix)
    }

    /// The prefixes declared by `extension-element-prefixes` on `el` and
    /// its ancestors (`#default` as None).
    fn extension_prefixes(&self, d: &Doc, el: NodeId) -> HashSet<Option<Sym>> {
        let mut out = HashSet::new();
        let mut cur = Some(el);
        while let Some(e) = cur {
            if matches!(d.doc.kind(e), NodeKind::Element { .. }) {
                let v = if self.is_xslt(d.doc, e) {
                    self.attr(d.doc, e, "extension-element-prefixes")
                } else {
                    self.xsl_attr(d.doc, e, "extension-element-prefixes")
                };
                for p in v.unwrap_or("").split_ascii_whitespace() {
                    match p {
                        "#default" => {
                            out.insert(None);
                        }
                        p => {
                            if let Some(sym) = self.names.get(p) {
                                out.insert(Some(sym));
                            }
                        }
                    }
                }
            }
            cur = d.doc.parent(e);
        }
        out
    }

    /// `exsl:document` or `xt:document` (libxslt's xsltDocumentComp): None
    /// for other extension elements.
    fn document_element(&mut self, d: &Doc, el: NodeId) -> R<Option<Instr>> {
        let q = d.doc.name(el).unwrap();
        let ns = q.ns.map(|u| self.names.str(u)).unwrap_or("");
        let local = self.names.str(q.local);
        let saxon = ns == SAXON_NS && local == "output";
        if !saxon && (local != "document" || !DOCUMENT_ELEMENT_NAMESPACES.contains(&ns)) {
            return Ok(None);
        }
        let href = if saxon {
            // saxon:output's file: libxslt takes a static value at compile
            // time; an AVT makes it look for saxon:file or saxon:href
            // instead, which stylesheets don't have, and fail.
            match self.attr(d.doc, el, "file") {
                Some(v) if !v.contains('{') => Some(self.avt(d, el, v)?),
                _ => {
                    let in_saxon = |name: &str| {
                        d.doc.attributes(el).find_map(|a| {
                            let q = d.doc.name(a)?;
                            (q.ns.is_some_and(|u| self.names.str(u) == SAXON_NS)
                                && self.names.str(q.local) == name)
                                .then(|| d.doc.value(a).unwrap_or("").to_string())
                        })
                    };
                    match in_saxon("file").or_else(|| in_saxon("href")) {
                        Some(v) => Some(self.avt(d, el, &v)?),
                        None => None,
                    }
                }
            }
        } else {
            match self.attr(d.doc, el, "href") {
                Some(v) => Some(self.avt(d, el, v)?),
                None => None,
            }
        };
        let mut settings = Vec::new();
        for name in DOCUMENT_SETTINGS {
            if let Some(v) = self.attr(d.doc, el, name) {
                settings.push((name, self.avt(d, el, v)?));
            }
        }
        let body = self.body(d, el)?;
        Ok(Some(Instr::Document {
            href,
            settings,
            body,
            location: self.location(d, el),
        }))
    }

    /// An unknown extension element: its `xsl:fallback` children run instead.
    fn fallback(&mut self, d: &Doc, el: NodeId) -> R<Body> {
        let mut out = Vec::new();
        let mut found = false;
        for child in d.doc.children(el).collect::<Vec<_>>() {
            if self.is_xslt_named(d.doc, child, "fallback") {
                found = true;
                out.extend(self.body(d, child)?);
            }
        }
        if !found {
            let local = self.local(d.doc, el).unwrap_or("").to_string();
            out.push(Instr::Unsupported {
                what: format!("extension element {local}"),
                location: self.location(d, el),
            });
        }
        Ok(out)
    }

    /// A literal result element. Its namespace nodes are, as libxslt copies
    /// them: the declarations written on the element itself, then, only for
    /// an element directly in a template body ("first level"), the
    /// template's inherited namespaces. A literal element inside xsl:if,
    /// xsl:for-each or a variable doesn't inherit them, unlike the spec.
    pub(super) fn literal_element(&mut self, d: &Doc, el: NodeId) -> R<Instr> {
        let q = d.doc.name(el).unwrap();
        let (ns, prefix) = self.alias(q.ns, q.prefix);
        let name = QName {
            ns,
            local: q.local,
            prefix,
        };
        // Exclusions written on this element apply to its descendants only.
        let excluded = match d.doc.parent(el) {
            Some(p) if matches!(d.doc.kind(p), NodeKind::Element { .. }) => self.excluded(d, p),
            _ => HashSet::from([self.xslt]),
        };
        let mut candidates: Vec<(Binding, bool)> = d
            .doc
            .ns_decls(el)
            .iter()
            .map(|n| ((n.prefix, n.uri), true))
            .collect();
        let parent = d.doc.parent(el);
        if let Some(t) = parent.filter(|&p| self.is_xslt_named(d.doc, p, "template")) {
            // xsltGetInheritedNsList: from xsl:template upwards, nearest first,
            // minus stylesheet-level exclusions and extension prefixes.
            // Extension namespaces are skipped by prefix
            // (xsltCheckExtPrefix), exclusions by URI.
            let template_excluded = self.excluded_uris(d, t, false);
            let extension = self.extension_prefixes(d, t);
            for b in self.in_scope(d.doc, t) {
                let ext = b.0.is_some() && extension.contains(&b.0);
                if !template_excluded.contains(&b.1) && !ext {
                    candidates.push((b, false));
                }
            }
        }
        let mut namespaces: Vec<Binding> = Vec::new();
        let mut inherited: Vec<Binding> = Vec::new();
        for ((p, uri), own) in candidates {
            if self.names.str(uri).is_empty()
                || (own && excluded.contains(&uri))
                || uri == self.xslt
            {
                continue;
            }
            if namespaces.iter().chain(&inherited).any(|(q, _)| *q == p) {
                continue;
            }
            let uri = match self.aliases.get(&uri) {
                Some(&(_, au)) => au,
                None => uri,
            };
            if self.names.str(uri).is_empty() {
                continue;
            }
            if own {
                namespaces.push((p, uri));
            } else {
                inherited.push((p, uri));
            }
        }
        let mut attributes = Vec::new();
        let mut attribute_sets = Vec::new();
        for a in d.doc.attributes(el).collect::<Vec<_>>() {
            let aq = d.doc.name(a).unwrap();
            let value = d.doc.value(a).unwrap_or("").to_string();
            if aq.ns == Some(self.xslt) {
                if self.names.str(aq.local) == "use-attribute-sets" {
                    attribute_sets = self.qname_list(d, el, Some(&value))?;
                }
                continue;
            }
            let (ans, aprefix) = self.alias(aq.ns, aq.prefix);
            let avt = self.avt(d, el, &value)?;
            attributes.push((
                QName {
                    ns: ans,
                    local: aq.local,
                    prefix: aprefix,
                },
                avt,
            ));
        }
        let body = self.body(d, el)?;
        let location = self.location(d, el);
        Ok(Instr::LiteralElement {
            name,
            namespaces,
            inherited,
            attributes,
            attribute_sets,
            body,
            location,
        })
    }

    fn sorts_and_params(
        &mut self,
        d: &Doc,
        el: NodeId,
    ) -> R<(Vec<Sort>, Vec<Variable>, Vec<NodeId>)> {
        let mut sorts = Vec::new();
        let mut params = Vec::new();
        let mut rest = Vec::new();
        for child in d.doc.children(el) {
            if self.is_xslt_named(d.doc, child, "sort") {
                let select = self.attr(d.doc, child, "select").unwrap_or(".").to_string();
                let select = self.xpath(d, child, &select)?;
                let data_type = self.opt_avt(d, child, "data-type")?;
                let order = self.opt_avt(d, child, "order")?;
                let case_order = self.opt_avt(d, child, "case-order")?;
                // `lang` is ignored for now: the reference xsltproc has no
                // locales installed, so it sorts as without one.
                sorts.push(Sort {
                    select,
                    data_type,
                    order,
                    case_order,
                });
            } else if self.is_xslt_named(d.doc, child, "with-param") {
                params.push(self.variable(d, child)?);
            } else {
                rest.push(child);
            }
        }
        Ok((sorts, params, rest))
    }

    pub(super) fn instruction(&mut self, d: &Doc, el: NodeId) -> R<Instr> {
        let local = self.local(d.doc, el).unwrap_or("").to_string();
        let location = self.location(d, el);
        let doc = d.doc;
        Ok(match local.as_str() {
            "apply-templates" => {
                let select = match self.attr(doc, el, "select") {
                    Some(s) => Some(self.xpath(d, el, s)?),
                    None => None,
                };
                let mode = match self.attr(doc, el, "mode") {
                    Some(m) => Some(self.qname(d, el, m)?),
                    None => None,
                };
                let (sorts, params, _) = self.sorts_and_params(d, el)?;
                Instr::ApplyTemplates {
                    select,
                    mode,
                    sorts,
                    params,
                    location,
                }
            }
            "call-template" => {
                let name = self.required(d, el, "name")?;
                let name = self.qname(d, el, name)?;
                let (_, params, _) = self.sorts_and_params(d, el)?;
                Instr::CallTemplate {
                    name,
                    params,
                    location,
                }
            }
            "value-of" => {
                let select = self.required(d, el, "select")?;
                let unescaped = self
                    .attr(doc, el, "disable-output-escaping")
                    .is_some_and(|v| v.trim() == "yes");
                Instr::ValueOf {
                    select: self.xpath(d, el, select)?,
                    unescaped,
                    location,
                }
            }
            "for-each" => {
                let select = self.required(d, el, "select")?;
                let select = self.xpath(d, el, select)?;
                let (sorts, _, rest) = self.sorts_and_params(d, el)?;
                let body = self.nodes(d, el, &rest)?;
                Instr::ForEach {
                    select,
                    sorts,
                    body,
                    location,
                }
            }
            "if" => {
                let test = self.required(d, el, "test")?;
                Instr::If {
                    test: self.xpath(d, el, test)?,
                    body: self.body(d, el)?,
                    location,
                }
            }
            "choose" => {
                let mut whens = Vec::new();
                let mut otherwise = None;
                for child in doc.children(el) {
                    if self.is_xslt_named(doc, child, "when") {
                        let test = self.required(d, child, "test")?;
                        let test = self.xpath(d, child, test)?;
                        let location = self.location(d, child);
                        whens.push((test, self.body(d, child)?, location));
                    } else if self.is_xslt_named(doc, child, "otherwise") {
                        otherwise = Some(self.body(d, child)?);
                    } else if matches!(doc.kind(child), NodeKind::Element { .. }) {
                        return Err(self.error(
                            d,
                            child,
                            "Only xsl:when and xsl:otherwise are allowed in xsl:choose",
                        ));
                    }
                }
                if whens.is_empty() {
                    return Err(self.error(d, el, "xsl:choose needs at least one xsl:when"));
                }
                Instr::Choose { whens, otherwise }
            }
            "text" => {
                let mut s = String::new();
                for child in doc.children(el) {
                    match doc.kind(child) {
                        NodeKind::Text(_) => s.push_str(doc.value(child).unwrap_or("")),
                        NodeKind::Element { .. } => {
                            return Err(self.error(d, child, "xsl:text can only contain text"));
                        }
                        _ => {}
                    }
                }
                if self
                    .attr(doc, el, "disable-output-escaping")
                    .is_some_and(|v| v.trim() == "yes")
                {
                    Instr::UnescapedText(s)
                } else {
                    Instr::Text(s)
                }
            }
            "variable" => Instr::Variable(self.variable(d, el)?),
            "param" => {
                return Err(self.error(
                    d,
                    el,
                    "xsl:param is only allowed at the top level or first in xsl:template",
                ));
            }
            "copy" => {
                let sets = self.attr(doc, el, "use-attribute-sets");
                Instr::Copy {
                    attribute_sets: self.qname_list(d, el, sets)?,
                    body: self.body(d, el)?,
                }
            }
            "copy-of" => {
                let select = self.required(d, el, "select")?;
                Instr::CopyOf {
                    select: self.xpath(d, el, select)?,
                    location,
                }
            }
            "element" | "attribute" => {
                let raw = self.required(d, el, "name")?;
                let name = self.avt(d, el, raw)?;
                let namespace = self.opt_avt(d, el, "namespace")?;
                let scope = self.in_scope(doc, el);
                // libxslt checks a literal name at compile time.
                if let Avt::Literal(literal) = &name {
                    let literal = literal.trim();
                    let (prefix, local_part) = match literal.split_once(':') {
                        Some((p, l)) => (Some(p), l),
                        None => (None, literal),
                    };
                    if !is_ncname(local_part) || prefix.is_some_and(|p| !is_ncname(p)) {
                        return Err(self.error(d, el, format!(
                            "xsl:{local}: The value '{literal}' of the attribute 'name' is not a valid QName."
                        )));
                    }
                    if local == "attribute" && literal == "xmlns" {
                        return Err(self.error(
                            d,
                            el,
                            "xsl:attribute: The attribute name 'xmlns' is not allowed.",
                        ));
                    }
                    if let Some(p) = prefix
                        && namespace.is_none()
                        && p != "xml"
                        && self.resolve_prefix(&scope, p).is_none()
                    {
                        return Err(self.error(d, el, format!("xsl:{local}: The QName '{literal}' has no namespace binding in scope in the stylesheet")));
                    }
                }
                let body = self.body(d, el)?;
                if local == "element" {
                    let sets = self.attr(doc, el, "use-attribute-sets");
                    let attribute_sets = self.qname_list(d, el, sets)?;
                    Instr::Element {
                        name,
                        namespace,
                        scope,
                        attribute_sets,
                        body,
                        location,
                    }
                } else {
                    Instr::Attribute {
                        name,
                        namespace,
                        scope,
                        body,
                        location,
                    }
                }
            }
            "comment" => Instr::Comment(self.body(d, el)?),
            "processing-instruction" => {
                let name = self.required(d, el, "name")?;
                let name = self.avt(d, el, name)?;
                Instr::ProcessingInstruction {
                    name,
                    body: self.body(d, el)?,
                    location,
                }
            }
            "message" => {
                let terminate = self
                    .attr(doc, el, "terminate")
                    .is_some_and(|t| t.trim() == "yes");
                Instr::Message {
                    terminate,
                    body: self.body(d, el)?,
                }
            }
            "apply-imports" => Instr::ApplyImports { location },
            "number" => self.number(d, el, location)?,
            _ if d.forwards_compatible => {
                let mut body = Vec::new();
                for child in doc.children(el).collect::<Vec<_>>() {
                    if self.is_xslt_named(doc, child, "fallback") {
                        body.extend(self.body(d, child)?);
                    }
                }
                return Ok(Instr::Choose {
                    whens: vec![(Expr::Literal("x".into()), body, location)],
                    otherwise: None,
                });
            }
            _ => return Err(self.error(d, el, format!("Unknown XSLT instruction xsl:{local}"))),
        })
    }

    /// As libxslt's xsltNumberComp: `level`, `count`, `from`,
    /// `grouping-separator` and `grouping-size` are read as they are, not
    /// as attribute value templates; only `format` is one.
    fn number(&mut self, d: &Doc, el: NodeId, location: Location) -> R<Instr> {
        // libxslt evaluates "number(" + value + ")".
        let value = match self.attr(d.doc, el, "value") {
            Some(v) => Some(Expr::Call(
                xpath::ast::Function::Core(xpath::ast::CoreFn::Number),
                vec![self.xpath(d, el, v)?],
            )),
            None => None,
        };
        let level = match self.attr(d.doc, el, "level") {
            None | Some("single") => NumberLevel::Single,
            Some("multiple") => NumberLevel::Multiple,
            Some("any") => NumberLevel::Any,
            Some(other) => {
                let e = self.error(
                    d,
                    el,
                    format!("xsl:number : invalid value {other} for level"),
                );
                self.warnings.push(e.to_string());
                NumberLevel::Invalid
            }
        };
        let pattern = |c: &mut Self, attr: &str| -> R<Option<pattern::Pattern>> {
            match c.attr(d.doc, el, attr) {
                Some(p) => {
                    let expr = c.pattern_xpath(d, el, p)?;
                    let pat = pattern::from_expr(expr, true)
                        .map_err(|e| c.error(d, el, format!("{} '{p}'", e.0)))?;
                    Ok(Some(pat))
                }
                None => Ok(None),
            }
        };
        let count = pattern(self, "count")?;
        let from = pattern(self, "from")?;
        let format = match self.attr(d.doc, el, "format") {
            Some(f) => self.avt(d, el, f)?,
            None => Avt::Literal(String::new()),
        };
        let separator = self
            .attr(d.doc, el, "grouping-separator")
            .and_then(|s| s.chars().next());
        let grouping = match self.attr(d.doc, el, "grouping-size") {
            Some(size) => {
                let n = xpath::number::from_string(size);
                crate::xslt::number::Grouping {
                    separator,
                    // C's (int) of a double, as x86 does it for NaN.
                    size: if n.is_nan() {
                        i64::from(i32::MIN)
                    } else {
                        n as i64
                    },
                }
            }
            None => crate::xslt::number::Grouping::default(),
        };
        Ok(Instr::Number {
            value,
            level,
            count,
            from,
            format,
            grouping,
            location,
        })
    }
}
