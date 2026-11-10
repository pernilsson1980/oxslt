//! DTD processing, as libxml2 does it for xsltproc: entity declarations,
//! default attribute values and attribute types. No validation.
//!
//! The internal subset comes from the document's `<!DOCTYPE>`; the external
//! subset and external entities are read through a [`Loader`]. Parameter
//! entities are expanded between and inside declarations, and conditional
//! sections (`<![INCLUDE[…]]>`, `<![IGNORE[…]]>`) are honoured.

use std::collections::HashMap;
use std::path::PathBuf;

/// Where an entity or subset was read from, for resolving relative system IDs.
pub type Base = Option<PathBuf>;

/// Reads external subsets and entities. `public` and `system` are the
/// declared identifiers; `base` is where the declaration was read from.
pub trait Loader {
    /// The decoded text (without a text declaration) and where it came from.
    fn load(
        &self,
        public: Option<&str>,
        system: &str,
        base: &Base,
    ) -> Result<(String, Base), String>;
}

/// A loader that never finds anything.
pub struct NoLoader;

impl Loader for NoLoader {
    fn load(&self, _: Option<&str>, system: &str, _: &Base) -> Result<(String, Base), String> {
        Err(format!("failed to load external entity \"{system}\""))
    }
}

#[derive(Debug, Clone)]
pub enum Entity {
    /// Replacement text, with parameter-entity and character references
    /// already expanded; general entity references are left for later.
    Internal(String),
    External {
        public: Option<String>,
        system: String,
        base: Base,
    },
    Unparsed {
        system: String,
        base: Base,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttType {
    Cdata,
    Id,
    /// IDREF(S), ENTITY/ENTITIES, NMTOKEN(S), NOTATION or an enumeration:
    /// values are normalised further (spaces collapsed, ends trimmed).
    Tokenized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttDefault {
    Required,
    Implied,
    /// `#FIXED "v"` or a plain default value, already normalised.
    Value(String),
}

#[derive(Debug, Clone)]
pub struct AttDef {
    /// The attribute's name as written, prefix included.
    pub name: String,
    pub kind: AttType,
    pub default: AttDefault,
}

/// Limits on DTD processing, against hostile input.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Nesting of parameter-entity references and conditional sections.
    pub max_depth: usize,
    /// Total characters produced by parameter-entity expansion.
    pub max_expansion: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_depth: 40,
            max_expansion: 10_000_000,
        }
    }
}

#[derive(Debug, Default)]
pub struct Dtd {
    pub root: String,
    pub public_id: Option<String>,
    pub system_id: Option<String>,
    pub general: HashMap<String, Entity>,
    pub parameter: HashMap<String, Entity>,
    /// ATTLIST declarations by element name as written, in declaration order.
    pub attlists: HashMap<String, Vec<AttDef>>,
    /// The document declares an external subset.
    pub has_external_subset: bool,
    /// The subset referenced a parameter entity, so an undeclared general
    /// entity is only a warning (XML 1.0 well-formedness constraint).
    pub has_pe_refs: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct DtdError(pub String);

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '\u{B7}')
}

/// A name starting at byte `i`: (name, end).
fn name_at(s: &str, i: usize) -> Option<(&str, usize)> {
    let rest = &s[i..];
    let end = rest
        .char_indices()
        .find(|&(_, c)| !is_name_char(c))
        .map_or(rest.len(), |(j, _)| j);
    (end > 0).then(|| (&rest[..end], i + end))
}

impl Dtd {
    /// The attribute declaration for `attr` on `element`, if any.
    pub fn attdef(&self, element: &str, attr: &str) -> Option<&AttDef> {
        self.attlists.get(element)?.iter().find(|d| d.name == attr)
    }
}

