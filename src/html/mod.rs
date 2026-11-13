//! `--html`: libxml2 2.9.14's HTML parser (HTMLparser.c), as xsltproc's
//! `htmlReadFile` runs it, ported function by function.
//!
//! The parser works on a byte buffer as libxml2 does: at first the raw
//! input, read as ASCII; on the first non-ASCII byte it looks ahead for a
//! `<meta http-equiv>` charset or falls back to ISO-8859-1, and a `<meta>`
//! charset switches the rest of the input. Text is copied byte by byte, so
//! libxml2's encoding quirks carry over. The comments name the C functions.
//!
//! libxml2's SAX2 tree builder can attach an element to one finished
//! earlier (an element after `</html>` goes into the first `html`), so the
//! parser builds a small tree of its own and hands it to [`Builder`] in
//! document order at the end.
//!
//! What xsltproc's options mean here: no recovery mode (`</` and a letter
//! end a script), blanks kept, implied `html`, `head`, `body` and `p`
//! added. HTML documents have no namespaces: `xmlns` is an ordinary
//! attribute, and an element name's prefix is dropped (`xmlSplitQName`).
//! `id` attributes, and `name` on `a`, are IDs (`xmlIsID`).

mod tables;
mod tree;
use tree::*;

use crate::encoding::{self, Charset};
use crate::serialize::html::is_boolean;
use crate::tree::{Builder, Document, NamePool, QName, TextKind};

/// HTML_PARSER_BUFFER_SIZE: names are cut at this length.
const BUFFER_SIZE: usize = 100;
/// HTML_PARSER_BIG_BUFFER_SIZE: text is handed over in chunks of this size.
const BIG_BUFFER_SIZE: usize = 1000;
/// xmlParserMaxDepth: a deeper element stops the parse.
const MAX_DEPTH: usize = 256;
/// XML_MAX_TEXT_LENGTH: text past this is dropped from a node.
const MAX_TEXT: usize = 10_000_000;
/// INPUT_CHUNK and MINLEN: libxml2 reads input 4000 bytes at a time when
/// fewer than 250 are left, which bounds `htmlFindEncoding`'s look-ahead.
const INPUT_CHUNK: usize = 250;
const MINLEN: usize = 4000;

pub struct Parsed {
    pub document: Document,
    pub warnings: Vec<String>,
}

/// Parses an HTML document. Like `htmlReadFile`, it never fails: errors
/// are warnings, and the tree is whatever the parser built.
pub fn parse(bytes: &[u8], names: &mut NamePool, file: &str) -> Parsed {
    let mut p = Parser::new(bytes, file);
    p.parse_document();
    let warnings = std::mem::take(&mut p.warnings);
    let mut document = p.tree.build(names);
    document.html = true;
    Parsed { document, warnings }
}

// ---------- character classes ----------

fn is_blank(c: u8) -> bool {
    matches!(c, 0x20 | 0x09 | 0x0A | 0x0D)
}

