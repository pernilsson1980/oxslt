//! `format-number()`: a port of libxslt 1.1.35's `xsltFormatNumberConversion`
//! (numbers.c), which follows Java's DecimalFormat. It works on the UTF-8
//! bytes of the pattern as the C code does, so prefix lengths and the
//! comparison of positive and negative affixes behave identically.

/// `xsl:decimal-format`. Each symbol is compared by its first character,
/// except the grouping separator in the integer part (the whole string).
#[derive(Debug, Clone)]
pub struct DecimalFormat {
    pub decimal_point: String,
    pub grouping: String,
    pub infinity: String,
    pub minus_sign: String,
    pub nan: String,
    pub percent: String,
    pub per_mille: String,
    pub zero_digit: String,
    pub digit: String,
    pub pattern_separator: String,
}

impl Default for DecimalFormat {
    /// libxslt's xsltNewDecimalFormat.
    fn default() -> Self {
        DecimalFormat {
            decimal_point: ".".into(),
            grouping: ",".into(),
            infinity: "Infinity".into(),
            minus_sign: "-".into(),
            nan: "NaN".into(),
            percent: "%".into(),
            per_mille: "\u{2030}".into(),
            zero_digit: "0".into(),
            digit: "#".into(),
            pattern_separator: ";".into(),
        }
    }
}

/// The result, and the warning libxslt prints for an invalid pattern.
#[derive(Debug)]
pub struct Formatted {
    pub text: String,
    pub warning: Option<String>,
}

const QUOTE: u8 = b'\'';

fn char_len(b: &[u8], i: usize) -> usize {
    match b.get(i) {
        None => 0,
        Some(&c) if c < 0x80 => 1,
        Some(&c) if c < 0xE0 => 2,
        Some(&c) if c < 0xF0 => 3,
        Some(_) => 4,
    }
    .min(b.len() - i.min(b.len()))
}

/// `xsltUTF8Charcmp`: the character at `b[i]` equals the first character of `sym`.
fn char_eq(b: &[u8], i: usize, sym: &str) -> bool {
    let Some(first) = sym.chars().next() else {
        return false;
    };
    let mut buf = [0u8; 4];
    let s = first.encode_utf8(&mut buf).as_bytes();
    b.get(i..i + s.len()) == Some(s)
}

#[derive(Default)]
struct Info {
    integer_hash: i32,
    integer_digits: i32,
    frac_digits: i32,
    frac_hash: i32,
    group: i32,
    multiplier: f64,
    add_decimal: bool,
    multiplier_set: bool,
}

impl DecimalFormat {
    fn is_special(&self, b: &[u8], i: usize) -> bool {
        char_eq(b, i, &self.zero_digit)
            || char_eq(b, i, &self.digit)
            || char_eq(b, i, &self.decimal_point)
            || char_eq(b, i, &self.grouping)
            || char_eq(b, i, &self.pattern_separator)
    }

    /// `xsltFormatNumberPreSuffix`: scans a prefix or suffix and returns its
    /// length in bytes, not counting quote characters, or None on an error.
    fn affix(&self, b: &[u8], i: &mut usize, info: &mut Info) -> Option<usize> {
        let mut count = 0;
        loop {
            if *i >= b.len() {
                return Some(count);
            }
            if b[*i] == QUOTE {
                *i += 1;
                if *i >= b.len() {
                    return None;
                }
            } else if self.is_special(b, *i) {
                return Some(count);
            } else if char_eq(b, *i, &self.percent) {
                if info.multiplier_set {
                    return None;
                }
                info.multiplier = 100.0;
                info.multiplier_set = true;
            } else if char_eq(b, *i, &self.per_mille) {
                if info.multiplier_set {
                    return None;
                }
                info.multiplier = 1000.0;
                info.multiplier_set = true;
            }
            let len = char_len(b, *i);
            if len < 1 {
                return None;
            }
            count += len;
            *i += len;
        }
    }