/// Parses `<!DOCTYPE …>`'s content (everything between `<!DOCTYPE` and the
/// final `>`) and its internal subset into `dtd`. Doesn't load the external
/// subset; see [`load_external_subset`].
pub fn parse_doctype(
    content: &str,
    dtd: &mut Dtd,
    loader: &dyn Loader,
    base: &Base,
    limits: Limits,
) -> Result<(), DtdError> {
    let s = content.trim_start();
    let (root, mut i) = name_at(s, 0).ok_or_else(|| DtdError("DOCTYPE has no name".into()))?;
    dtd.root = root.to_string();
    let b = s.as_bytes();
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    if s[i..].starts_with("SYSTEM") || s[i..].starts_with("PUBLIC") {
        let (public, system, end) = external_id(s, i)?;
        dtd.public_id = public;
        dtd.system_id = Some(system);
        dtd.has_external_subset = true;
        i = end;
    }
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    if i < b.len() && b[i] == b'[' {
        let close = s
            .rfind(']')
            .ok_or_else(|| DtdError("Internal subset isn't closed".into()))?;
        let mut p = SubsetParser {
            dtd,
            loader,
            limits,
            expanded: 0,
        };
        p.subset(&s[i + 1..close], false, base, 0)?;
    }
    Ok(())
}

/// Loads and parses the external subset the DOCTYPE names, if any. A
/// failure to load is a warning, as in libxml2.
pub fn load_external_subset(
    dtd: &mut Dtd,
    loader: &dyn Loader,
    base: &Base,
    limits: Limits,
) -> Result<(), DtdError> {
    let Some(system) = dtd.system_id.clone() else {
        return Ok(());
    };
    match loader.load(dtd.public_id.as_deref(), &system, base) {
        Ok((text, ext_base)) => {
            let mut p = SubsetParser {
                dtd,
                loader,
                limits,
                expanded: 0,
            };
            p.subset(&text, true, &ext_base, 0)
        }
        Err(w) => {
            dtd.warnings.push(w);
            Ok(())
        }
    }
}

/// `SYSTEM "s"` or `PUBLIC "p" "s"` at `i`: (public, system, end).
fn external_id(s: &str, i: usize) -> Result<(Option<String>, String, usize), DtdError> {
    let public = s[i..].starts_with("PUBLIC");
    let mut j = i + 6;
    let (first, end) = literal(s, j)?;
    j = end;
    if public {
        let (system, end) = literal(s, j)?;
        return Ok((Some(first), system, end));
    }
    Ok((None, first, j))
}

/// A quoted literal after optional whitespace: (content, end after the quote).
fn literal(s: &str, mut i: usize) -> Result<(String, usize), DtdError> {
    let b = s.as_bytes();
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    let q = *b
        .get(i)
        .ok_or_else(|| DtdError("Expected a quoted literal".into()))?;
    if q != b'"' && q != b'\'' {
        return Err(DtdError("Expected a quoted literal".into()));
    }
    let end = s[i + 1..]
        .find(q as char)
        .ok_or_else(|| DtdError("Unfinished literal".into()))?
        + i
        + 1;
    Ok((s[i + 1..end].to_string(), end + 1))
}

/// Strips a text declaration (`<?xml encoding="…"?>`) from an external entity.
pub fn strip_text_decl(text: &str) -> &str {
    let t = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    if t.starts_with("<?xml")
        && t[5..].starts_with(|c: char| c.is_whitespace())
        && let Some(end) = t.find("?>")
    {
        return &t[end + 2..];
    }
    t
}

struct SubsetParser<'a> {
    dtd: &'a mut Dtd,
    loader: &'a dyn Loader,
    limits: Limits,
    expanded: usize,
}