/// IS_CHAR.
fn is_char(c: u32) -> bool {
    matches!(c, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

/// IS_PUBIDCHAR_CH.
fn is_pubid_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b" \r\n-'()+,./:=?;!*#@$_%".contains(&c)
}

/// IS_LETTER, IS_DIGIT, IS_COMBINING and IS_EXTENDER, approximated by
/// Unicode's classes: they matter only for non-ASCII names in entity
/// references, PI targets and DOCTYPE names.
fn is_letter(c: u32) -> bool {
    char::from_u32(c).is_some_and(char::is_alphabetic)
}

fn is_name_char(c: u32) -> bool {
    char::from_u32(c).is_some_and(|ch| ch.is_alphanumeric())
        || matches!(c, 0x2E | 0x2D | 0x5F | 0x3A | 0xB7)
        || (0x300..=0x36F).contains(&c)
}

/// Appends a character as UTF-8, as the parser's `bits` loops and
/// xmlCopyChar do (any value below 0x110000, surrogates included).
fn push_char(out: &mut Vec<u8>, c: u32) {
    if c < 0x80 {
        out.push(c as u8);
    } else if c < 0x800 {
        out.push((((c >> 6) & 0x1F) | 0xC0) as u8);
        out.push(((c & 0x3F) | 0x80) as u8);
    } else if c < 0x10000 {
        out.push((((c >> 12) & 0x0F) | 0xE0) as u8);
        out.push((((c >> 6) & 0x3F) | 0x80) as u8);
        out.push(((c & 0x3F) | 0x80) as u8);
    } else {
        out.push((((c >> 18) & 0x07) | 0xF0) as u8);
        out.push((((c >> 12) & 0x3F) | 0x80) as u8);
        out.push((((c >> 6) & 0x3F) | 0x80) as u8);
        out.push(((c & 0x3F) | 0x80) as u8);
    }
}

/// COPY_BUF: a one-byte character is copied as the byte it was.
fn copy_buf(out: &mut Vec<u8>, l: usize, c: u32) {
    if l == 1 {
        out.push(c as u8);
    } else {
        push_char(out, c);
    }
}

/// An input converter (input->buf->encoder), fed the raw input a chunk at
/// a time as libxml2 reads it. Like libxml2's converters it stops at the
/// first invalid byte: the parser then sees the end of the input there.
enum Converter {
    Latin1,
    Ascii,
    Rs(encoding_rs::Decoder),
}

impl Converter {
    fn new(charset: Charset) -> Self {
        match charset {
            Charset::Latin1 => Converter::Latin1,
            Charset::Ascii => Converter::Ascii,
            Charset::Utf8 => Converter::Rs(encoding_rs::UTF_8.new_decoder_without_bom_handling()),
            Charset::Utf16 { big_endian, .. } => Converter::Rs(
                if big_endian {
                    encoding_rs::UTF_16BE
                } else {
                    encoding_rs::UTF_16LE
                }
                .new_decoder_without_bom_handling(),
            ),
            Charset::Other(e) => Converter::Rs(e.new_decoder_without_bom_handling()),
        }
    }

    /// Converts a chunk, appending UTF-8 to `out`; false at an error.
    fn convert(&mut self, raw: &[u8], last: bool, out: &mut Vec<u8>) -> bool {
        match self {
            Converter::Latin1 => {
                for &b in raw {
                    push_char(out, u32::from(b));
                }
                true
            }
            Converter::Ascii => {
                let n = raw.iter().take_while(|b| b.is_ascii()).count();
                out.extend_from_slice(&raw[..n]);
                n == raw.len()
            }
            Converter::Rs(d) => {
                let mut s = String::with_capacity(
                    d.max_utf8_buffer_length_without_replacement(raw.len())
                        .unwrap_or(raw.len() * 3 + 16),
                );
                let (result, _) = d.decode_to_string_without_replacement(raw, &mut s, last);
                out.extend_from_slice(s.as_bytes());
                !matches!(result, encoding_rs::DecoderResult::Malformed(..))
            }
        }
    }
}

/// xmlParseCharEncoding: the encoding names libxml2 knows itself, which a
/// `<meta>` charset is looked up in first. UTF-8 here needs no converter.
fn parse_char_encoding(name: &str) -> Option<Charset> {
    let upper = name.to_ascii_uppercase();
    match upper.as_str() {
        // An empty name is XML_CHAR_ENCODING_NONE, taken as UTF-8.
        "" | "UTF-8" | "UTF8" => Some(Charset::Utf8),
        "UTF-16" | "UTF16" => Some(Charset::Utf16 {
            big_endian: false,
            bom: false,
        }),
        "ISO-8859-1" | "ISO-LATIN-1" | "ISO LATIN 1" => Some(Charset::Latin1),
        "ISO-2022-JP" | "SHIFT_JIS" | "EUC-JP" => encoding::lookup(&upper),
        _ => {
            let n = upper
                .strip_prefix("ISO-8859-")
                .or_else(|| upper.strip_prefix("ISO-LATIN-"))
                .or_else(|| upper.strip_prefix("ISO LATIN "))?;
            matches!(n, "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                .then(|| encoding::lookup(&format!("ISO-8859-{n}")))
                .flatten()
        }
    }
}

/// What xmlFindCharEncodingHandler finds for a name.
enum Handler {
    /// libxml2's own UTF-8 handler: no conversion.
    Utf8,
    /// A converter: libxml2's ISO-8859-1, ASCII and UTF-16 handlers, or
    /// iconv's (a strict UTF-8 converter for names like "UTF8" or
    /// "UTF-8//").
    Convert(Charset),
}

/// xmlFindCharEncodingHandler: libxml2's registered handlers by exact
/// (uppercased) name, else iconv. glibc's iconv drops every character but
/// ASCII letters, digits and `_-/.,:`, ignores a suffix after '/', and
/// takes an empty name as the locale's charset: ASCII in the C locale
/// xsltproc runs in.
fn find_handler(name: &str) -> Option<Handler> {
    if name.is_empty() {
        return None;
    }
    match name.to_ascii_uppercase().as_str() {
        "UTF-8" => return Some(Handler::Utf8),
        "ISO-8859-1" => return Some(Handler::Convert(Charset::Latin1)),
        "ASCII" | "US-ASCII" => return Some(Handler::Convert(Charset::Ascii)),
        "UTF-16LE" | "UTF-16" => {
            return Some(Handler::Convert(Charset::Utf16 {
                big_endian: false,
                bom: false,
            }));
        }
        "UTF-16BE" => {
            return Some(Handler::Convert(Charset::Utf16 {
                big_endian: true,
                bom: false,
            }));
        }
        _ => {}
    }
    let kept: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || "_-/.,:".contains(*c))
        .collect();
    let code = kept.split('/').next().unwrap_or("");
    if code.is_empty() {
        return Some(Handler::Convert(Charset::Ascii));
    }
    encoding::lookup(code).map(Handler::Convert)
}

fn element_info(name: &str) -> Option<(u8, u8, bool)> {
    tables::ELEMENTS
        .binary_search_by(|(n, ..)| n.cmp(&name))
        .ok()
        .map(|i| {
            let (_, start, end, empty) = tables::ELEMENTS[i];
            (start, end, empty)
        })
}

/// htmlCheckAutoClose: does the start tag `new` close an open `old`?
fn check_auto_close(new: &str, old: &str) -> bool {
    tables::START_CLOSE
        .binary_search_by(|&(o, n)| o.cmp(old).then(n.cmp(new)))
        .is_ok()
}

/// htmlGetEndPriority.
fn end_priority(name: &str) -> u32 {
    match name {
        "div" => 150,
        "td" | "th" => 160,
        "tr" => 170,
        "thead" | "tbody" | "tfoot" => 180,
        "table" => 190,
        "head" | "body" => 200,
        "html" => 220,
        _ => 100,
    }
}

/// htmlEntityLookup.
fn entity(name: &[u8]) -> Option<u32> {
    tables::ENTITIES
        .iter()
        .find(|(n, _)| n.as_bytes() == name)
        .map(|&(_, v)| v)
}

/// allowPCData: elements whose blank text `areBlanks` keeps.
const ALLOW_PCDATA: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "applet",
    "b",
    "bdo",
    "big",
    "blockquote",
    "body",
    "button",
    "caption",
    "center",
    "cite",
    "code",
    "dd",
    "del",
    "dfn",
    "div",
    "dt",
    "em",
    "font",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "i",
    "iframe",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "noframes",
    "noscript",
    "object",
    "p",
    "pre",
    "q",
    "s",
    "samp",
    "small",
    "span",
    "strike",
    "strong",
    "td",
    "th",
    "tt",
    "u",
    "var",
];

/// xmlSplitQName as xmlSAX2StartElement applies it to HTML element names:
/// the part after the first colon, unless the name starts with a colon or
/// ends with its only colon.
fn local_name(name: &str) -> String {
    // (Its `xml:` exception is compiled out: XML_XML_NAMESPACE is defined.)
    if name.starts_with(':') {
        return name.to_string();
    }
    match name.split_once(':') {
        Some((_, "")) => name.to_string(),
        Some((_, local)) => local.to_string(),
        None => name.to_string(),
    }
}

// ---------- the parser ----------

struct Parser<'a> {
    raw: &'a [u8],
    /// How much of `raw` has been read into `buf` (xmlParserInputGrow
    /// reads 4000 bytes when fewer than 250 are left).
    raw_read: usize,
    /// The parser's input buffer: the raw bytes read, or after an encoding
    /// switch, the bytes before it followed by the rest converted to UTF-8
    /// chunk by chunk with the converter in place when each was read.
    buf: Vec<u8>,
    /// input->buf->encoder.
    converter: Option<Converter>,
    /// A conversion failed: nothing more is read.
    convert_failed: bool,
    /// Counts the times libxml2's input->base would have moved (see
    /// `parse_name_complex`).
    base_moves: u32,
    cur: usize,
    line: u32,
    /// ctxt->charset == XML_CHAR_ENCODING_UTF8.
    utf8: bool,
    /// input->encoding != NULL: a charset was declared or guessed.
    input_encoding: bool,
    /// ctxt->instate == XML_PARSER_EOF.
    eof: bool,
    /// ctxt->disableSAX: set by xmlNextChar's encoding errors (recovery is
    /// off) and when the parser halts. Text, comments, PIs and the DOCTYPE
    /// are dropped from then on; elements are still built, as the HTML
    /// parser calls startElement without checking.
    disable_sax: bool,
    /// ctxt->nameTab.
    names: Vec<String>,
    /// ctxt->html: 3 once a head was opened, 10 once a body was.
    html: u8,
    /// ctxt->depth: discarded html, head and body start tags.
    depth: usize,
    tree: Tree,
    file: &'a str,
    warnings: Vec<String>,
}

impl<'a> Parser<'a> {
    fn new(raw: &'a [u8], file: &'a str) -> Self {
        Parser {
            raw,
            raw_read: 0,
            buf: Vec::new(),
            converter: None,
            convert_failed: false,
            base_moves: 0,
            cur: 0,
            line: 1,
            utf8: false,
            input_encoding: false,
            eof: false,
            disable_sax: false,
            names: Vec::new(),
            html: 1,
            depth: 0,
            tree: Tree::default(),
            file,
            warnings: Vec::new(),
        }
    }

