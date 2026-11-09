//! XML parsing: quick-xml tokenizes, this module builds the [`Document`].
//!
//! quick-xml leaves several things to the caller, all handled here:
//! - detecting the encoding (BOM or XML declaration) and transcoding to UTF-8
//! - normalising line endings (`\r\n` and `\r` become `\n`)
//! - namespaces: this module keeps its own binding stack, because entity
//!   replacement text is parsed in the context of the reference
//! - references: character references, the predefined entities, and the
//!   entities a DTD declares, expanded as libxml2 does with XML_PARSE_NOENT
//! - DTD processing ([`crate::dtd`]): default attributes, attribute types
//! - turning byte offsets into line and column numbers

use std::borrow::Cow;

use encoding_rs::Encoding;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

use crate::dtd::{self, AttDefault, AttType, Base, Dtd, Entity, Limits, Loader, NoLoader};
use crate::tree::{Builder, Document, NamePool, NsDecl, QName, Sym, TooLarge};

#[derive(Clone, Debug)]
pub struct ParseOptions {
    /// Maximum element nesting depth. libxml2 (without XML_PARSE_HUGE,
    /// as xsltproc runs it) accepts 257 nested elements.
    pub max_depth: usize,
    /// Maximum bytes in one text node, comment, PI or CDATA section
    /// (libxml2's XML_MAX_TEXT_LENGTH), and, roughly, in an attribute value.
    pub max_text: usize,
    /// Maximum bytes in a name (XML_MAX_NAME_LENGTH).
    pub max_name: usize,
    /// Maximum namespace bindings in scope at once, a guard on the cost of
    /// each prefix lookup.
    pub max_namespace_bindings: usize,
    /// Load the external DTD subset (libxml2's XML_PARSE_DTDLOAD). xsltproc
    /// sets it unless `--novalid` is given.
    pub load_dtd: bool,
    /// Add default attributes from the DTD (XML_PARSE_DTDATTR). xsltproc
    /// sets it unless `--novalid` is given; it applies to the internal
    /// subset too.
    pub dtd_attributes: bool,
    /// Total characters entity expansion may produce, against "billion
    /// laughs" documents.
    pub max_entity_expansion: usize,
    /// `--xinclude`: process XIncludes in inputs and in documents loaded by
    /// `document()` (`crate::xinclude`). Not done by the parser itself.
    pub xinclude: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions {
            max_depth: 257,
            max_text: 10_000_000,
            max_name: 50_000,
            max_namespace_bindings: 1_024,
            load_dtd: true,
            dtd_attributes: true,
            max_entity_expansion: 10_000_000,
            xinclude: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    /// 1-based line and column. Column counts characters.
    pub line: u32,
    pub column: u32,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

/// A parsed document and the warnings parsing raised, such as an external
/// DTD that couldn't be loaded.
pub struct Parsed {
    pub document: Document,
    pub warnings: Vec<String>,
}

/// Parses an XML document from raw bytes, with no way to load external
/// DTDs or entities.
pub fn parse(
    bytes: &[u8],
    names: &mut NamePool,
    options: &ParseOptions,
) -> Result<Document, ParseError> {
    parse_with(bytes, names, options, &NoLoader, &None).map(|p| p.document)
}

/// Parses an XML document, loading external DTDs and entities through
/// `loader`. `base` is where the document was read from.
pub fn parse_with(
    bytes: &[u8],
    names: &mut NamePool,
    options: &ParseOptions,
    loader: &dyn Loader,
    base: &Base,
) -> Result<Parsed, ParseError> {
    let decoded = decode(bytes)?;
    let text = normalize_line_endings(&decoded);
    let mut parser = Parser {
        text: &text,
        names,
        options,
        loader,
        base: base.clone(),
        lines: LineCounter::default(),
        builder: Builder::new(),
        ns: Vec::new(),
        ns_counts: Vec::new(),
        dtd: Dtd::default(),
        seen_root: false,
        entities: Vec::new(),
        expanded: 0,
        warnings: Vec::new(),
        attr_text: String::new(),
        attr_spans: Vec::new(),
        attr_seen: Vec::new(),
    };
    parser.run()?;
    let mut warnings = parser.warnings;
    warnings.append(&mut parser.dtd.warnings);
    Ok(Parsed {
        document: parser.builder.finish(),
        warnings,
    })
}

/// Decodes an external entity or subset's bytes, as a document's: encoding
/// from a BOM or the text declaration, line endings normalised.
pub(crate) fn decode_entity(bytes: &[u8]) -> Result<String, ParseError> {
    let decoded = decode(bytes)?;
    Ok(normalize_line_endings(&decoded).into_owned())
}

// ---------- encoding ----------

/// Decodes the input to UTF-8, using the BOM, then the XML declaration's
/// `encoding`, then UTF-8 as the default.
fn decode(bytes: &[u8]) -> Result<Cow<'_, str>, ParseError> {
    use crate::encoding::{self, Charset};
    let (charset, skip) = if let Some((enc, bom_len)) = Encoding::for_bom(bytes) {
        let cs = if enc == encoding_rs::UTF_8 {
            Charset::Utf8
        } else {
            Charset::Other(enc)
        };
        (cs, bom_len)
    } else if bytes.starts_with(&[0x3C, 0x00, 0x3F, 0x00]) {
        (Charset::Other(encoding_rs::UTF_16LE), 0)
    } else if bytes.starts_with(&[0x00, 0x3C, 0x00, 0x3F]) {
        (Charset::Other(encoding_rs::UTF_16BE), 0)
    } else if let Some(label) = declared_encoding(bytes) {
        // ISO-8859-1 and US-ASCII are taken literally, as libxml2 does, not
        // as windows-1252 the way the WHATWG labels say.
        let cs = encoding::lookup(&label).ok_or_else(|| ParseError {
            message: format!("Unsupported encoding {label}"),
            line: 1,
            column: 1,
        })?;
        (cs, 0)
    } else {
        (Charset::Utf8, 0)
    };
    let body = &bytes[skip..];

    if charset == Charset::Utf8 {
        return std::str::from_utf8(body).map(Cow::Borrowed).map_err(|e| {
            let valid = std::str::from_utf8(&body[..e.valid_up_to()]).unwrap_or_default();
            let (line, column) = line_col(valid, valid.len());
            ParseError {
                message: "Input is not proper UTF-8".into(),
                line,
                column,
            }
        });
    }
    charset.decode(body).ok_or_else(|| ParseError {
        message: format!("Input is not proper {}", charset.name()),
        line: 1,
        column: 1,
    })
}

/// The `encoding` pseudo-attribute of an XML declaration in an ASCII-compatible
/// encoding, if there is one.
fn declared_encoding(bytes: &[u8]) -> Option<String> {
    let head = bytes.get(..bytes.len().min(1024))?;
    if !head.starts_with(b"<?xml") {
        return None;
    }
    let end = head.windows(2).position(|w| w == b"?>")?;
    let decl = std::str::from_utf8(&head[..end]).ok()?;
    let rest = &decl[decl.find("encoding")? + "encoding".len()..];
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = &rest[1..];
    Some(value[..value.find(quote)?].to_string())
}

/// XML 1.0 section 2.11: `\r\n` and lone `\r` become `\n` before parsing.
fn normalize_line_endings(s: &str) -> Cow<'_, str> {
    if !s.contains('\r') {
        return Cow::Borrowed(s);
    }
    Cow::Owned(s.replace("\r\n", "\n").replace('\r', "\n"))
}