impl SubsetParser<'_> {
    fn check_depth(&self, depth: usize) -> Result<(), DtdError> {
        if depth > self.limits.max_depth {
            return Err(DtdError("Detected an entity reference loop".into()));
        }
        Ok(())
    }

    /// The replacement text of parameter entity `name`, or None (with a
    /// warning) if it's undeclared or can't be loaded.
    fn pe_text(&mut self, name: &str) -> Result<Option<(String, Base)>, DtdError> {
        self.dtd.has_pe_refs = true;
        let Some(ent) = self.dtd.parameter.get(name).cloned() else {
            self.dtd
                .warnings
                .push(format!("PEReference: %{name}; not found"));
            return Ok(None);
        };
        let (text, base) = match ent {
            Entity::Internal(t) => (t, None),
            Entity::External {
                public,
                system,
                base,
            } => match self.loader.load(public.as_deref(), &system, &base) {
                Ok(r) => r,
                Err(w) => {
                    self.dtd.warnings.push(w);
                    return Ok(None);
                }
            },
            Entity::Unparsed { .. } => return Err(DtdError(format!("Unparsed entity %{name};"))),
        };
        self.expanded += text.len();
        if self.expanded > self.limits.max_expansion {
            return Err(DtdError(
                "Maximum entity amplification factor exceeded".into(),
            ));
        }
        Ok(Some((text, base)))
    }

    /// A sequence of markup declarations, PE references and (in an external
    /// subset or entity) conditional sections.
    fn subset(
        &mut self,
        s: &str,
        external: bool,
        base: &Base,
        depth: usize,
    ) -> Result<(), DtdError> {
        self.check_depth(depth)?;
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if is_ws(b[i]) {
                i += 1;
                continue;
            }
            let rest = &s[i..];
            if rest.starts_with('%') {
                let (name, end) =
                    name_at(s, i + 1).ok_or_else(|| DtdError("Invalid PE reference".into()))?;
                if !s[end..].starts_with(';') {
                    return Err(DtdError(format!(
                        "PEReference: expecting ';' after %{name}"
                    )));
                }
                i = end + 1;
                if let Some((text, pe_base)) = self.pe_text(name)? {
                    let text = strip_text_decl(&text).to_string();
                    let b2 = if pe_base.is_some() {
                        pe_base
                    } else {
                        base.clone()
                    };
                    // A PE's declarations may use conditional sections.
                    self.subset(&text, true, &b2, depth + 1)?;
                }
            } else if rest.starts_with("<!--") {
                i += rest
                    .find("-->")
                    .ok_or_else(|| DtdError("Comment not terminated".into()))?
                    + 3;
            } else if rest.starts_with("<?") {
                i += rest
                    .find("?>")
                    .ok_or_else(|| DtdError("PI not terminated".into()))?
                    + 2;
            } else if rest.starts_with("<![") {
                if !external {
                    return Err(DtdError(
                        "Conditional sections are only allowed in the external subset".into(),
                    ));
                }
                i = self.conditional(s, i, base, depth)?;
            } else if rest.starts_with("<!") {
                let end = decl_end(s, i)?;
                self.declaration(&s[i + 2..end], external, base, depth)?;
                i = end + 1;
            } else {
                return Err(DtdError(format!(
                    "Unexpected content in the DTD: '{}'",
                    rest.chars().take(20).collect::<String>()
                )));
            }
        }
        Ok(())
    }

    /// `<![ INCLUDE [ … ]]>` or `<![ IGNORE [ … ]]>` at `i`; returns the end.
    fn conditional(
        &mut self,
        s: &str,
        i: usize,
        base: &Base,
        depth: usize,
    ) -> Result<usize, DtdError> {
        let open = s[i..].find('[').map(|_| i + 3).unwrap_or(i + 3);
        let bracket = s[open..]
            .find('[')
            .ok_or_else(|| DtdError("Invalid conditional section".into()))?
            + open;
        let mut keyword = s[open..bracket].trim().to_string();
        // The keyword is often a parameter entity (%draft;).
        if let Some(name) = keyword.strip_prefix('%').and_then(|k| k.strip_suffix(';')) {
            keyword = match self.pe_text(name)? {
                Some((t, _)) => t.trim().to_string(),
                None => {
                    return Err(DtdError(format!(
                        "PEReference: %{name}; not found in a conditional section"
                    )));
                }
            };
        }
        // Find the matching ]]>, counting nested sections.
        let mut level = 1;
        let mut j = bracket + 1;
        let b = s.as_bytes();
        let end = loop {
            if j >= b.len() {
                return Err(DtdError("Conditional section not terminated".into()));
            }
            // Bytes, not str slices: j steps through multi-byte characters.
            if b[j..].starts_with(b"<![") {
                level += 1;
                j += 3;
            } else if b[j..].starts_with(b"]]>") {
                level -= 1;
                if level == 0 {
                    break j;
                }
                j += 3;
            } else {
                j += 1;
            }
        };
        match keyword.as_str() {
            "INCLUDE" => self.subset(&s[bracket + 1..end], true, base, depth + 1)?,
            "IGNORE" => {}
            k => {
                return Err(DtdError(format!(
                    "Invalid conditional section keyword '{k}'"
                )));
            }
        }
        Ok(end + 3)
    }

    /// Expands parameter-entity references outside quoted literals in a
    /// declaration's text, padding each with spaces as XML 1.0 requires.
    fn expand_pe_in_decl(&mut self, s: &str, depth: usize) -> Result<String, DtdError> {
        self.check_depth(depth)?;
        let mut out = String::with_capacity(s.len());
        let mut quote: Option<char> = None;
        let mut chars = s.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            match (quote, c) {
                (None, '"' | '\'') => {
                    quote = Some(c);
                    out.push(c);
                }
                (Some(q), c) if c == q => {
                    quote = None;
                    out.push(c);
                }
                (None, '%') if name_at(s, i + 1).is_some() => {
                    let (name, end) = name_at(s, i + 1).expect("checked");
                    if !s[end..].starts_with(';') {
                        out.push(c);
                        continue;
                    }
                    while chars.peek().is_some_and(|&(j, _)| j <= end) {
                        chars.next();
                    }
                    if let Some((text, _)) = self.pe_text(name)? {
                        let inner = self.expand_pe_in_decl(strip_text_decl(&text), depth + 1)?;
                        out.push(' ');
                        out.push_str(&inner);
                        out.push(' ');
                    }
                }
                _ => out.push(c),
            }
        }
        Ok(out)
    }

    fn declaration(
        &mut self,
        body: &str,
        external: bool,
        base: &Base,
        depth: usize,
    ) -> Result<(), DtdError> {
        if let Some(rest) = body.strip_prefix("ENTITY") {
            self.entity_decl(rest, external, base, depth)
        } else if let Some(rest) = body.strip_prefix("ATTLIST") {
            let rest = self.expand_pe_in_decl(rest, depth)?;
            self.attlist_decl(&rest)
        } else if body.starts_with("ELEMENT") || body.starts_with("NOTATION") {
            Ok(()) // no validation: content models and notations aren't needed
        } else {
            Err(DtdError(format!(
                "Unknown declaration <!{}",
                body.chars().take(20).collect::<String>()
            )))
        }
    }

    fn entity_decl(
        &mut self,
        rest: &str,
        external: bool,
        base: &Base,
        depth: usize,
    ) -> Result<(), DtdError> {
        // PE references may build the declaration itself, outside its literal.
        let text = if rest.contains('%') && external {
            self.expand_pe_in_decl_keep_pe_marker(rest, depth)?
        } else {
            rest.to_string()
        };
        let s = text.trim_start();
        let (is_pe, s) = match s.strip_prefix('%') {
            Some(r) if r.starts_with(|c: char| c.is_whitespace()) => (true, r.trim_start()),
            _ => (false, s),
        };
        let (name, mut i) =
            name_at(s, 0).ok_or_else(|| DtdError("Entity declaration without a name".into()))?;
        let name = name.to_string();
        let b = s.as_bytes();
        while i < b.len() && is_ws(b[i]) {
            i += 1;
        }
        let entity = if b.get(i).is_some_and(|&c| c == b'"' || c == b'\'') {
            let (lit, _) = literal(s, i)?;
            Entity::Internal(self.entity_value(&lit, external, depth)?)
        } else {
            let (public, system, end) = external_id(s, i)?;
            let after = s[end..].trim();
            if let Some(_notation) = after.strip_prefix("NDATA") {
                if is_pe {
                    return Err(DtdError("A parameter entity can't be unparsed".into()));
                }
                Entity::Unparsed {
                    system,
                    base: base.clone(),
                }
            } else {
                Entity::External {
                    public,
                    system,
                    base: base.clone(),
                }
            }
        };
        // The first declaration of a name binds it; later ones are ignored.
        let table = if is_pe {
            &mut self.dtd.parameter
        } else {
            &mut self.dtd.general
        };
        table.entry(name).or_insert(entity);
        Ok(())
    }

    /// Like `expand_pe_in_decl`, but a `%` that starts a parameter-entity
    /// declaration (`<!ENTITY % name …>`) is kept.
    fn expand_pe_in_decl_keep_pe_marker(
        &mut self,
        s: &str,
        depth: usize,
    ) -> Result<String, DtdError> {
        let t = s.trim_start();
        if let Some(r) = t.strip_prefix('%')
            && r.starts_with(|c: char| c.is_whitespace())
        {
            return Ok(format!("% {}", self.expand_outside_literal(r, depth)?));
        }
        self.expand_outside_literal(s, depth)
    }

    /// PE references before the entity's literal value are expanded; the
    /// literal itself is processed by `entity_value`.
    fn expand_outside_literal(&mut self, s: &str, depth: usize) -> Result<String, DtdError> {
        match s.find(['"', '\'']) {
            Some(q) => Ok(format!(
                "{}{}",
                self.expand_pe_in_decl(&s[..q], depth)?,
                &s[q..]
            )),
            None => self.expand_pe_in_decl(s, depth),
        }
    }

    /// An entity's literal value: character references and (in the external
    /// subset) parameter-entity references are expanded now; general entity
    /// references are kept for when the entity is used.
    fn entity_value(
        &mut self,
        lit: &str,
        external: bool,
        depth: usize,
    ) -> Result<String, DtdError> {
        self.check_depth(depth)?;
        let mut out = String::with_capacity(lit.len());
        let mut i = 0;
        while i < lit.len() {
            let rest = &lit[i..];
            if rest.starts_with("&#") {
                let end = rest
                    .find(';')
                    .ok_or_else(|| DtdError("Invalid character reference".into()))?;
                out.push(char_ref(&rest[2..end]).ok_or_else(|| {
                    DtdError(format!("Invalid character reference {}", &rest[..=end]))
                })?);
                i += end + 1;
            } else if rest.starts_with('%') && external {
                let (name, end) =
                    name_at(lit, i + 1).ok_or_else(|| DtdError("Invalid PE reference".into()))?;
                if !lit[end..].starts_with(';') {
                    return Err(DtdError(format!(
                        "PEReference: expecting ';' after %{name}"
                    )));
                }
                if let Some((text, _)) = self.pe_text(name)? {
                    let inner = self.entity_value(strip_text_decl(&text), external, depth + 1)?;
                    out.push_str(&inner);
                }
                i = end + 1;
            } else {
                let c = rest.chars().next().expect("not empty");
                out.push(c);
                i += c.len_utf8();
            }
        }
        Ok(out)
    }

    fn attlist_decl(&mut self, s: &str) -> Result<(), DtdError> {
        let mut t = Tokens { s, i: 0 };
        let element = t
            .name()
            .ok_or_else(|| DtdError("ATTLIST without an element name".into()))?
            .to_string();
        let mut defs = Vec::new();
        while let Some(name) = t.name() {
            let name = name.to_string();
            let kind = if t.peek_char() == Some('(') {
                t.group()?;
                AttType::Tokenized
            } else {
                match t.name() {
                    Some("CDATA") => AttType::Cdata,
                    Some("ID") => AttType::Id,
                    Some("NOTATION") => {
                        t.group()?;
                        AttType::Tokenized
                    }
                    Some("IDREF" | "IDREFS" | "ENTITY" | "ENTITIES" | "NMTOKEN" | "NMTOKENS") => {
                        AttType::Tokenized
                    }
                    other => {
                        return Err(DtdError(format!(
                            "Invalid attribute type {other:?} for {name}"
                        )));
                    }
                }
            };
            let default = match t.peek_char() {
                Some('#') => {
                    t.i += 1;
                    match t.name() {
                        Some("REQUIRED") => AttDefault::Required,
                        Some("IMPLIED") => AttDefault::Implied,
                        Some("FIXED") => AttDefault::Value(t.literal()?),
                        other => {
                            return Err(DtdError(format!("Invalid attribute default #{other:?}")));
                        }
                    }
                }
                Some('"' | '\'') => AttDefault::Value(t.literal()?),
                _ => return Err(DtdError(format!("Missing default for attribute {name}"))),
            };
            defs.push(AttDef {
                name,
                kind,
                default,
            });
        }
        for mut d in defs {
            // The first declaration of an attribute wins.
            if self
                .dtd
                .attlists
                .get(&element)
                .is_some_and(|l| l.iter().any(|x| x.name == d.name))
            {
                continue;
            }
            if let AttDefault::Value(v) = &d.default {
                let mut v = self.normalize_default(v)?;
                if d.kind != AttType::Cdata {
                    v = collapse_spaces(&v);
                }
                d.default = AttDefault::Value(v);
            }
            self.dtd
                .attlists
                .entry(element.clone())
                .or_default()
                .push(d);
        }
        Ok(())
    }

    /// A default value is an attribute value: references are expanded and
    /// whitespace characters become spaces.
    fn normalize_default(&self, v: &str) -> Result<String, DtdError> {
        decode_attribute_value(v, &self.dtd.general, 0).map_err(DtdError)
    }
}