    fn err(&mut self, msg: impl AsRef<str>) {
        self.warnings.push(format!(
            "{}:{}: HTML parser error : {}",
            self.file,
            self.line,
            msg.as_ref().trim_end()
        ));
    }

    // ----- input macros -----

    fn encoder(&self) -> bool {
        self.converter.is_some()
    }

    /// Reads the next 4000 bytes of input, converted if there's a
    /// converter (xmlParserInputBufferGrow). Whether anything came.
    fn read_chunk(&mut self) -> bool {
        if self.convert_failed || self.raw_read >= self.raw.len() {
            return false;
        }
        let end = (self.raw_read + MINLEN).min(self.raw.len());
        let chunk = &self.raw[self.raw_read..end];
        match &mut self.converter {
            None => self.buf.extend_from_slice(chunk),
            Some(c) => {
                if !c.convert(chunk, end == self.raw.len(), &mut self.buf) {
                    self.convert_failed = true;
                    self.err("input conversion failed due to input error");
                }
            }
        }
        self.raw_read = end;
        self.base_moves += 1;
        true
    }

    /// xmlParserInputGrow: reads a chunk if at most 250 bytes are left.
    /// libxml2 reads only here, so how far the input has been read (which
    /// bounds `htmlFindEncoding` and decides which input a later charset
    /// switch converts) follows where the parser calls it.
    fn input_grow(&mut self) -> bool {
        if self.buf.len().saturating_sub(self.cur) > INPUT_CHUNK {
            return false;
        }
        self.read_chunk()
    }

    /// GROW: the same, with fewer than 250 bytes left.
    fn grow(&mut self) {
        if self.buf.len().saturating_sub(self.cur) < INPUT_CHUNK {
            self.input_grow();
        }
    }

    /// CUR, NXT(n): 0 past the end.
    fn at(&self, n: usize) -> u8 {
        self.buf.get(self.cur + n).copied().unwrap_or(0)
    }

    fn upp(&self, n: usize) -> u8 {
        self.at(n).to_ascii_uppercase()
    }

    fn end(&self) -> bool {
        self.cur >= self.buf.len()
    }

    /// SKIP(n): ASCII known not to hold newlines.
    fn skip(&mut self, n: usize) {
        self.cur = (self.cur + n).min(self.buf.len());
    }

    /// NEXTL(l).
    fn nextl(&mut self, l: usize) {
        if self.at(0) == b'\n' {
            self.line += 1;
        }
        self.skip(l);
    }

    /// SKIP_BLANKS (htmlSkipBlankChars).
    fn skip_blanks(&mut self) -> usize {
        let mut n = 0;
        while is_blank(self.at(0)) {
            if self.at(0) == b'\n' {
                self.line += 1;
            }
            self.cur += 1;
            n += 1;
            if self.at(0) == 0 {
                self.input_grow();
            }
        }
        n
    }

    /// NEXT (xmlNextChar). At a NUL byte (or the end of what's been read)
    /// it reads more, and doesn't move if nothing came.
    fn next(&mut self) {
        if self.eof {
            return;
        }
        if self.at(0) == 0 && !self.input_grow() {
            return;
        }
        self.next_inner();
        if self.at(0) == 0 {
            self.input_grow();
        }
    }

    fn next_inner(&mut self) {
        if self.at(0) == b'\n' {
            self.line += 1;
        }
        if !self.utf8 {
            self.skip(1);
            return;
        }
        let c = self.at(0);
        if c & 0x80 == 0 {
            self.skip(1);
            return;
        }
        self.grow_for_sequence(c);
        let bad = c == 0xC0
            || self.at(1) & 0xC0 != 0x80
            || (c & 0xE0 == 0xE0
                && (self.at(2) & 0xC0 != 0x80
                    || (c & 0xF0 == 0xF0 && (c & 0xF8 != 0xF0 || self.at(3) & 0xC0 != 0x80))));
        if bad {
            self.err("Input is not proper UTF-8, indicate encoding !");
            self.disable_sax = true;
            // Only the flag changes: the next non-ASCII character makes
            // htmlCurrentChar switch.
            self.utf8 = false;
            self.skip(1);
            return;
        }
        let len = if c & 0xF0 == 0xF0 {
            4
        } else if c & 0xE0 == 0xE0 {
            3
        } else {
            2
        };
        if len > 2 {
            let val = if len == 4 {
                (u32::from(c & 0x7) << 18)
                    | (u32::from(self.at(1) & 0x3F) << 12)
                    | (u32::from(self.at(2) & 0x3F) << 6)
                    | u32::from(self.at(3) & 0x3F)
            } else {
                (u32::from(c & 0xF) << 12)
                    | (u32::from(self.at(1) & 0x3F) << 6)
                    | u32::from(self.at(2) & 0x3F)
            };
            if (0xD800..0xE000).contains(&val)
                || (0xFFFE..0x10000).contains(&val)
                || val >= 0x110000
            {
                self.err(format!("Char 0x{val:X} out of allowed range"));
                self.disable_sax = true;
            }
        }
        self.skip(len);
    }

    /// Switches the input to `charset` (xmlSwitchEncoding and
    /// xmlSwitchToEncoding). With no converter yet, the buffer from here on
    /// is the raw input read so far, which is converted from here (for HTML
    /// libxml2 converts all of it at once). With a converter already, libxml2
    /// only swaps it: input already read stays as it was converted.
    fn switch_to(&mut self, charset: Charset) {
        if charset == Charset::Utf8 {
            self.utf8 = true;
            return;
        }
        self.switch_to_handler(charset);
    }

    /// xmlSwitchToEncoding: always a converter, UTF-8 included.
    fn switch_to_handler(&mut self, charset: Charset) {
        self.utf8 = true;
        if let Some(c) = &mut self.converter {
            *c = Converter::new(charset);
            return;
        }
        self.base_moves += 1;
        let mut conv = Converter::new(charset);
        let tail = &self.raw[self.cur.min(self.raw_read)..self.raw_read];
        let mut converted = Vec::new();
        let ok = conv.convert(tail, self.raw_read == self.raw.len(), &mut converted);
        self.buf.truncate(self.cur);
        self.buf.extend_from_slice(&converted);
        self.converter = Some(conv);
        if !ok {
            self.convert_failed = true;
            self.err("input conversion failed due to input error");
        }
    }

    /// htmlFindEncoding: a charset in a `http-equiv` meta ahead, in what's
    /// been read so far.
    fn find_encoding(&self) -> Option<String> {
        if self.input_encoding || self.encoder() {
            return None;
        }
        let hay = &self.buf[self.cur.min(self.buf.len())..];
        let find = |h: &[u8], needle: &[u8]| {
            h.windows(needle.len())
                .position(|w| w.eq_ignore_ascii_case(needle))
        };
        // The C string stops at a NUL byte.
        let hay = &hay[..hay.iter().position(|&b| b == 0).unwrap_or(hay.len())];
        let i = find(hay, b"HTTP-EQUIV")?;
        let j = i + find(&hay[i..], b"CONTENT")?;
        let k = j + find(&hay[j..], b"CHARSET=")? + 8;
        let len = hay[k..]
            .iter()
            .take_while(|&&c| c.is_ascii_alphanumeric() || b"-_:/".contains(&c))
            .count();
        (len > 0).then(|| String::from_utf8_lossy(&hay[k..k + len]).into_owned())
    }

