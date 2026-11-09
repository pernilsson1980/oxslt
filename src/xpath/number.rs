//! Number and string conversions, matching libxml2 2.9's XPath code
//! (`xmlXPathFormatNumber` and `xmlXPathStringEvalNumber`) rather than the
//! XPath 1.0 spec exactly. xsltproc's output depends on these details: large
//! and tiny numbers print in exponent form (`1e+21`), others with at most 15
//! significant digits (`0.1 + 0.2` prints `0.3`).

/// Converts a number to a string, as libxml2 does.
pub fn to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if n == 0.0 {
        return "0".into(); // also -0
    }
    if n > f64::from(i32::MIN) && n < f64::from(i32::MAX) && n == f64::from(n as i32) {
        return (n as i32).to_string();
    }
    let abs = n.abs();
    if !(1e-5..=1e9).contains(&abs) {
        // C's "%.14e": a mantissa with 14 fraction digits, then e, a sign and
        // at least two exponent digits. Trailing mantissa zeros are dropped.
        let s = format!("{n:.14e}");
        let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        let sign = if exp < 0 { '-' } else { '+' };
        return format!(
            "{}e{sign}{:02}",
            strip_fraction_zeros(mantissa),
            exp.unsigned_abs()
        );
    }
    // 15 significant digits in total. The cast truncates toward zero, as C's does.
    let integer_place = abs.log10() as i32;
    let fraction = if integer_place > 0 {
        15 - integer_place - 1
    } else {
        15 - integer_place
    };
    strip_fraction_zeros(&format!("{n:.*}", fraction.max(0) as usize))
}

fn strip_fraction_zeros(s: &str) -> String {
    if !s.contains('.') {
        return s.to_string();
    }
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn is_blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Converts a string to a number, as libxml2 does: optional whitespace, an
/// optional `-`, digits with an optional fraction and exponent, optional
/// whitespace. Anything else is NaN.
pub fn from_string(s: &str) -> f64 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && is_blank(b[i]) {
        i += 1;
    }
    let negative = b.get(i) == Some(&b'-');
    if negative {
        i += 1;
    }
    match scan_number(b, i) {
        Some((value, end)) => {
            let mut j = end;
            while j < b.len() && is_blank(b[j]) {
                j += 1;
            }
            if j != b.len() {
                return f64::NAN;
            }
            if negative { -value } else { value }
        }
        None => f64::NAN,
    }
}

/// Scans an unsigned number starting at `i`: digits, an optional fraction and
/// an optional exponent. Returns the value and the end position, or `None` if
/// there are no digits. Also used by the expression tokenizer.
pub(crate) fn scan_number(b: &[u8], mut i: usize) -> Option<(f64, usize)> {
    const MAX_FRACTION_DIGITS: i32 = 20;
    let digit = |i: usize| {
        b.get(i)
            .filter(|c| c.is_ascii_digit())
            .map(|c| f64::from(c - b'0'))
    };

    if b.get(i) != Some(&b'.') && digit(i).is_none() {
        return None;
    }
    let mut value = 0.0;
    let mut any = false;
    while let Some(d) = digit(i) {
        value = value * 10.0 + d;
        any = true;
        i += 1;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if digit(i).is_none() && !any {
            return None;
        }
        let mut places = 0;
        while b.get(i) == Some(&b'0') {
            places += 1;
            i += 1;
        }
        let max = places + MAX_FRACTION_DIGITS;
        let mut fraction = 0.0;
        while let Some(d) = digit(i) {
            if places >= max {
                break;
            }
            fraction = fraction * 10.0 + d;
            places += 1;
            i += 1;
        }
        value += fraction / 10f64.powf(f64::from(places));
        while digit(i).is_some() {
            i += 1;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        let negative = b.get(j) == Some(&b'-');
        if negative || b.get(j) == Some(&b'+') {
            j += 1;
        }
        if digit(j).is_some() {
            let mut exponent: i32 = 0;
            while let Some(d) = digit(j) {
                if exponent < 1_000_000 {
                    exponent = exponent * 10 + d as i32;
                }
                j += 1;
            }
            if negative {
                exponent = -exponent;
            }
            value *= 10f64.powf(f64::from(exponent));
            i = j;
        }
    }
    Some((value, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_libxml2() {
        // Values seen in xsltproc output (tests/cases/05-xpath-functions).
        let cases = [
            (1.5, "1.5"),
            (3.0, "3"),
            (-2.0, "-2"),
            (-0.0, "0"),
            (0.1 + 0.2, "0.3"),
            (1e21, "1e+21"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (f64::NAN, "NaN"),
            (1.0 / 3.0, "0.333333333333333"),
            (123456.789, "123456.789"),
            (1e-6, "1e-06"),
            (3e9, "3e+09"),
        ];
        for (n, want) in cases {
            assert_eq!(to_string(n), want, "{n:?}");
        }
    }

    #[test]
    fn parses_like_libxml2() {
        assert_eq!(from_string(" 12 "), 12.0);
        assert_eq!(from_string("-3.5"), -3.5);
        assert_eq!(from_string(".5"), 0.5);
        assert_eq!(from_string("5."), 5.0);
        assert_eq!(from_string("1e3"), 1000.0);
        for nan in ["", " ", "abc", "+1", "1 2", ".", "-", "--1", "1x"] {
            assert!(from_string(nan).is_nan(), "{nan:?}");
        }
    }
}
