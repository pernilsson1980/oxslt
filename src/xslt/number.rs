//! `xsl:number`'s formatting: a port of libxslt 1.1.35's `numbers.c`
//! (xsltNumberFormatTokenize, xsltNumberFormatInsertNumbers and the
//! decimal, alphabetic and Roman formatters), floating-point arithmetic
//! included, so the output matches byte for byte.

/// The digit zeros libxslt recognises (xsltIsDigitZero).
fn is_digit_zero(c: u32) -> bool {
    matches!(
        c,
        0x0030
            | 0x0660
            | 0x06F0
            | 0x0966
            | 0x09E6
            | 0x0A66
            | 0x0AE6
            | 0x0B66
            | 0x0C66
            | 0x0CE6
            | 0x0D66
            | 0x0E50
            | 0x0E60
            | 0x0F20
            | 0x1040
            | 0x17E0
            | 0x1810
            | 0xFF10
    )
}

fn is_digit_one(c: u32) -> bool {
    c > 0 && is_digit_zero(c - 1)
}

/// libxslt's xsltIsLetterDigit: XML 1.0's BaseChar, Ideographic and Digit
/// classes. Unicode's alphanumerics are close enough: they differ only on
/// characters XML 1.0's tables (from Unicode 2.0) leave out.
fn is_letter_digit(c: char) -> bool {
    c.is_alphanumeric()
}

#[derive(Debug, Clone)]
struct Token {
    /// The text before this token; None for the first one.
    separator: Option<String>,
    /// '0' (or another digit zero), 'A', 'a', 'I' or 'i'.
    token: u32,
    width: usize,
}

/// A tokenized `format` attribute.
#[derive(Debug, Clone)]
pub struct Format {
    start: Option<String>,
    tokens: Vec<Token>,
    end: Option<String>,
}

const MAX_TOKENS: usize = 1024;

impl Format {
    /// xsltNumberFormatTokenize.
    pub fn parse(format: &str) -> Format {
        let chars: Vec<char> = format.chars().collect();
        let mut ix = 0;
        while ix < chars.len() && !is_letter_digit(chars[ix]) {
            ix += 1;
        }
        let start = (ix > 0).then(|| chars[..ix].iter().collect());
        let mut tokens = Vec::new();
        let mut end: Option<String> = None;
        let at = |i: usize| chars.get(i).map_or(0, |&c| c as u32);
        while tokens.len() < MAX_TOKENS && ix < chars.len() {
            let separator = if tokens.is_empty() { None } else { end.take() };
            let mut val = at(ix);
            let (token, width);
            if is_digit_one(val) || is_digit_zero(val) {
                let mut w = 1;
                while is_digit_zero(val) {
                    w += 1;
                    ix += 1;
                    val = at(ix);
                }
                if is_digit_one(val) {
                    token = val - 1;
                    width = w;
                    ix += 1;
                } else {
                    token = '0' as u32;
                    width = 1;
                }
            } else if matches!(char::from_u32(val), Some('A' | 'a' | 'I' | 'i')) {
                token = val;
                width = 1;
                ix += 1;
            } else {
                // XSLT 7.7: an unsupported token means "1".
                token = '0' as u32;
                width = 1;
            }
            while ix < chars.len() && is_letter_digit(chars[ix]) {
                ix += 1;
            }
            let j = ix;
            while ix < chars.len() && !is_letter_digit(chars[ix]) {
                ix += 1;
            }
            if ix > j {
                end = Some(chars[j..ix].iter().collect());
            }
            tokens.push(Token {
                separator,
                token,
                width,
            });
        }
        Format { start, tokens, end }
    }
}

/// Grouping, from `grouping-separator` and `grouping-size`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Grouping {
    /// None when either attribute is missing.
    pub separator: Option<char>,
    pub size: i64,
}

