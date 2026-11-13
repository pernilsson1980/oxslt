//! Reading the input as libxml2 does: 4000-byte chunks at its GROW points,
//! CUR/NEXT/CUR_CHAR with their UTF-8 checks and encoding switches, and the
//! character classes the parser uses.

use super::*;

/// INPUT_CHUNK and MINLEN: libxml2 reads input 4000 bytes at a time when
/// fewer than 250 are left, which bounds `htmlFindEncoding`'s look-ahead.
const INPUT_CHUNK: usize = 250;

const MINLEN: usize = 4000;

pub(super) fn is_blank(c: u8) -> bool {
    matches!(c, 0x20 | 0x09 | 0x0A | 0x0D)
}

/// IS_CHAR.
pub(super) fn is_char(c: u32) -> bool {
    matches!(c, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

/// IS_PUBIDCHAR_CH.
pub(super) fn is_pubid_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b" \r\n-'()+,./:=?;!*#@$_%".contains(&c)
}

/// IS_LETTER, IS_DIGIT, IS_COMBINING and IS_EXTENDER, approximated by
/// Unicode's classes: they matter only for non-ASCII names in entity
/// references, PI targets and DOCTYPE names.
pub(super) fn is_letter(c: u32) -> bool {
    char::from_u32(c).is_some_and(char::is_alphabetic)
}

pub(super) fn is_name_char(c: u32) -> bool {
    char::from_u32(c).is_some_and(|ch| ch.is_alphanumeric())
        || matches!(c, 0x2E | 0x2D | 0x5F | 0x3A | 0xB7)
        || (0x300..=0x36F).contains(&c)
}

/// Appends a character as UTF-8, as the parser's `bits` loops and
/// xmlCopyChar do (any value below 0x110000, surrogates included).
pub(super) fn push_char(out: &mut Vec<u8>, c: u32) {
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
pub(super) fn copy_buf(out: &mut Vec<u8>, l: usize, c: u32) {
    if l == 1 {
        out.push(c as u8);
    } else {
        push_char(out, c);
    }
}

impl<'a> Parser<'a> {
    pub(super) fn encoder(&self) -> bool {
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
    pub(super) fn grow(&mut self) {
        if self.buf.len().saturating_sub(self.cur) < INPUT_CHUNK {
            self.input_grow();
        }
    }

    /// CUR, NXT(n): 0 past the end.
    pub(super) fn at(&self, n: usize) -> u8 {
        self.buf.get(self.cur + n).copied().unwrap_or(0)
    }

    pub(super) fn upp(&self, n: usize) -> u8 {
        self.at(n).to_ascii_uppercase()
    }

    fn end(&self) -> bool {
        self.cur >= self.buf.len()
    }

    /// SKIP(n): ASCII known not to hold newlines.
    pub(super) fn skip(&mut self, n: usize) {
        self.cur = (self.cur + n).min(self.buf.len());
    }

    /// NEXTL(l).
    pub(super) fn nextl(&mut self, l: usize) {
        if self.at(0) == b'\n' {
            self.line += 1;
        }
        self.skip(l);
    }

    /// SKIP_BLANKS (htmlSkipBlankChars).
    pub(super) fn skip_blanks(&mut self) -> usize {
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
    pub(super) fn next(&mut self) {
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
    pub(super) fn cur_char(&mut self) -> (u32, usize) {
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
}