    pub fn format(&self, pattern: &str, number: f64) -> Formatted {
        let mut warning = None;
        if pattern.is_empty() {
            warning = Some("xsltFormatNumberConversion : Invalid format (0-length)".to_string());
        }
        if number.is_infinite() {
            let mut text = if number < 0.0 {
                self.minus_sign.clone()
            } else {
                String::new()
            };
            text.push_str(&self.infinity);
            return Formatted { text, warning };
        }
        if number.is_nan() {
            return Formatted {
                text: self.nan.clone(),
                warning,
            };
        }
        let b = pattern.as_bytes();
        let mut info = Info {
            group: -1,
            multiplier: 1.0,
            ..Info::default()
        };
        match self.parse(b, number, &mut info) {
            Some(parsed) => Formatted {
                text: self.output(number, &info, &parsed, b),
                warning,
            },
            None => {
                // An invalid pattern: libxslt warns and uses "0.0###".
                let default = Info {
                    integer_digits: 1,
                    frac_digits: 1,
                    frac_hash: 4,
                    group: -1,
                    multiplier: 1.0,
                    add_decimal: true,
                    ..Info::default()
                };
                let parsed = Parsed {
                    default_sign: number < 0.0,
                    prefix: (0, 0),
                    suffix: (0, 0),
                };
                Formatted {
                    text: self.output(number, &default, &parsed, b),
                    warning: Some(format!(
                        "xsltFormatNumberConversion : error in format string '{pattern}', using default"
                    )),
                }
            }
        }
    }