    /// The reads xmlNextChar and htmlCurrentChar do when a multi-byte
    /// sequence runs into the end of what's been read.
    fn grow_for_sequence(&mut self, c: u8) {
        if self.at(1) == 0 {
            self.input_grow();
        }
        if c & 0xE0 == 0xE0 && self.at(2) == 0 {
            self.input_grow();
        }
        if c & 0xF0 == 0xF0 && self.at(3) == 0 {
            self.input_grow();
        }
    }

    /// CUR_CHAR (htmlCurrentChar): the character here and its length.
    fn cur_char(&mut self) -> (u32, usize) {
        if self.eof {
            return (0, 0);
        }
        if !self.utf8 {
            let c = self.at(0);
            if c < 0x80 {
                if c == 0 && !self.end() {
                    self.err("Char 0x0 out of allowed range");
                    return (0x20, 1);
                }
                return (u32::from(c), 1);
            }
            // A non-ASCII byte before any declared encoding: guess.
            match self.find_encoding() {
                None => self.switch_to(Charset::Latin1),
                Some(guess) => {
                    self.input_encoding = true;
                    match find_handler(&guess) {
                        Some(Handler::Utf8) => {}
                        Some(Handler::Convert(cs)) => self.switch_to_handler(cs),
                        None => self.err(format!("Unsupported encoding {guess}")),
                    }
                }
            }
            self.utf8 = true;
        }
        let c = self.at(0);
        if c & 0x80 != 0 {
            self.grow_for_sequence(c);
            let decoded = (|| {
                if c & 0x40 == 0 || self.at(1) & 0xC0 != 0x80 {
                    return None;
                }
                let c1 = u32::from(self.at(1) & 0x3F);
                if c & 0xE0 == 0xE0 {
                    if self.at(2) & 0xC0 != 0x80 {
                        return None;
                    }
                    let c2 = u32::from(self.at(2) & 0x3F);
                    if c & 0xF0 == 0xF0 {
                        if c & 0xF8 != 0xF0 || self.at(3) & 0xC0 != 0x80 {
                            return None;
                        }
                        let v = (u32::from(c & 0x7) << 18)
                            | (c1 << 12)
                            | (c2 << 6)
                            | u32::from(self.at(3) & 0x3F);
                        (v >= 0x10000).then_some((v, 4))
                    } else {
                        let v = (u32::from(c & 0xF) << 12) | (c1 << 6) | c2;
                        (v >= 0x800).then_some((v, 3))
                    }
                } else {
                    let v = (u32::from(c & 0x1F) << 6) | c1;
                    (v >= 0x80).then_some((v, 2))
                }
            })();
            if let Some((v, l)) = decoded {
                if !is_char(v) {
                    self.err(format!("Char 0x{v:X} out of allowed range"));
                }
                return (v, l);
            }
            // encoding_error: switch to Latin-1 if no encoder yet, and
            // return the byte now here, as one byte.
            self.err("Input is not proper UTF-8, indicate encoding !");
            if !self.encoder() {
                self.switch_to(Charset::Latin1);
            }
            return (u32::from(self.at(0)), 1);
        }
        if c == 0 && !self.end() {
            self.err("Char 0x0 out of allowed range");
            return (0x20, 1);
        }
        (u32::from(c), 1)
    }

    // ----- the name stack and SAX events -----

    /// htmlnamePush.
    fn name_push(&mut self, name: &str) {
        if self.html < 3 && name == "head" {
            self.html = 3;
        }
        if self.html < 10 && name == "body" {
            self.html = 10;
        }
        self.names.push(name.to_string());
    }

    fn name(&self) -> Option<&str> {
        self.names.last().map(String::as_str)
    }

    /// sax->startElement (xmlSAX2StartElement).
    fn start_element(&mut self, name: &str, attrs: Vec<(String, Option<Vec<u8>>)>) {
        let attrs = attrs
            .into_iter()
            .map(|(a, v)| {
                // Boolean attributes without a value get their name.
                let v = match v {
                    None if is_boolean(&a) => Some(a.as_bytes().to_vec()),
                    v => v,
                };
                (a, v)
            })
            .collect();
        if !self
            .tree
            .start_element(local_name(name), attrs, self.line.min(65535))
        {
            // nodePush failed: "Excessive depth in document", and
            // xmlHaltParser empties the input.
            self.err(format!(
                "Excessive depth in document: {MAX_DEPTH} use XML_PARSE_HUGE option"
            ));
            self.eof = true;
            self.disable_sax = true;
            self.buf.truncate(self.cur);
        }
    }

    /// sax->endElement and htmlnamePop.
    fn end_and_pop(&mut self) {
        self.tree.end_element();
        self.names.pop();
    }

    fn characters(&mut self, s: &[u8]) {
        self.tree.text(s, false);
    }

    // ----- auto-closing and implied elements -----

    /// htmlAutoCloseOnClose.
    fn auto_close_on_close(&mut self, newtag: &str) {
        let priority = end_priority(newtag);
        let mut found = false;
        for n in self.names.iter().rev() {
            if n == newtag {
                found = true;
                break;
            }
            if end_priority(n) > priority {
                return;
            }
        }
        if !found {
            return;
        }
        while self.name() != Some(newtag) {
            let Some(name) = self.name().map(str::to_string) else {
                break;
            };
            if element_info(&name).is_some_and(|(_, end, _)| end == 3) {
                self.err(format!(
                    "Opening and ending tag mismatch: {newtag} and {name}"
                ));
            }
            self.end_and_pop();
        }
    }

    /// htmlAutoCloseOnEnd.
    fn auto_close_on_end(&mut self) {
        while !self.names.is_empty() {
            self.end_and_pop();
        }
    }

    /// htmlAutoClose, for a new tag.
    fn auto_close(&mut self, newtag: &str) {
        while let Some(name) = self.name() {
            if !check_auto_close(newtag, name) {
                break;
            }
            self.end_and_pop();
        }
    }

    /// htmlCheckImplied.
    fn check_implied(&mut self, newtag: &str) {
        if newtag == "html" {
            return;
        }
        if self.names.is_empty() {
            self.name_push("html");
            self.start_element("html", Vec::new());
        }
        if newtag == "body" || newtag == "head" {
            return;
        }
        if self.names.len() <= 1
            && matches!(
                newtag,
                "script" | "style" | "meta" | "link" | "title" | "base"
            )
        {
            if self.html >= 3 {
                return;
            }
            self.name_push("head");
            self.start_element("head", Vec::new());
        } else if !matches!(newtag, "noframes" | "frame" | "frameset") {
            if self.html >= 10 {
                return;
            }
            if self.names.iter().any(|n| n == "body" || n == "head") {
                return;
            }
            self.name_push("body");
            self.start_element("body", Vec::new());
        }
    }

    /// htmlCheckParagraph.
    fn check_paragraph(&mut self) {
        let implied = match self.name() {
            None => true,
            Some(tag) => tag == "html" || tag == "head",
        };
        if implied {
            self.auto_close("p");
            self.check_implied("p");
            self.name_push("p");
            self.start_element("p", Vec::new());
        }
    }

