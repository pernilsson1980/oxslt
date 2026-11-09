//! Character encodings, as libxml2 interprets their names.
//!
//! `encoding_rs` follows the WHATWG Encoding Standard, which treats
//! "ISO-8859-1" and "US-ASCII" as windows-1252. libxml2 implements them
//! literally: ISO-8859-1 maps each byte to U+0000–U+00FF, and US-ASCII has
//! only 7-bit characters. Those two are handled here; everything else goes
//! to `encoding_rs`.

use std::borrow::Cow;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Charset {
    Utf8,
    Latin1,
    Ascii,
    /// UTF-16. libxml2 writes "UTF-16" as little-endian with a byte-order
    /// mark, and "UTF-16LE"/"UTF-16BE" without one. `encoding_rs` decodes
    /// UTF-16 but can't encode it.
    Utf16 {
        big_endian: bool,
        bom: bool,
    },
    Other(&'static encoding_rs::Encoding),
}

/// Resolves an encoding name, or None if it's unknown.
pub fn lookup(label: &str) -> Option<Charset> {
    let l = label.trim().to_ascii_lowercase();
    Some(match l.as_str() {
        "utf-8" | "utf8" => Charset::Utf8,
        "iso-8859-1" | "iso8859-1" | "iso_8859-1" | "latin1" | "latin-1" | "l1" | "iso-ir-100"
        | "cp819" | "ibm819" => Charset::Latin1,
        "us-ascii" | "ascii" | "iso646-us" | "us" | "ansi_x3.4-1968" => Charset::Ascii,
        "utf-16" | "utf16" => Charset::Utf16 {
            big_endian: false,
            bom: true,
        },
        "utf-16le" => Charset::Utf16 {
            big_endian: false,
            bom: false,
        },
        "utf-16be" => Charset::Utf16 {
            big_endian: true,
            bom: false,
        },
        _ => match encoding_rs::Encoding::for_label(l.as_bytes())? {
            e if e == encoding_rs::UTF_8 => Charset::Utf8,
            e => Charset::Other(e),
        },
    })
}

impl Charset {
    pub fn name(self) -> &'static str {
        match self {
            Charset::Utf8 => "UTF-8",
            Charset::Latin1 => "ISO-8859-1",
            Charset::Ascii => "US-ASCII",
            Charset::Utf16 {
                big_endian: true, ..
            } => "UTF-16BE",
            Charset::Utf16 { .. } => "UTF-16",
            Charset::Other(e) => e.name(),
        }
    }

    /// Decodes input, or None if it isn't valid in this encoding.
    pub fn decode<'a>(self, bytes: &'a [u8]) -> Option<Cow<'a, str>> {
        match self {
            Charset::Utf8 => std::str::from_utf8(bytes).ok().map(Cow::Borrowed),
            Charset::Latin1 => Some(Cow::Owned(bytes.iter().map(|&b| char::from(b)).collect())),
            Charset::Ascii => bytes
                .is_ascii()
                .then(|| Cow::Borrowed(std::str::from_utf8(bytes).unwrap_or_default())),
            Charset::Utf16 { big_endian, .. } => {
                let e = if big_endian {
                    encoding_rs::UTF_16BE
                } else {
                    encoding_rs::UTF_16LE
                };
                e.decode_without_bom_handling_and_without_replacement(bytes)
            }
            Charset::Other(e) => e.decode_without_bom_handling_and_without_replacement(bytes),
        }
    }

    /// Encodes output. A character the encoding can't represent becomes
    /// `&#N;` when `references` is set (markup), and `?` otherwise (text).
    pub fn encode(self, s: &str, references: bool) -> Vec<u8> {
        let unmappable = |c: char, out: &mut Vec<u8>| {
            if references {
                out.extend_from_slice(format!("&#{};", c as u32).as_bytes());
            } else {
                out.push(b'?');
            }
        };
        let mut out = Vec::with_capacity(s.len());
        match self {
            Charset::Utf8 => return s.as_bytes().to_vec(),
            Charset::Latin1 | Charset::Ascii => {
                let max = if self == Charset::Latin1 { 0xFF } else { 0x7F };
                for c in s.chars() {
                    if (c as u32) <= max {
                        out.push(c as u32 as u8);
                    } else {
                        unmappable(c, &mut out);
                    }
                }
            }
            Charset::Utf16 { big_endian, bom } => {
                if bom {
                    out.extend_from_slice(if big_endian {
                        &[0xFE, 0xFF]
                    } else {
                        &[0xFF, 0xFE]
                    });
                }
                for unit in s.encode_utf16() {
                    out.extend_from_slice(&if big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    });
                }
            }
            Charset::Other(enc) => {
                let mut encoder = enc.new_encoder();
                let mut buf = [0u8; 4];
                for c in s.chars() {
                    let mut dst = [0u8; 16];
                    let (result, _, written) = encoder.encode_from_utf8_without_replacement(
                        c.encode_utf8(&mut buf),
                        &mut dst,
                        false,
                    );
                    match result {
                        encoding_rs::EncoderResult::InputEmpty => {
                            out.extend_from_slice(&dst[..written])
                        }
                        _ => unmappable(c, &mut out),
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin1_and_ascii_are_literal() {
        let latin1 = lookup("ISO-8859-1").unwrap();
        assert_eq!(latin1, Charset::Latin1);
        // In windows-1252 0x89 would be '‰'; in ISO-8859-1 it's U+0089.
        assert_eq!(latin1.decode(b"\x89\xe9").unwrap(), "\u{89}é");
        assert_eq!(latin1.encode("é‰", true), b"\xe9&#8240;");
        let ascii = lookup("US-ASCII").unwrap();
        assert_eq!(ascii.encode("aé", true), b"a&#233;");
        assert!(ascii.decode(b"\xe9").is_none());
        assert_eq!(
            lookup("windows-1252"),
            Some(Charset::Other(encoding_rs::WINDOWS_1252))
        );
        assert_eq!(lookup("no-such"), None);
        let utf16 = lookup("UTF-16").unwrap();
        assert_eq!(utf16.encode("<é", true), [0xFF, 0xFE, b'<', 0, 0xE9, 0]);
        assert_eq!(lookup("UTF-16BE").unwrap().encode("<", true), [0, b'<']);
    }
}