    /// The pattern's structure: whether to write the minus sign, and the
    /// byte ranges (start, length without quotes) of the prefix and suffix.
    fn parse(&self, b: &[u8], number: f64, info: &mut Info) -> Option<Parsed> {
        let mut i = 0;
        let prefix_start = i;
        let prefix_len = self.affix(b, &mut i, info)?;

        // The integer part. A percent or per-mille may turn out to belong to
        // the suffix, so its multiplier is applied only if more digits follow.
        let grouping = self.grouping.as_bytes();
        let mut delayed = 0.0;
        let mut len = 0;
        while i < b.len()
            && !char_eq(b, i, &self.decimal_point)
            && !char_eq(b, i, &self.pattern_separator)
        {
            if delayed != 0.0 {
                info.multiplier = delayed;
                info.multiplier_set = true;
                delayed = 0.0;
            }
            if char_eq(b, i, &self.digit) {
                if info.integer_digits > 0 {
                    return None;
                }
                info.integer_hash += 1;
                if info.group >= 0 {
                    info.group += 1;
                }
            } else if char_eq(b, i, &self.zero_digit) {
                info.integer_digits += 1;
                if info.group >= 0 {
                    info.group += 1;
                }
            } else if !grouping.is_empty() && b[i..].starts_with(grouping) {
                info.group = 0;
                i += grouping.len();
                continue;
            } else if char_eq(b, i, &self.percent) {
                if info.multiplier_set {
                    return None;
                }
                delayed = 100.0;
            } else if char_eq(b, i, &self.per_mille) {
                if info.multiplier_set {
                    return None;
                }
                delayed = 1000.0;
            } else {
                break;
            }
            len = char_len(b, i);
            if len < 1 {
                return None;
            }
            i += len;
        }

        // The fraction.
        if i < b.len() && char_eq(b, i, &self.decimal_point) {
            info.add_decimal = true;
            len = char_len(b, i);
            if len < 1 {
                return None;
            }
            i += len;
        }
        while i < b.len() {
            if char_eq(b, i, &self.zero_digit) {
                if info.frac_hash != 0 {
                    return None;
                }
                info.frac_digits += 1;
            } else if char_eq(b, i, &self.digit) {
                info.frac_hash += 1;
            } else if char_eq(b, i, &self.percent) || char_eq(b, i, &self.per_mille) {
                if info.multiplier_set {
                    return None;
                }
                delayed = if char_eq(b, i, &self.percent) {
                    100.0
                } else {
                    1000.0
                };
                len = char_len(b, i);
                if len < 1 {
                    return None;
                }
                i += len;
                continue;
            } else if !char_eq(b, i, &self.grouping) {
                break;
            }
            len = char_len(b, i);
            if len < 1 {
                return None;
            }
            i += len;
            if delayed != 0.0 {
                info.multiplier = delayed;
                delayed = 0.0;
                info.multiplier_set = true;
            }
        }
        // A percent or per-mille at the end belongs to the suffix.
        if delayed != 0.0 {
            i -= len;
        }
        let suffix_start = i;
        let suffix_len = self.affix(b, &mut i, info)?;
        if i < b.len() && !char_eq(b, i, &self.pattern_separator) {
            return None;
        }

        let mut parsed = Parsed {
            default_sign: false,
            prefix: (prefix_start, prefix_len),
            suffix: (suffix_start, suffix_len),
        };
        if number < 0.0 {
            // The negative subpattern, after the first pattern separator.
            let sep = pattern_separator_offset(b, &self.pattern_separator);
            match sep {
                None => parsed.default_sign = true,
                Some(after) => {
                    let mut j = after;
                    info.multiplier_set = false;
                    let nprefix_start = j;
                    let nprefix_len = self.affix(b, &mut j, info)?;
                    let mut delayed_n = false;
                    let mut len = 0;
                    while j < b.len() {
                        if char_eq(b, j, &self.percent) || char_eq(b, j, &self.per_mille) {
                            if info.multiplier_set {
                                return None;
                            }
                            info.multiplier_set = true;
                            delayed_n = true;
                        } else if self.is_special(b, j) {
                            delayed_n = false;
                        } else {
                            break;
                        }
                        len = char_len(b, j);
                        if len < 1 {
                            return None;
                        }
                        j += len;
                    }
                    if delayed_n {
                        info.multiplier_set = false;
                        j -= len;
                    }
                    let (nsuffix_start, nsuffix_len) = if j < b.len() {
                        let start = j;
                        (start, self.affix(b, &mut j, info)?)
                    } else {
                        (j, 0)
                    };
                    if j < b.len() {
                        return None;
                    }
                    // Java's rule: negative affixes equal to the positive
                    // ones are discarded. libxslt compares raw bytes.
                    let same = nprefix_len == prefix_len
                        && nsuffix_len == suffix_len
                        && (nprefix_len == 0
                            || b[nprefix_start..].starts_with(
                                &b[prefix_start
                                    ..prefix_start + prefix_len.min(b.len() - prefix_start)],
                            ))
                        && (nsuffix_len == 0
                            || b[nsuffix_start..].starts_with(
                                &b[suffix_start
                                    ..suffix_start + suffix_len.min(b.len() - suffix_start)],
                            ));
                    if !same {
                        parsed.prefix = (nprefix_start, nprefix_len);
                        parsed.suffix = (nsuffix_start, nsuffix_len);
                    }
                }
            }
        }
        Some(parsed)
    }

    fn output(&self, number: f64, info: &Info, parsed: &Parsed, b: &[u8]) -> String {
        let mut out = String::new();
        if parsed.default_sign {
            out.push_str(first_char(&self.minus_sign));
        }
        write_affix(b, parsed.prefix, &mut out);

        let mut frac_digits = info.frac_digits;
        let mut frac_hash = info.frac_hash;
        let mut n = number.abs() * info.multiplier;
        let scale = 10f64.powf(f64::from(frac_digits + frac_hash));
        n = (scale * n + 0.5).floor() / scale;
        let zero = self.zero_digit.as_bytes().first().copied().unwrap_or(b'0');
        let group_char = self.grouping.chars().next();
        digits(
            &mut out,
            n.floor(),
            zero,
            info.integer_digits,
            info.group,
            group_char.unwrap_or(','),
        );

        // Java treats ".#" like ".0", ".##" like ".0#" and so on.
        if info.integer_digits + info.integer_hash + frac_digits == 0 && frac_hash > 0 {
            frac_digits += 1;
            frac_hash -= 1;
        }
        if n.floor() == 0.0 && info.integer_digits + frac_digits == 0 {
            out.push_str(first_char(&self.zero_digit));
        }
        if frac_digits + frac_hash == 0 {
            if info.add_decimal {
                out.push_str(first_char(&self.decimal_point));
            }
        } else {
            let mut frac = n - n.floor();
            if frac != 0.0 || frac_digits != 0 {
                out.push_str(first_char(&self.decimal_point));
                frac = (scale * frac + 0.5).floor();
                let mut j = frac_hash;
                while j > 0 {
                    if frac % 10.0 >= 1.0 {
                        break;
                    }
                    frac /= 10.0;
                    j -= 1;
                }
                digits(&mut out, frac.floor(), zero, frac_digits + j, 0, ',');
            }
        }
        write_affix(b, parsed.suffix, &mut out);
        out
    }
}