    /// areBlanks: is this blank text that shouldn't imply a paragraph?
    fn are_blanks(&self, s: &[u8]) -> bool {
        if !s.iter().all(|&c| is_blank(c)) {
            return false;
        }
        if self.at(0) == 0 {
            return true;
        }
        if self.at(0) != b'<' {
            return false;
        }
        let Some(name) = self.name() else {
            return true;
        };
        if name == "html" || name == "head" {
            return true;
        }
        if name == "body"
            && let Some(id) = &self.tree.external_id
            && (id.eq_ignore_ascii_case(b"-//W3C//DTD HTML 4.01//EN")
                || id.eq_ignore_ascii_case(b"-//W3C//DTD HTML 4//EN"))
        {
            return true;
        }
        let Some(node) = self.tree.current() else {
            return false;
        };
        let last = self.tree.nodes[node]
            .children
            .iter()
            .rev()
            .find(|&&c| !matches!(self.tree.nodes[c].kind, Kind::Comment(_)));
        match last {
            None => !ALLOW_PCDATA.contains(&name),
            Some(&c) => match &self.tree.nodes[c].kind {
                Kind::Text(_) => false,
                Kind::Element { name, .. } => !ALLOW_PCDATA.contains(&name.as_str()),
                Kind::Pi { target, .. } => !ALLOW_PCDATA.contains(&target.as_str()),
                _ => true,
            },
        }
    }

    // ----- names -----

    /// htmlParseHTMLName: a tag or attribute name, lowercased, at most 100
    /// characters.
    fn parse_html_name(&mut self) -> Option<String> {
        let c = self.at(0);
        if !c.is_ascii_alphabetic() && c != b'_' && c != b':' && c != b'.' {
            return None;
        }
        let mut s = String::new();
        while s.len() < BUFFER_SIZE {
            let c = self.at(0);
            if !(c.is_ascii_alphanumeric() || matches!(c, b':' | b'-' | b'_' | b'.')) {
                break;
            }
            s.push(char::from(c.to_ascii_lowercase()));
            self.next();
        }
        Some(s)
    }

    /// htmlParseHTMLName_nonInvasive: the name after '<', without moving.
    fn peek_html_name(&self) -> Option<String> {
        let c = self.at(1);
        if !c.is_ascii_alphabetic() && c != b'_' && c != b':' {
            return None;
        }
        let mut s = String::new();
        while s.len() < BUFFER_SIZE {
            let c = self.at(1 + s.len());
            if !(c.is_ascii_alphanumeric() || matches!(c, b':' | b'-' | b'_')) {
                break;
            }
            s.push(char::from(c.to_ascii_lowercase()));
        }
        Some(s)
    }

    /// htmlParseName: a case-sensitive name (entities, PI targets, the
    /// DOCTYPE name).
    fn parse_name(&mut self) -> Option<Vec<u8>> {
        self.grow();
        let start = self.cur;
        let c = self.at(0);
        if c.is_ascii_alphabetic() || c == b'_' || c == b':' {
            let mut i = start + 1;
            while i < self.buf.len()
                && (self.buf[i].is_ascii_alphanumeric()
                    || matches!(self.buf[i], b'_' | b'-' | b':' | b'.'))
            {
                i += 1;
            }
            // A name running to the end of the input isn't one.
            if i == self.buf.len() {
                return None;
            }
            if self.buf[i] > 0 && self.buf[i] < 0x80 {
                self.cur = i;
                return Some(self.buf[start..i].to_vec());
            }
        }
        self.parse_name_complex()
    }

    /// htmlParseNameComplex. If reading switches the encoding, it starts
    /// again from where it is, as libxml2 does.
    fn parse_name_complex(&mut self) -> Option<Vec<u8>> {
        loop {
            // libxml2 restarts when input->base moves: a new buffer after an
            // encoding switch, or (taken here as always) a read that grows
            // the buffer. `base` is taken before the first GROW, as in C.
            let base = self.base_moves;
            self.grow();
            let (mut c, mut l) = self.cur_char();
            if c == 0x20
                || c == u32::from(b'>')
                || c == u32::from(b'/')
                || (!is_letter(c) && c != 0x5F && c != 0x3A)
            {
                return None;
            }
            let mut len = 0;
            let mut count = 0;
            let mut restarted = false;
            while c != 0x20 && c != u32::from(b'>') && c != u32::from(b'/') && is_name_char(c) {
                count += 1;
                if count > 100 {
                    count = 0;
                    self.grow();
                }
                len += l;
                self.nextl(l);
                (c, l) = self.cur_char();
                if self.base_moves != base {
                    restarted = true;
                    break;
                }
            }
            if restarted {
                continue;
            }
            return Some(self.buf[self.cur - len..self.cur].to_vec());
        }
    }

    // ----- values and references -----

    /// htmlParseCharRef: the value, or 0 if it isn't a character.
    fn parse_char_ref(&mut self) -> u32 {
        let mut val: u32 = 0;
        if self.at(0) == b'&' && self.at(1) == b'#' && matches!(self.at(2), b'x' | b'X') {
            self.skip(3);
            while self.at(0) != b';' {
                let d = match self.at(0) {
                    c @ b'0'..=b'9' => c - b'0',
                    c @ b'a'..=b'f' => c - b'a' + 10,
                    c @ b'A'..=b'F' => c - b'A' + 10,
                    _ => {
                        self.err("htmlParseCharRef: missing semicolon");
                        break;
                    }
                };
                if val < 0x110000 {
                    val = val * 16 + u32::from(d);
                }
                self.next();
            }
            if self.at(0) == b';' {
                self.next();
            }
        } else if self.at(0) == b'&' && self.at(1) == b'#' {
            self.skip(2);
            while self.at(0) != b';' {
                match self.at(0) {
                    c @ b'0'..=b'9' => {
                        if val < 0x110000 {
                            val = val * 10 + u32::from(c - b'0');
                        }
                    }
                    _ => {
                        self.err("htmlParseCharRef: missing semicolon");
                        break;
                    }
                }
                self.next();
            }
            if self.at(0) == b';' {
                self.next();
            }
        } else {
            self.err("htmlParseCharRef: invalid value");
        }
        if is_char(val) {
            val
        } else {
            self.err(format!("htmlParseCharRef: invalid xmlChar value {val}"));
            0
        }
    }

    /// htmlParseEntityRef: (the entity's value, its name).
    fn parse_entity_ref(&mut self) -> (Option<u32>, Option<Vec<u8>>) {
        if self.at(0) != b'&' {
            return (None, None);
        }
        self.next();
        let Some(name) = self.parse_name() else {
            self.err("htmlParseEntityRef: no name");
            return (None, None);
        };
        self.grow();
        if self.at(0) == b';' {
            let ent = entity(&name);
            if ent.is_some() {
                self.next();
            }
            (ent, Some(name))
        } else {
            self.err("htmlParseEntityRef: expecting ';'");
            (None, Some(name))
        }
    }