/// The end (the closing `>`) of a declaration starting at `i`, skipping
/// quoted literals.
fn decl_end(s: &str, i: usize) -> Result<usize, DtdError> {
    let mut quote: Option<char> = None;
    for (j, c) in s[i..].char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Ok(i + j),
            _ => {}
        }
    }
    Err(DtdError("Declaration not terminated".into()))
}

struct Tokens<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Tokens<'a> {
    fn skip_ws(&mut self) {
        while self.s.as_bytes().get(self.i).is_some_and(|&b| is_ws(b)) {
            self.i += 1;
        }
    }
    fn peek_char(&mut self) -> Option<char> {
        self.skip_ws();
        self.s[self.i..].chars().next()
    }
    fn name(&mut self) -> Option<&'a str> {
        self.skip_ws();
        let (n, end) = name_at(self.s, self.i)?;
        self.i = end;
        Some(n)
    }
    fn group(&mut self) -> Result<(), DtdError> {
        self.skip_ws();
        let end = self.s[self.i..]
            .find(')')
            .ok_or_else(|| DtdError("Unclosed enumeration".into()))?;
        self.i += end + 1;
        Ok(())
    }
    fn literal(&mut self) -> Result<String, DtdError> {
        let (lit, end) = literal(self.s, self.i)?;
        self.i = end;
        Ok(lit)
    }
}