struct Parsed {
    default_sign: bool,
    prefix: (usize, usize),
    suffix: (usize, usize),
}

fn first_char(s: &str) -> &str {
    s.char_indices().nth(1).map_or(s, |(i, _)| &s[..i])
}

/// The byte offset just after the first pattern separator, if any.
fn pattern_separator_offset(b: &[u8], sep: &str) -> Option<usize> {
    let mut i = 0;
    while i < b.len() {
        if char_eq(b, i, sep) {
            return Some(i + char_len(b, i));
        }
        i += char_len(b, i).max(1);
    }
    None
}

/// Copies `len` bytes of an affix starting at `start`, skipping quote characters.
fn write_affix(b: &[u8], (start, len): (usize, usize), out: &mut String) {
    let mut p = start;
    let mut j = 0;
    let mut bytes = Vec::new();
    while j < len && p < b.len() {
        if b[p] == QUOTE {
            p += 1;
        }
        let l = char_len(b, p);
        if l == 0 {
            break;
        }
        bytes.extend_from_slice(&b[p..p + l]);
        p += l;
        j += l;
    }
    out.push_str(&String::from_utf8_lossy(&bytes));
}

/// `xsltNumberFormatDecimal`: writes `number`'s digits with at least `width`
/// of them, inserting `group` every `per_group` digits. Each digit is the
/// zero digit's first byte plus the digit's value, as libxslt computes it.
fn digits(out: &mut String, mut number: f64, zero: u8, width: i32, per_group: i32, group: char) {
    const LIMIT: usize = 499; // libxslt's 500-byte buffer
    let mut rev: Vec<char> = Vec::new();
    let mut bytes = 0;
    let mut i = 0;
    loop {
        if i >= width && number.abs() < 1.0 {
            break;
        }
        if i > 0 && per_group > 0 && i % per_group == 0 {
            rev.push(group);
            bytes += group.len_utf8();
        }
        let value = u32::from(zero) + (number % 10.0) as u32;
        let c = char::from_u32(value).unwrap_or('?');
        bytes += c.len_utf8();
        if bytes > LIMIT {
            break;
        }
        rev.push(c);
        number /= 10.0;
        i += 1;
    }
    out.extend(rev.into_iter().rev());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(pattern: &str, n: f64) -> String {
        DecimalFormat::default().format(pattern, n).text
    }

    /// Expected values verified against Debian's xsltproc
    /// (tests/cases/14-format-number has the full set).
    #[test]
    fn common_patterns() {
        assert_eq!(f("#,##0.00", 1234567.891), "1,234,567.89");
        assert_eq!(f("0%", 0.5), "50%");
        assert_eq!(f("000;(000)", -3.0), "(003)");
        assert_eq!(f("#", 0.0), "0");
        assert_eq!(f(".##", 0.256), ".26");
        assert_eq!(f("#.#", 12.0), "12");
        assert_eq!(f("0.0", -0.04), "-0.0");
        assert_eq!(f("'#'0", 5.0), "#05");
        assert_eq!(f("#", f64::NAN), "NaN");
        assert_eq!(f("#", f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn an_invalid_pattern_falls_back_with_a_warning() {
        let r = DecimalFormat::default().format("#0#", 1.5);
        assert_eq!(r.text, "1.5");
        assert!(r.warning.unwrap().contains("using default"));
    }
}