    /// htmlParseHTMLAttribute: a value up to `stop`, or with 0 up to a
    /// blank or '>'. References are replaced.
    fn parse_html_attribute(&mut self, stop: u8) -> Vec<u8> {
        let mut out = Vec::new();
        while self.at(0) != 0 && self.at(0) != stop {
            if stop == 0 && (self.at(0) == b'>' || is_blank(self.at(0))) {
                break;
            }
            if self.at(0) == b'&' {
                if self.at(1) == b'#' {
                    let c = self.parse_char_ref();
                    push_char(&mut out, c);
                } else {
                    match self.parse_entity_ref() {
                        (_, None) => out.push(b'&'),
                        (None, Some(name)) => {
                            out.push(b'&');
                            out.extend_from_slice(&name);
                        }
                        (Some(c), Some(_)) => push_char(&mut out, c),
                    }
                }
            } else {
                let (c, _) = self.cur_char();
                push_char(&mut out, c);
                self.next();
            }
        }
        // The value is a C string: an invalid character reference (0)
        // ends it.
        if let Some(i) = out.iter().position(|&b| b == 0) {
            out.truncate(i);
        }
        out
    }

    /// htmlParseAttValue.
    fn parse_att_value(&mut self) -> Vec<u8> {
        let q = self.at(0);
        if q == b'"' || q == b'\'' {
            self.next();
            let v = self.parse_html_attribute(q);
            if self.at(0) != q {
                self.err(format!("AttValue: {} expected", char::from(q)));
            } else {
                self.next();
            }
            v
        } else {
            self.parse_html_attribute(0)
        }
    }

    /// htmlParseAttribute: the name, and the value if there's an '='.
    fn parse_attribute(&mut self) -> Option<(String, Option<Vec<u8>>)> {
        let Some(name) = self.parse_html_name() else {
            self.err("error parsing attribute name");
            return None;
        };
        self.skip_blanks();
        let mut value = None;
        if self.at(0) == b'=' {
            self.next();
            self.skip_blanks();
            value = Some(self.parse_att_value());
        }
        Some((name, value))
    }

    // ----- encoding declarations -----

    /// htmlCheckEncodingDirect.
    fn check_encoding_direct(&mut self, encoding: &[u8]) {
        if self.input_encoding {
            return;
        }
        let start = encoding
            .iter()
            .position(|&c| c != b' ' && c != b'\t')
            .unwrap_or(encoding.len());
        // A C string: up to a NUL byte.
        let encoding = &encoding[start..];
        let encoding = &encoding[..encoding
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(encoding.len())];
        let name = String::from_utf8_lossy(encoding).into_owned();
        self.input_encoding = true;
        match parse_char_encoding(&name) {
            Some(Charset::Utf16 { .. }) if !self.encoder() => {
                self.err("htmlCheckEncoding: wrong encoding meta");
                self.utf8 = true;
            }
            Some(cs) => self.switch_to(cs),
            None => match find_handler(&name) {
                Some(Handler::Utf8) => self.utf8 = true,
                Some(Handler::Convert(cs)) => self.switch_to_handler(cs),
                None => self.err(format!("htmlCheckEncoding: unknown encoding {name}")),
            },
        }
    }

    /// htmlCheckEncoding: `charset=` in a Content-Type value.
    fn check_encoding(&mut self, value: &[u8]) {
        let find = |h: &[u8], n: &[u8]| h.windows(n.len()).position(|w| w.eq_ignore_ascii_case(n));
        let mut at = find(value, b"charset").map(|i| i + 7);
        if let Some(i) = at
            && value.get(i).is_some_and(|&c| is_blank(c))
        {
            at = find(value, b"=");
        }
        if let Some(i) = at
            && value.get(i) == Some(&b'=')
        {
            self.check_encoding_direct(&value[i + 1..]);
        }
    }

    /// htmlCheckMeta.
    fn check_meta(&mut self, attrs: &[(String, Option<Vec<u8>>)]) {
        let mut http = false;
        let mut content = None;
        for (a, v) in attrs {
            let Some(v) = v else { continue };
            if a.eq_ignore_ascii_case("http-equiv") && v.eq_ignore_ascii_case(b"Content-Type") {
                http = true;
            } else if a.eq_ignore_ascii_case("charset") {
                self.check_encoding_direct(v);
            } else if a.eq_ignore_ascii_case("content") {
                content = Some(v.clone());
            }
        }
        if http && let Some(c) = content {
            self.check_encoding(&c);
        }
    }

    // ----- markup -----

    /// htmlParseStartTag: Ok(discarded) or Err(()) on failure.
    fn parse_start_tag(&mut self) -> Result<bool, ()> {
        if self.eof || self.at(0) != b'<' {
            return Err(());
        }
        self.next();
        self.grow();
        let Some(name) = self.parse_html_name() else {
            self.err("htmlParseStartTag: invalid element name");
            while self.at(0) != 0 && self.at(0) != b'>' && !self.eof {
                self.next();
            }
            return Err(());
        };
        let meta = name == "meta";
        self.auto_close(&name);
        self.check_implied(&name);
        let mut discard = false;
        if !self.names.is_empty() && name == "html" {
            self.err("htmlParseStartTag: misplaced <html> tag");
            discard = true;
            self.depth += 1;
        }
        if self.names.len() != 1 && name == "head" {
            self.err("htmlParseStartTag: misplaced <head> tag");
            discard = true;
            self.depth += 1;
        }
        if name == "body" {
            let bodies = self.names.iter().filter(|n| *n == "body").count();
            for _ in 0..bodies {
                self.err("htmlParseStartTag: misplaced <body> tag");
                discard = true;
                self.depth += 1;
            }
        }
        let mut attrs: Vec<(String, Option<Vec<u8>>)> = Vec::new();
        self.skip_blanks();
        while self.at(0) != 0 && self.at(0) != b'>' && (self.at(0) != b'/' || self.at(1) != b'>') {
            self.grow();
            match self.parse_attribute() {
                Some((a, v)) => {
                    if attrs.iter().any(|(n, _)| *n == a) {
                        self.err(format!("Attribute {a} redefined"));
                    } else {
                        attrs.push((a, v));
                    }
                }
                None => {
                    while self.at(0) != 0
                        && !is_blank(self.at(0))
                        && self.at(0) != b'>'
                        && (self.at(0) != b'/' || self.at(1) != b'>')
                    {
                        self.next();
                    }
                }
            }
            self.skip_blanks();
        }
        if meta && !attrs.is_empty() {
            self.check_meta(&attrs);
        }
        if !discard {
            self.name_push(&name);
            self.start_element(&name, attrs);
        }
        Ok(discard)
    }

    /// htmlParseEndTag: whether the current element was closed.
    fn parse_end_tag(&mut self) -> bool {
        if self.at(0) != b'<' || self.at(1) != b'/' {
            self.err("htmlParseEndTag: '</' not found");
            return false;
        }
        self.skip(2);
        let Some(name) = self.parse_html_name() else {
            return false;
        };
        self.skip_blanks();
        if self.at(0) != b'>' {
            self.err("End tag : expected '>'");
            while self.at(0) != 0 && self.at(0) != b'>' {
                self.next();
            }
        }
        if self.at(0) == b'>' {
            self.next();
        }
        if self.depth > 0 && matches!(name.as_str(), "html" | "body" | "head") {
            self.depth -= 1;
            return false;
        }
        if !self.names.contains(&name) {
            self.err(format!("Unexpected end tag : {name}"));
            return false;
        }
        self.auto_close_on_close(&name);
        if let Some(cur) = self.name()
            && cur != name
        {
            self.err(format!("Opening and ending tag mismatch: {name} and {cur}"));
        }
        if self.name() == Some(name.as_str()) {
            self.end_and_pop();
            true
        } else {
            false
        }
    }