// ---------- positions ----------

/// 1-based line and column (in characters) of byte offset `pos` in `text`.
fn line_col(text: &str, pos: usize) -> (u32, u32) {
    let pos = floor_char_boundary(text, pos.min(text.len()));
    let before = &text[..pos];
    let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
    let col = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (saturate(line), saturate(col))
}

fn floor_char_boundary(text: &str, mut pos: usize) -> usize {
    while !text.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn saturate(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Line numbers for increasing offsets in linear total time.
#[derive(Default)]
struct LineCounter {
    offset: usize,
    line: usize,
}

impl LineCounter {
    fn line_at(&mut self, text: &str, pos: usize) -> u32 {
        let pos = pos.min(text.len());
        if pos < self.offset {
            *self = LineCounter::default();
        }
        self.line += text.as_bytes()[self.offset..pos]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        self.offset = pos;
        saturate(self.line + 1)
    }
}

// ---------- the parser ----------

struct Parser<'a> {
    /// The document text, for error positions.
    text: &'a str,
    names: &'a mut NamePool,
    options: &'a ParseOptions,
    loader: &'a dyn Loader,
    base: Base,
    lines: LineCounter,
    builder: Builder,
    /// Namespace bindings in scope, innermost last.
    ns: Vec<(Option<Sym>, Sym)>,
    /// How many bindings each open element pushed.
    ns_counts: Vec<usize>,
    dtd: Dtd,
    seen_root: bool,
    /// Entities being expanded, to detect a reference loop.
    entities: Vec<String>,
    /// Characters produced by entity expansion so far.
    expanded: usize,
    warnings: Vec<String>,
    /// Scratch space for `start_element`, reused to save allocations: the
    /// attributes' names and decoded values, (name start, value start,
    /// value end) in `attr_text`, and the expanded names seen.
    attr_text: String,
    attr_spans: Vec<(usize, usize, usize)>,
    attr_seen: Vec<(Option<Sym>, Sym)>,
}

fn error_at(text: &str, pos: usize, message: impl Into<String>) -> ParseError {
    let (line, column) = line_col(text, pos);
    ParseError {
        message: message.into(),
        line,
        column,
    }
}

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

impl Parser<'_> {
    fn err(&self, pos: usize, message: impl Into<String>) -> ParseError {
        error_at(self.text, pos, message)
    }

    fn reader(text: &str) -> Reader<&[u8]> {
        let mut reader = Reader::from_str(text);
        reader.config_mut().expand_empty_elements = true;
        reader.config_mut().trim_text(false);
        reader
    }

    fn run(&mut self) -> Result<(), ParseError> {
        let text = self.text;
        let mut reader = Self::reader(text);
        loop {
            let start = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            let line = self.lines.line_at(text, start);
            let event = reader.read_event().map_err(|e| {
                let at = usize::try_from(reader.error_position()).unwrap_or(start);
                self.err(at, e.to_string())
            })?;
            if matches!(event, Event::Eof) {
                break;
            }
            self.event(event, start, line, None)?;
        }
        if !self.seen_root {
            return Err(self.err(text.len(), "Start tag expected, '<' not found"));
        }
        if self.builder.depth() > 0 {
            return Err(self.err(text.len(), "Premature end of data"));
        }
        Ok(())
    }

    /// Handles one event. `pos` and `line` locate it in the document; inside
    /// an entity's replacement text (`entity` is its name) they're those of
    /// the reference.
    fn event(
        &mut self,
        event: Event<'_>,
        pos: usize,
        line: u32,
        entity: Option<&str>,
    ) -> Result<(), ParseError> {
        let text = self.text;
        let tl = move |_: TooLarge| error_at(text, pos, "Document too large");
        let in_root = self.builder.depth() > 0;
        match event {
            Event::Start(e) => {
                if !in_root && self.seen_root && entity.is_none() {
                    return Err(self.err(pos, "Extra content at the end of the document"));
                }
                if self.builder.depth() >= self.options.max_depth {
                    return Err(self.err(
                        pos,
                        "Excessive depth in document: 256 use XML_PARSE_HUGE option",
                    ));
                }
                self.seen_root = true;
                self.start_element(&e, pos, line)?;
            }
            Event::End(_) => {
                self.builder.end_element();
                let n = self.ns_counts.pop().unwrap_or(0);
                self.ns.truncate(self.ns.len() - n);
            }
            Event::Text(t) => {
                // XML 1.0 section 2.4: "]]>" may not appear in character
                // data. quick-xml doesn't check this.
                // A byte search first: setting up a substring search costs
                // more than most text nodes do.
                if let Some(i) = t.contains(']').then(|| t.find("]]>")).flatten() {
                    return Err(self.err(
                        if entity.is_some() { pos } else { pos + i },
                        "Sequence ']]>' not allowed in content",
                    ));
                }
                if in_root {
                    self.builder.text(&t, line).map_err(tl)?;
                    self.check_text_length(pos)?;
                } else if !t.chars().all(|c| matches!(c, ' ' | '\t' | '\n')) {
                    let msg = if self.seen_root {
                        "Extra content at the end of the document"
                    } else {
                        "Start tag expected, '<' not found"
                    };
                    return Err(self.err(pos, msg));
                }
            }
            Event::CData(t) => {
                if !in_root {
                    return Err(self.err(pos, "CDATA section outside the document element"));
                }
                if t.len() > self.options.max_text {
                    return Err(self.err(pos, "CData section too big found"));
                }
                self.builder.text(&t, line).map_err(tl)?;
                self.check_text_length(pos)?;
            }
            Event::GeneralRef(r) => {
                if !in_root {
                    return Err(self.err(pos, "Entity reference outside the document element"));
                }
                let name: &str = &r;
                self.reference(name, pos, line)?;
            }
            Event::Comment(c) => {
                if c.len() > self.options.max_text {
                    return Err(self.err(pos, "Comment too big found"));
                }
                // XML 1.0 section 2.5: no "--" inside, and no '-' at the end
                // (as in "--->"). quick-xml doesn't check this.
                if c.contains("--") || c.ends_with('-') {
                    let start: String = c.chars().take(50).collect();
                    return Err(self.err(pos, format!("Double hyphen within comment: <!--{start}")));
                }
                self.builder.comment(&c, line).map_err(tl)?
            }
            Event::PI(p) => {
                let target_name = p.target();
                if p.content().len() > self.options.max_text {
                    return Err(self.err(pos, format!("PI {target_name} too big found")));
                }
                if target_name.len() > self.options.max_name {
                    return Err(self.err(pos, "Name too long"));
                }
                let target = self.names.intern(p.target());
                self.builder
                    .pi(target, p.content().trim_start(), line)
                    .map_err(tl)?;
            }
            Event::DocType(d) => {
                if entity.is_some() || self.seen_root {
                    return Err(self.err(pos, "DOCTYPE not allowed here"));
                }
                self.builder.set_has_doctype();
                let content: &str = &d;
                self.doctype(content.to_string(), pos)?;
            }
            Event::Decl(_) | Event::Empty(_) | Event::Eof => {}
        }
        Ok(())
    }

    fn doctype(&mut self, content: String, pos: usize) -> Result<(), ParseError> {
        let limits = Limits {
            max_expansion: self.options.max_entity_expansion,
            ..Limits::default()
        };
        dtd::parse_doctype(&content, &mut self.dtd, self.loader, &self.base, limits)
            .map_err(|e| self.err(pos, e.0))?;
        if self.options.load_dtd {
            dtd::load_external_subset(&mut self.dtd, self.loader, &self.base, limits)
                .map_err(|e| self.err(pos, e.0))?;
        }
        // Unparsed entities' URIs, resolved against where they were declared.
        for (name, ent) in &self.dtd.general {
            if let Entity::Unparsed { system, base } = ent {
                let uri = match base
                    .as_ref()
                    .and_then(|b| b.parent())
                    .filter(|p| !p.as_os_str().is_empty())
                {
                    Some(dir) if !system.contains("://") && !system.starts_with('/') => {
                        dir.join(system).to_string_lossy().into_owned()
                    }
                    _ => system.clone(),
                };
                self.builder.add_unparsed_entity(name, &uri);
            }
        }
        Ok(())
    }

    /// A reference in content: a character reference, a predefined entity,
    /// or an entity the DTD declares, whose replacement text is parsed as
    /// content in place.
    fn reference(&mut self, name: &str, pos: usize, line: u32) -> Result<(), ParseError> {
        let text = self.text;
        let tl = move |_: TooLarge| error_at(text, pos, "Document too large");
        if let Some(digits) = name.strip_prefix('#') {
            let c = dtd::char_ref(digits)
                .ok_or_else(|| self.err(pos, format!("Invalid character reference &{name};")))?;
            let mut buf = [0u8; 4];
            return self.builder.text(c.encode_utf8(&mut buf), line).map_err(tl);
        }
        if let Some(c) = dtd::predefined(name) {
            let mut buf = [0u8; 4];
            return self.builder.text(c.encode_utf8(&mut buf), line).map_err(tl);
        }
        let replacement = match self.dtd.general.get(name).cloned() {
            Some(Entity::Internal(t)) => t,
            Some(Entity::External {
                public,
                system,
                base,
            }) => match self.loader.load(public.as_deref(), &system, &base) {
                Ok((t, _)) => dtd::strip_text_decl(&t).to_string(),
                Err(w) => {
                    self.warnings.push(w);
                    return Ok(());
                }
            },
            Some(Entity::Unparsed { .. }) => {
                return Err(self.err(pos, format!("Entity reference to unparsed entity {name}")));
            }
            None => {
                // With an external subset or parameter entities, an undeclared
                // entity is only a warning (XML 1.0 section 4.1, WFC: Entity
                // Declared), and the reference is dropped.
                if self.dtd.has_external_subset || self.dtd.has_pe_refs {
                    self.warnings.push(format!("Entity '{name}' not defined"));
                    self.builder.break_text();
                    return Ok(());
                }
                return Err(self.err(pos, format!("Entity '{name}' not defined")));
            }
        };
        if self.entities.iter().any(|e| e == name) || self.entities.len() >= 40 {
            return Err(self.err(pos, "Detected an entity reference loop"));
        }
        self.expanded += replacement.len();
        if self.expanded > self.options.max_entity_expansion {
            return Err(self.err(pos, "Maximum entity amplification factor exceeded"));
        }
        if !replacement.contains(['<', '&']) {
            return self.builder.text(&replacement, line).map_err(tl);
        }
        // Markup in the replacement text: parse it as content here, with the
        // namespaces in scope. It must be balanced.
        self.entities.push(name.to_string());
        let depth = self.builder.depth();
        let mut reader = Self::reader(&replacement);
        loop {
            let event = reader
                .read_event()
                .map_err(|e| self.err(pos, format!("In entity '{name}': {e}")))?;
            if matches!(event, Event::Eof) {
                break;
            }
            self.event(event, pos, line, Some(name))?;
        }
        if self.builder.depth() != depth {
            return Err(self.err(pos, format!("Entity '{name}' isn't balanced")));
        }
        self.entities.pop();
        Ok(())
    }

    fn lookup(&self, prefix: Option<&str>) -> Option<Sym> {
        match prefix {
            Some("xml") => self.names.get(XML_NS),
            Some(p) => {
                let sym = self.names.get(p)?;
                self.ns
                    .iter()
                    .rev()
                    .find(|(q, _)| *q == Some(sym))
                    .map(|&(_, u)| u)
            }
            None => self
                .ns
                .iter()
                .rev()
                .find(|(q, _)| q.is_none())
                .map(|&(_, u)| u),
        }
    }

    /// libxml2's xmlSAX2Text: a text node, with the pieces merged into it,
    /// may hold at most `max_text` bytes.
    fn check_text_length(&self, pos: usize) -> Result<(), ParseError> {
        if self.builder.current_text_len() > self.options.max_text {
            return Err(self.err(pos, "xmlSAX2Characters: huge text node"));
        }
        Ok(())
    }

    fn start_element(
        &mut self,
        e: &BytesStart<'_>,
        pos: usize,
        line: u32,
    ) -> Result<(), ParseError> {
        let text = self.text;
        let tl = move |_: TooLarge| error_at(text, pos, "Document too large");
        let qname = e.name();
        let raw_name: &str = AsRef::<str>::as_ref(&qname);
        if raw_name.len() > self.options.max_name {
            return Err(self.err(pos, "Name too long"));
        }

        // Attributes as written, decoded and normalised as their declared
        // type requires; then the DTD's defaults for any not given.
        let mut text_buf = std::mem::take(&mut self.attr_text);
        let mut spans = std::mem::take(&mut self.attr_spans);
        text_buf.clear();
        spans.clear();
        // Duplicate names are checked here, not by quick-xml, whose check
        // allocates for every element: a direct scan for a few attributes,
        // a hash set past that.
        let mut many: Option<std::collections::HashSet<String>> = None;
        let mut attributes = e.attributes();
        attributes.with_checks(false);
        for a in attributes {
            let a = a.map_err(|err| self.err(pos, err.to_string()))?;
            let key: &str = a.key.as_ref();
            if key.len() > self.options.max_name {
                return Err(self.err(pos, "Name too long"));
            }
            let duplicate = match &mut many {
                Some(set) => !set.insert(key.to_string()),
                None => {
                    let dup = spans.iter().any(|&(k, vs, _)| &text_buf[k..vs] == key);
                    if spans.len() >= 16 {
                        let mut set: std::collections::HashSet<String> = spans
                            .iter()
                            .map(|&(k, vs, _)| text_buf[k..vs].to_string())
                            .collect();
                        set.insert(key.to_string());
                        many = Some(set);
                    }
                    dup
                }
            };
            if duplicate {
                return Err(self.err(pos, format!("Attribute {key} redefined")));
            }
            // libxml2 fails once its read position is 10,000,000 bytes past
            // its buffer's start ("Huge input lookup"), which depends on
            // where the buffer was last shrunk; for an attribute near the
            // start of a document, that's a value of 9,999,993 bytes.
            if a.value.len() + 8 > self.options.max_text {
                return Err(self.err(pos, "internal error: Huge input lookup"));
            }
            let start = text_buf.len();
            text_buf.push_str(key);
            let value_start = text_buf.len();
            dtd::decode_attribute_value_into(&a.value, &self.dtd.general, 0, &mut text_buf)
                .map_err(|m| self.err(pos, m))?;
            if self
                .dtd
                .attdef(raw_name, key)
                .is_some_and(|d| d.kind != AttType::Cdata)
            {
                dtd::collapse_spaces_from(&mut text_buf, value_start);
            }
            spans.push((start, value_start, text_buf.len()));
        }
        if self.options.dtd_attributes
            && let Some(defs) = self.dtd.attlists.get(raw_name)
        {
            for d in defs {
                if let AttDefault::Value(v) = &d.default
                    && !spans.iter().any(|&(k, vs, _)| text_buf[k..vs] == d.name)
                {
                    let start = text_buf.len();
                    text_buf.push_str(&d.name);
                    let value_start = text_buf.len();
                    text_buf.push_str(v);
                    spans.push((start, value_start, text_buf.len()));
                }
            }
        }
        let attr = |&(k, vs, ve): &(usize, usize, usize)| (&text_buf[k..vs], &text_buf[vs..ve]);

        // Namespace declarations apply to the element's own name and attributes.
        let mut decls = Vec::new();
        for (k, v) in spans.iter().map(attr) {
            let prefix = if k == "xmlns" {
                None
            } else if let Some(p) = k.strip_prefix("xmlns:") {
                Some(self.names.intern(p))
            } else {
                continue;
            };
            decls.push(NsDecl {
                prefix,
                uri: self.names.intern(v),
            });
        }
        if self.ns.len() + decls.len() > self.options.max_namespace_bindings {
            return Err(self.err(
                pos,
                format!(
                    "more than {} namespace bindings in scope",
                    self.options.max_namespace_bindings
                ),
            ));
        }
        for d in &decls {
            self.ns.push((d.prefix, d.uri));
        }
        self.ns_counts.push(decls.len());

        let (prefix, local) = split_qname(raw_name);
        let ns = match prefix {
            Some(p) => Some(
                self.lookup(Some(p))
                    .ok_or_else(|| self.err(pos, format!("Namespace prefix {p} is not defined")))?,
            ),
            None => self.lookup(None).filter(|&u| !self.names.str(u).is_empty()),
        };
        let name = QName {
            ns,
            local: self.names.intern(local),
            prefix: prefix.map(|p| self.names.intern(p)),
        };
        self.builder.start_element(name, line).map_err(tl)?;
        for d in decls {
            self.builder.add_ns_decl(d);
        }
        let mut seen = std::mem::take(&mut self.attr_seen);
        seen.clear();
        for (k, v) in spans.iter().map(attr) {
            if k == "xmlns" || k.starts_with("xmlns:") {
                continue;
            }
            // libxml2's xmlIsID: declared as ID in the DTD, or xml:id.
            if k == "xml:id"
                || self
                    .dtd
                    .attdef(raw_name, k)
                    .is_some_and(|d| d.kind == AttType::Id)
            {
                self.builder.add_id(v);
            }
            let (p, l) = split_qname(k);
            let ns = match p {
                Some(p) => Some(self.lookup(Some(p)).ok_or_else(|| {
                    self.err(pos, format!("Namespace prefix {p} is not defined"))
                })?),
                None => None,
            };
            let q = QName {
                ns,
                local: self.names.intern(l),
                prefix: p.map(|p| self.names.intern(p)),
            };
            if seen.contains(&q.expanded()) {
                return Err(self.err(pos, format!("Namespaced Attribute {l} redefined")));
            }
            seen.push(q.expanded());
            self.builder.add_attribute(q, v).map_err(tl)?;
        }
        self.attr_text = text_buf;
        self.attr_spans = spans;
        self.attr_seen = seen;
        Ok(())
    }
}

fn split_qname(q: &str) -> (Option<&str>, &str) {
    match q.split_once(':') {
        Some((p, l)) if !p.is_empty() && !l.is_empty() => (Some(p), l),
        _ => (None, q),
    }
}

#[cfg(test)]
mod tests;