/// Parses a character reference's digits (`x41` or `65`).
pub fn char_ref(digits: &str) -> Option<char> {
    let n = match digits.strip_prefix('x') {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse().ok()?,
    };
    char::from_u32(n).filter(|&c| matches!(c, '\t' | '\n' | '\r') || c >= ' ')
}

/// Collapses runs of spaces to one and trims both ends: the extra
/// normalisation of non-CDATA attribute values.
/// [`collapse_spaces`] on `s[from..]`, in place.
pub fn collapse_spaces_from(s: &mut String, from: usize) {
    let mut bytes = std::mem::take(s).into_bytes();
    let mut w = from;
    let mut pending_space = false;
    for r in from..bytes.len() {
        let b = bytes[r];
        if b == b' ' {
            pending_space = w > from;
        } else {
            if pending_space {
                bytes[w] = b' ';
                w += 1;
                pending_space = false;
            }
            bytes[w] = b;
            w += 1;
        }
    }
    bytes.truncate(w);
    // Only ASCII spaces were removed, so the bytes are still UTF-8.
    *s = String::from_utf8(bytes).expect("still UTF-8");
}

pub fn collapse_spaces(v: &str) -> String {
    v.split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Decodes a raw attribute value (XML 1.0 section 3.3.3): character
/// references give their character as is; whitespace characters become
/// spaces; entity references are replaced by their (recursively decoded)
/// replacement text. `<` and references to external or undeclared
/// entities are errors.
pub fn decode_attribute_value(
    raw: &str,
    entities: &HashMap<String, Entity>,
    depth: usize,
) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    decode_attribute_value_into(raw, entities, depth, &mut out, None)?;
    Ok(out)
}