    /// htmlParseReference.
    fn parse_reference(&mut self) {
        if self.at(0) != b'&' {
            return;
        }
        if self.at(1) == b'#' {
            let c = self.parse_char_ref();
            if c == 0 {
                return;
            }
            let mut out = Vec::new();
            push_char(&mut out, c);
            self.check_paragraph();
            self.characters(&out);
            return;
        }
        match self.parse_entity_ref() {
            (_, None) => {
                self.check_paragraph();
                self.characters(b"&");
            }
            (Some(c), Some(_)) if c > 0 => {
                let mut out = Vec::new();
                push_char(&mut out, c);
                self.check_paragraph();
                self.characters(&out);
            }
            (_, Some(name)) => {
                self.check_paragraph();
                self.characters(b"&");
                self.characters(&name);
            }
        }
    }

    /// htmlParseScript: script or style content, as CDATA, up to `</` and
    /// a letter.
    fn parse_script(&mut self) {
        let mut buf = Vec::new();
        let (mut c, mut l) = self.cur_char();
        while c != 0 {
            if c == u32::from(b'<') && self.at(1) == b'/' && self.at(2).is_ascii_alphabetic() {
                break;
            }
            if is_char(c) {
                copy_buf(&mut buf, l, c);
            } else {
                self.err(format!("Invalid char in CDATA 0x{c:X}"));
            }
            if buf.len() >= BIG_BUFFER_SIZE {
                // (This flush doesn't check disableSAX, the last one does.)
                self.tree.text(&buf, true);
                buf.clear();
            }
            self.grow();
            self.nextl(l);
            (c, l) = self.cur_char();
        }
        if !buf.is_empty() && !self.disable_sax {
            self.tree.text(&buf, true);
        }
    }

    /// Hands over a chunk of text (the flush in htmlParseCharDataInternal).
    fn flush_text(&mut self, buf: &[u8]) {
        if self.disable_sax {
            return;
        }
        if !self.are_blanks(buf) {
            self.check_paragraph();
        }
        self.characters(buf);
    }

    /// htmlParseCharDataInternal.
    fn parse_char_data(&mut self) {
        let mut buf = Vec::new();
        let mut chunk = 0;
        let (mut c, mut l) = self.cur_char();
        while c != u32::from(b'<') && c != u32::from(b'&') && c != 0 {
            if !is_char(c) {
                self.err(format!("Invalid char in CDATA 0x{c:X}"));
            } else {
                copy_buf(&mut buf, l, c);
            }
            if buf.len() >= BIG_BUFFER_SIZE {
                let text = std::mem::take(&mut buf);
                self.flush_text(&text);
            }
            self.nextl(l);
            chunk += 1;
            if chunk > BUFFER_SIZE {
                chunk = 0;
                self.grow();
            }
            (c, l) = self.cur_char();
            if c == 0 {
                self.grow();
                (c, l) = self.cur_char();
            }
        }
        if !buf.is_empty() {
            self.flush_text(&buf);
        } else if c == 0 {
            self.eof = true;
        }
    }

    /// htmlParseSystemLiteral and htmlParsePubidLiteral.
    fn parse_literal(&mut self, pubid: bool) -> Option<Vec<u8>> {
        let q = self.at(0);
        if q != b'"' && q != b'\'' {
            self.err("SystemLiteral \" or ' expected");
            return None;
        }
        self.next();
        let start = self.cur;
        let mut bad = false;
        while self.at(0) != 0 && self.at(0) != q {
            let c = self.at(0);
            if (pubid && !is_pubid_char(c)) || (!pubid && !is_char(u32::from(c))) {
                bad = true;
            }
            self.next();
        }
        if self.at(0) != q {
            self.err("Unfinished literal");
            return None;
        }
        let v = self.buf[start..self.cur].to_vec();
        self.next();
        (!bad).then_some(v)
    }

    /// htmlParseExternalID: (system ID, public ID).
    fn parse_external_id(&mut self) -> Option<Vec<u8>> {
        let word = |p: &Self, w: &[u8]| (0..w.len()).all(|i| p.upp(i) == w[i]);
        let mut public = None;
        if word(self, b"SYSTEM") {
            self.skip(6);
            self.skip_blanks();
            self.parse_literal(false);
        } else if word(self, b"PUBLIC") {
            self.skip(6);
            self.skip_blanks();
            public = self.parse_literal(true);
            self.skip_blanks();
            if matches!(self.at(0), b'"' | b'\'') {
                self.parse_literal(false);
            }
        }
        public
    }

    /// htmlParseDocTypeDecl, after `<!DOCTYPE` was seen.
    fn parse_doctype(&mut self) {
        self.skip(9);
        self.skip_blanks();
        if self.parse_name().is_none() {
            self.err("htmlParseDocTypeDecl : no DOCTYPE name !");
        }
        self.skip_blanks();
        let public = self.parse_external_id();
        self.skip_blanks();
        if self.at(0) != b'>' {
            self.err("DOCTYPE improperly terminated");
            while self.at(0) != 0 && self.at(0) != b'>' {
                self.next();
            }
        }
        if self.at(0) == b'>' {
            self.next();
        }
        if !self.disable_sax {
            self.tree.doctype(public);
        }
    }

    fn at_doctype(&self) -> bool {
        self.at(0) == b'<' && self.at(1) == b'!' && (2..9).all(|i| self.upp(i) == b"DOCTYPE"[i - 2])
    }

    /// htmlParsePI: `<?target data>`, ending at the first '>'.
    fn parse_pi(&mut self) {
        if self.at(0) != b'<' || self.at(1) != b'?' {
            return;
        }
        self.skip(2);
        let Some(target) = self.parse_name() else {
            self.err("PI is not started correctly");
            return;
        };
        let target = lossy(&target);
        let line = self.line.min(65535);
        if self.at(0) == b'>' {
            self.skip(1);
            if !self.disable_sax {
                self.tree.misc(
                    Kind::Pi {
                        target,
                        data: Vec::new(),
                    },
                    line,
                );
            }
            return;
        }
        if !is_blank(self.at(0)) {
            self.err(format!("ParsePI: PI {target} space expected"));
        }
        self.skip_blanks();
        let mut buf = Vec::new();
        let mut count = 0;
        let (mut c, mut l) = self.cur_char();
        while c != 0 && c != u32::from(b'>') {
            count += 1;
            if count > 50 {
                self.grow();
                count = 0;
            }
            if is_char(c) {
                copy_buf(&mut buf, l, c);
            } else {
                self.err(format!("Invalid char in processing instruction 0x{c:X}"));
            }
            self.nextl(l);
            (c, l) = self.cur_char();
            if c == 0 {
                self.grow();
                (c, l) = self.cur_char();
            }
        }
        if c != u32::from(b'>') {
            self.err(format!("ParsePI: PI {target} never end ..."));
        } else {
            self.skip(1);
            if !self.disable_sax {
                self.tree.misc(Kind::Pi { target, data: buf }, line);
            }
        }
    }

