//! Encodings as libxml2's HTML parser handles them: input converters,
//! libxml2's own encoding names and its handler lookup with iconv's rules,
//! switching encoding mid-input, the http-equiv guess (htmlFindEncoding) and
//! <meta> charsets (htmlCheckEncoding, htmlCheckMeta).

use super::*;

/// An input converter (input->buf->encoder), fed the raw input a chunk at
/// a time as libxml2 reads it. Like libxml2's converters it stops at the
/// first invalid byte: the parser then sees the end of the input there.
pub(super) enum Converter {
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
    pub(super) fn convert(&mut self, raw: &[u8], last: bool, out: &mut Vec<u8>) -> bool {
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
pub(super) enum Handler {
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
pub(super) fn find_handler(name: &str) -> Option<Handler> {
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

impl<'a> Parser<'a> {
    /// Switches the input to `charset` (xmlSwitchEncoding and
    /// xmlSwitchToEncoding). With no converter yet, the buffer from here on
    /// is the raw input read so far, which is converted from here (for HTML
    /// libxml2 converts all of it at once). With a converter already, libxml2
    /// only swaps it: input already read stays as it was converted.
    pub(super) fn switch_to(&mut self, charset: Charset) {
        if charset == Charset::Utf8 {
            self.utf8 = true;
            return;
        }
        self.switch_to_handler(charset);
    }

    /// xmlSwitchToEncoding: always a converter, UTF-8 included.
    pub(super) fn switch_to_handler(&mut self, charset: Charset) {
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
    pub(super) fn find_encoding(&self) -> Option<String> {
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
    pub(super) fn check_meta(&mut self, attrs: &[(String, Option<Vec<u8>>)]) {
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
}