/// [`decode_attribute_value`], appending to `out`. With `undeclared`, a
/// reference to an undeclared entity is dropped and a warning added there
/// instead of failing: libxml2's xmlParseEntityRef does that when the
/// document has an external subset or parameter-entity references (XML 1.0
/// section 4.1, WFC: Entity Declared), in attribute values as in content.
pub fn decode_attribute_value_into(
    raw: &str,
    entities: &HashMap<String, Entity>,
    depth: usize,
    out: &mut String,
    mut undeclared: Option<&mut Vec<String>>,
) -> Result<(), String> {
    if depth > 40 {
        return Err("Detected an entity reference loop".into());
    }
    if !raw
        .bytes()
        .any(|b| matches!(b, b'&' | b'<' | b'\t' | b'\n' | b'\r'))
    {
        out.push_str(raw);
        return Ok(());
    }
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        let c = rest.chars().next().expect("not empty");
        match c {
            '&' => {
                let end = rest
                    .find(';')
                    .ok_or_else(|| "EntityRef: expecting ';'".to_string())?;
                let r = &rest[1..end];
                if let Some(digits) = r.strip_prefix('#') {
                    out.push(
                        char_ref(digits)
                            .ok_or_else(|| format!("Invalid character reference &{r};"))?,
                    );
                } else if let Some(p) = predefined(r) {
                    out.push(p);
                } else {
                    match entities.get(r) {
                        Some(Entity::Internal(text)) => {
                            if text.contains('<') {
                                return Err(format!(
                                    "'<' in entity '{r}' is not allowed in attributes values"
                                ));
                            }
                            decode_attribute_value_into(
                                text,
                                entities,
                                depth + 1,
                                out,
                                undeclared.as_deref_mut(),
                            )?;
                        }
                        Some(_) => {
                            return Err(format!("Attribute references external entity '{r}'"));
                        }
                        None => match undeclared.as_deref_mut() {
                            Some(warnings) => warnings.push(format!("Entity '{r}' not defined")),
                            None => return Err(format!("Entity '{r}' not defined")),
                        },
                    }
                }
                i += end + 1;
            }
            '<' => return Err("Unescaped '<' not allowed in attributes values".into()),
            '\t' | '\n' | '\r' => {
                out.push(' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    Ok(())
}

pub fn predefined(name: &str) -> Option<char> {
    Some(match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "apos" => '\'',
        "quot" => '"',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn internal(subset: &str) -> Dtd {
        let mut dtd = Dtd::default();
        parse_doctype(
            &format!("d [{subset}]"),
            &mut dtd,
            &NoLoader,
            &None,
            Limits::default(),
        )
        .unwrap();
        dtd
    }

    #[test]
    fn conditional_section_with_multibyte_text() {
        // Found by fuzzing: scanning for "]]>" byte by byte sliced the str
        // inside a multi-byte character.
        // The section arrives through a parameter entity; whether it's then
        // accepted doesn't matter here, only that parsing returns.
        let mut dtd = Dtd::default();
        let _ = parse_doctype(
            "d [<!ENTITY % p \"<![IGNORE[ \u{2B1B6}é ]]>\"> %p;]",
            &mut dtd,
            &NoLoader,
            &None,
            Limits::default(),
        );
    }

    #[test]
    fn collapse_in_place() {
        for v in ["", " ", "a", "  a  b   c ", "x é  ü ", "ab"] {
            let mut s = String::from("keep  ");
            s.push_str(v);
            collapse_spaces_from(&mut s, 6);
            assert_eq!(s, format!("keep  {}", collapse_spaces(v)), "{v:?}");
        }
    }

    #[test]
    fn entities_and_attlists() {
        let dtd = internal(
            r#"<!ENTITY e "x<b>y</b>&#65;&amp;"> <!ENTITY e "second">
               <!ENTITY % pe "<!ENTITY viaPe 'from a PE'>"> %pe;
               <!ATTLIST p role CDATA "normal" id ID #IMPLIED kind (a|b) " a " >
               <!ATTLIST p role CDATA "ignored">
               <!ENTITY ext SYSTEM "ext.xml"> <!ENTITY pic SYSTEM "pic.png" NDATA png>
               <!-- comment --> <?pi data?> <!ELEMENT p (#PCDATA)>"#,
        );
        assert!(matches!(&dtd.general["e"], Entity::Internal(t) if t == "x<b>y</b>A&amp;"));
        assert!(matches!(&dtd.general["viaPe"], Entity::Internal(t) if t == "from a PE"));
        assert!(matches!(dtd.general["ext"], Entity::External { .. }));
        assert!(matches!(dtd.general["pic"], Entity::Unparsed { .. }));
        let p = &dtd.attlists["p"];
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].default, AttDefault::Value("normal".into()));
        assert_eq!(p[1].kind, AttType::Id);
        assert_eq!(
            p[2].default,
            AttDefault::Value("a".into()),
            "non-CDATA defaults are collapsed"
        );
        assert!(dtd.has_pe_refs);
    }

    #[test]
    fn external_ids_and_conditional_sections() {
        let mut dtd = Dtd::default();
        parse_doctype(
            r#"article PUBLIC "-//X//EN" "x.dtd""#,
            &mut dtd,
            &NoLoader,
            &None,
            Limits::default(),
        )
        .unwrap();
        assert_eq!(
            (dtd.public_id.as_deref(), dtd.system_id.as_deref()),
            (Some("-//X//EN"), Some("x.dtd"))
        );
        assert!(dtd.has_external_subset);

        struct Ext;
        impl Loader for Ext {
            fn load(
                &self,
                _: Option<&str>,
                system: &str,
                _: &Base,
            ) -> Result<(String, Base), String> {
                assert_eq!(system, "x.dtd");
                Ok((
                    r#"<?xml version="1.0" encoding="UTF-8"?>
                    <!ENTITY % draft "INCLUDE"> <!ENTITY % final "IGNORE">
                    <![%draft;[ <!ENTITY state "draft"> <![IGNORE[ <!ENTITY state "nested"> ]]> ]]>
                    <![%final;[ <!ENTITY state "final"> ]]>
                    <!ENTITY % common.attrib "id ID #IMPLIED lang CDATA 'en'">
                    <!ATTLIST para %common.attrib;>"#
                        .into(),
                    None,
                ))
            }
        }
        load_external_subset(&mut dtd, &Ext, &None, Limits::default()).unwrap();
        assert!(matches!(&dtd.general["state"], Entity::Internal(t) if t == "draft"));
        assert_eq!(
            dtd.attdef("para", "lang").unwrap().default,
            AttDefault::Value("en".into())
        );
    }

    #[test]
    fn attribute_values_decode_entities_and_normalise() {
        let mut ents = HashMap::new();
        ents.insert("e".into(), Entity::Internal("a\tb &f;".into()));
        ents.insert("f".into(), Entity::Internal("f".into()));
        ents.insert("loop".into(), Entity::Internal("&loop;".into()));
        assert_eq!(
            decode_attribute_value("x\ny&#10;&e;", &ents, 0).unwrap(),
            "x y\na b f"
        );
        assert!(
            decode_attribute_value("&loop;", &ents, 0)
                .unwrap_err()
                .contains("loop")
        );
        assert!(
            decode_attribute_value("&nope;", &ents, 0)
                .unwrap_err()
                .contains("not defined")
        );
        assert!(decode_attribute_value("a<b", &ents, 0).is_err());
    }

    #[test]
    fn hostile_subsets_hit_the_limits() {
        // A parameter entity that references itself.
        let mut dtd = Dtd::default();
        let r = parse_doctype(
            "d [<!ENTITY % a '%a;'> %a;]",
            &mut dtd,
            &NoLoader,
            &None,
            Limits::default(),
        );
        // Internal-subset PE values aren't expanded at declaration (not
        // external), so %a; expands to "%a;" until the depth limit.
        assert!(r.is_err());
    }
}