    /// htmlParseComment: `<!-- ... -->`, also ended by `--!>`.
    fn parse_comment(&mut self) {
        if self.at(0) != b'<' || self.at(1) != b'!' || self.at(2) != b'-' || self.at(3) != b'-' {
            return;
        }
        let line = self.line.min(65535);
        self.skip(4);
        let mut buf = Vec::new();
        let (q, ql) = self.cur_char();
        if q == 0 {
            self.err("Comment not terminated");
            return;
        }
        self.nextl(ql);
        let (r, rl) = self.cur_char();
        if r == 0 {
            self.err("Comment not terminated");
            return;
        }
        self.nextl(rl);
        let (mut q, mut ql, mut r, mut rl) = (q, ql, r, rl);
        let (mut c, mut l) = self.cur_char();
        while c != 0 && (c != u32::from(b'>') || r != u32::from(b'-') || q != u32::from(b'-')) {
            self.nextl(l);
            let (mut next, mut nl) = self.cur_char();
            if next == 0 {
                self.grow();
                (next, nl) = self.cur_char();
            }
            if q == u32::from(b'-')
                && r == u32::from(b'-')
                && c == u32::from(b'!')
                && next == u32::from(b'>')
            {
                self.err("Comment incorrectly closed by '--!>'");
                c = u32::from(b'>');
                break;
            }
            if is_char(q) {
                copy_buf(&mut buf, ql, q);
            } else {
                self.err(format!("Invalid char in comment 0x{q:X}"));
            }
            (q, ql, r, rl) = (r, rl, c, l);
            (c, l) = (next, nl);
        }
        let _ = rl;
        if c == u32::from(b'>') {
            self.next();
            if !self.disable_sax {
                self.tree.misc(Kind::Comment(buf), line);
            }
            return;
        }
        self.err("Comment not terminated");
    }

    /// htmlParseElementInternal.
    fn parse_element(&mut self) {
        if self.eof {
            return;
        }
        let failed = self.parse_start_tag();
        let name = self.name().map(str::to_string);
        let (Ok(_), Some(name)) = (failed, name) else {
            if self.at(0) == b'>' {
                self.next();
            }
            return;
        };
        let info = element_info(&name);
        if info.is_none() {
            self.err(format!("Tag {name} invalid"));
        }
        if self.at(0) == b'/' && self.at(1) == b'>' {
            self.skip(2);
            self.end_and_pop();
            return;
        }
        if self.at(0) == b'>' {
            self.next();
        } else {
            self.err(format!("Couldn't find end of Start Tag {name}"));
            if self.name() == Some(name.as_str()) {
                self.end_and_pop();
            }
            self.finish_element();
            return;
        }
        if info.is_some_and(|(_, _, empty)| empty) {
            self.end_and_pop();
        }
    }

    /// htmlParserFinishElementParsing.
    fn finish_element(&mut self) {
        if self.at(0) == 0 {
            self.auto_close_on_end();
        }
    }

    /// htmlParseContentInternal.
    fn parse_content(&mut self) {
        let mut current = self.name().map(str::to_string);
        let mut depth = self.names.len();
        loop {
            self.grow();
            if self.eof {
                break;
            }
            if self.at(0) == b'<' && self.at(1) == b'/' {
                if self.parse_end_tag() && (current.is_some() || self.names.is_empty()) {
                    current = self.name().map(str::to_string);
                    depth = self.names.len();
                }
                continue;
            } else if self.at(0) == b'<'
                && (self.at(1).is_ascii_alphabetic() || self.at(1) == b'_' || self.at(1) == b':')
            {
                let name = self.peek_html_name().unwrap_or_default();
                if let Some(cur) = self.name()
                    && check_auto_close(&name, cur)
                {
                    self.auto_close(&name);
                    continue;
                }
            }
            if !self.names.is_empty()
                && depth >= self.names.len()
                && current.as_deref() != self.name()
            {
                self.finish_element();
                current = self.name().map(str::to_string);
                depth = self.names.len();
                continue;
            }
            if self.at(0) != 0 && matches!(current.as_deref(), Some("script" | "style")) {
                self.parse_script();
            } else {
                if self.at_doctype() {
                    self.err("Misplaced DOCTYPE declaration");
                    self.parse_doctype();
                }
                if self.at(0) == b'<'
                    && self.at(1) == b'!'
                    && self.at(2) == b'-'
                    && self.at(3) == b'-'
                {
                    self.parse_comment();
                } else if self.at(0) == b'<' && self.at(1) == b'?' {
                    self.parse_pi();
                } else if self.at(0) == b'<' && self.at(1).is_ascii_alphabetic() {
                    self.parse_element();
                    current = self.name().map(str::to_string);
                    depth = self.names.len();
                } else if self.at(0) == b'<' {
                    if !self.disable_sax {
                        self.characters(b"<");
                    }
                    self.next();
                } else if self.at(0) == b'&' {
                    self.parse_reference();
                } else if self.at(0) == 0 {
                    self.auto_close_on_end();
                    break;
                } else {
                    self.parse_char_data();
                }
            }
            self.grow();
        }
    }

    /// htmlParseDocument.
    fn parse_document(&mut self) {
        self.grow();
        // xmlDetectCharEncoding on the first four bytes.
        if self.buf.len() >= 4 {
            match self.buf[..4] {
                [0x00, 0x3C, 0x00, 0x3F] => self.switch_to(Charset::Utf16 {
                    big_endian: true,
                    bom: false,
                }),
                [0x3C, 0x00, 0x3F, 0x00] => self.switch_to(Charset::Utf16 {
                    big_endian: false,
                    bom: false,
                }),
                [0x3C, 0x3F, 0x78, 0x6D] => self.utf8 = true,
                [0xEF, 0xBB, 0xBF, _] => {
                    self.utf8 = true;
                    self.cur = 3;
                }
                [0xFE, 0xFF, ..] => {
                    self.cur = 2;
                    self.switch_to(Charset::Utf16 {
                        big_endian: true,
                        bom: false,
                    });
                }
                [0xFF, 0xFE, ..] => {
                    self.cur = 2;
                    self.switch_to(Charset::Utf16 {
                        big_endian: false,
                        bom: false,
                    });
                }
                _ => {}
            }
        }
        self.skip_blanks();
        if self.at(0) == 0 {
            self.err("Document is empty");
        }
        let misc = |p: &Self| {
            p.at(0) == b'<'
                && ((p.at(1) == b'!' && p.at(2) == b'-' && p.at(3) == b'-') || p.at(1) == b'?')
        };
        while misc(self) {
            self.parse_comment();
            self.parse_pi();
            self.skip_blanks();
        }
        if self.at_doctype() {
            self.parse_doctype();
        }
        self.skip_blanks();
        while misc(self) {
            self.parse_comment();
            self.parse_pi();
            self.skip_blanks();
        }
        self.parse_content();
        if self.at(0) == 0 {
            self.auto_close_on_end();
        }
    }
}

#[cfg(test)]
mod tests;