/// xsltNumberFormatInsertNumbers: `numbers` outermost first. Returns the
/// text and any warning.
pub fn insert_numbers(
    numbers: &[f64],
    format: &Format,
    grouping: Grouping,
) -> (String, Option<String>) {
    let mut out = String::new();
    let mut warning = None;
    if let Some(s) = &format.start {
        out.push_str(s);
    }
    let default = Token {
        separator: Some(".".into()),
        token: '0' as u32,
        width: 1,
    };
    for (i, &n) in numbers.iter().enumerate() {
        // Round to nearest, as XSLT 2.0 does.
        let mut number = (n + 0.5).floor();
        if number < 0.0 {
            warning = Some("xsl-number : negative value".to_string());
            number = 0.0;
        }
        let token = format
            .tokens
            .get(i)
            .or(format.tokens.last())
            .unwrap_or(&default);
        if i > 0 {
            out.push_str(token.separator.as_deref().unwrap_or("."));
        }
        if number.is_infinite() {
            out.push_str(if number < 0.0 {
                "-Infinity"
            } else {
                "Infinity"
            });
        } else if number.is_nan() {
            out.push_str("NaN");
        } else {
            match char::from_u32(token.token) {
                Some('A') => alpha(&mut out, number, true, grouping),
                Some('a') => alpha(&mut out, number, false, grouping),
                Some('I') => roman(&mut out, number, true, grouping),
                Some('i') => roman(&mut out, number, false, grouping),
                _ if is_digit_zero(token.token) => {
                    if let Some(w) = decimal(&mut out, number, token.token, token.width, grouping) {
                        warning = Some(w);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(e) = &format.end {
        out.push_str(e);
    }
    (out, warning)
}

/// xsltNumberFormatDecimal: digits built from the back into a 500-byte
/// buffer, one `fmod` and division by ten at a time.
fn decimal(
    out: &mut String,
    mut number: f64,
    digit_zero: u32,
    width: usize,
    grouping: Grouping,
) -> Option<String> {
    const CAPACITY: usize = 499; // temp_string[500], less the NUL
    let mut chars: Vec<char> = Vec::new(); // last digit first
    let mut used = 0;
    let mut i = 0;
    let mut failed = false;
    let group = match grouping.separator {
        Some(c) if grouping.size > 0 => Some((c, grouping.size as usize)),
        _ => None,
    };
    while used < CAPACITY {
        if i >= width && number.abs() < 1.0 {
            break;
        }
        if let Some((sep, size)) = group
            && i > 0
            && i % size == 0
        {
            if used + sep.len_utf8() > CAPACITY {
                failed = true;
                break;
            }
            used += sep.len_utf8();
            chars.push(sep);
        }
        let val = digit_zero.wrapping_add((number % 10.0) as i32 as u32);
        let c = char::from_u32(val).unwrap_or('\u{FFFD}');
        if used + c.len_utf8() > CAPACITY {
            failed = true;
            break;
        }
        used += c.len_utf8();
        chars.push(c);
        number /= 10.0;
        i += 1;
    }
    out.extend(chars.iter().rev());
    failed.then(|| "xsltNumberFormatDecimal: Internal buffer size exceeded".to_string())
}

/// xsltNumberFormatAlpha: bijective base 26, at most 64 letters. Below 1
/// it falls back to decimal.
fn alpha(out: &mut String, mut number: f64, upper: bool, grouping: Grouping) {
    if number < 1.0 {
        decimal(out, number, '0' as u32, 1, grouping);
        return;
    }
    let list = if upper {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    } else {
        b"abcdefghijklmnopqrstuvwxyz"
    };
    let mut letters = Vec::new();
    for _ in 1..65 {
        number -= 1.0;
        letters.push(list[(number % 26.0) as i32 as usize % 26] as char);
        number /= 26.0;
        if number < 1.0 {
            break;
        }
    }
    out.extend(letters.iter().rev());
}

/// xsltNumberFormatRoman, for 1 to 5000; decimal otherwise.
fn roman(out: &mut String, mut number: f64, upper: bool, grouping: Grouping) {
    if !(1.0..=5000.0).contains(&number) {
        decimal(out, number, '0' as u32, 1, grouping);
        return;
    }
    let mut put = |s: &str| {
        if upper {
            out.push_str(&s.to_ascii_uppercase());
        } else {
            out.push_str(s);
        }
    };
    // (value, letters, repeats)
    for (value, letters, repeat) in [
        (1000.0, "m", true),
        (900.0, "cm", false),
        (500.0, "d", true),
        (400.0, "cd", false),
        (100.0, "c", true),
        (90.0, "xc", false),
        (50.0, "l", true),
        (40.0, "xl", false),
        (10.0, "x", true),
        (9.0, "ix", false),
        (5.0, "v", true),
        (4.0, "iv", false),
        (1.0, "i", true),
    ] {
        if repeat {
            while number >= value {
                put(letters);
                number -= value;
            }
        } else if number >= value {
            put(letters);
            number -= value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(numbers: &[f64], format: &str) -> String {
        insert_numbers(numbers, &Format::parse(format), Grouping::default()).0
    }

    #[test]
    fn formats() {
        assert_eq!(fmt(&[3.0], ""), "3");
        assert_eq!(fmt(&[1.0, 2.0, 3.0], ""), "1.2.3");
        assert_eq!(fmt(&[1.0, 2.0], "1.a"), "1.b");
        assert_eq!(fmt(&[1.0, 2.0, 3.0], "(1.a)"), "(1.b.c)");
        assert_eq!(fmt(&[7.0], "001"), "007");
        // More numbers than tokens: the last token, with its separator (none
        // for the first, so ".").
        assert_eq!(fmt(&[27.0, 52.0, 53.0], "a, "), "aa.az.ba, ");
        assert_eq!(fmt(&[1999.0, 4.0], "I-i"), "MCMXCIX-iv");
        assert_eq!(fmt(&[0.0], "a"), "0");
        assert_eq!(fmt(&[2.5], "1"), "3");
        assert_eq!(fmt(&[f64::NAN], "1"), "NaN");
        assert_eq!(fmt(&[5.0], "x"), "5");
        let g = Grouping {
            separator: Some(','),
            size: 3,
        };
        assert_eq!(
            insert_numbers(&[1234567.0], &Format::parse("1"), g).0,
            "1,234,567"
        );
    }
}
