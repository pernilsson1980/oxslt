//! EXSLT extension functions, as libexslt 0.8.20 (with libxslt 1.1.35)
//! implements them: common, sets, strings and math, most of dates and
//! times (`super::dates`), and the `node-set` functions libxslt registers in
//! other namespaces.
//!
//! `function-available()` answers for every function xsltproc registers
//! (`AVAILABLE`), including some oxslt doesn't implement yet; those fail
//! with a clear error when called.

use crate::tree::{Builder, Document, QName, Sym};
use crate::xpath::{self, Env, NodeRef, Value, XPathError};

pub const COMMON: &str = "http://exslt.org/common";
pub const MATH: &str = "http://exslt.org/math";
pub const SETS: &str = "http://exslt.org/sets";
pub const STRINGS: &str = "http://exslt.org/strings";
pub const DATES: &str = "http://exslt.org/dates-and-times";
pub const DYNAMIC: &str = "http://exslt.org/dynamic";
pub const CRYPTO: &str = "http://exslt.org/crypto";
pub const SAXON: &str = "http://icl.com/saxon";
pub const LIBXSLT: &str = "http://xmlsoft.org/XSLT/namespace";
pub const XT: &str = "http://www.jclark.com/xt";

/// Every extension function xsltproc reports as available, by namespace.
pub const AVAILABLE: &[(&str, &[&str])] = &[
    (COMMON, &["node-set", "object-type"]),
    (
        MATH,
        &[
            "min", "max", "highest", "lowest", "constant", "random", "abs", "sqrt", "power", "log",
            "exp", "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
        ],
    ),
    (
        SETS,
        &[
            "difference",
            "intersection",
            "distinct",
            "has-same-node",
            "leading",
            "trailing",
        ],
    ),
    (
        STRINGS,
        &[
            "tokenize",
            "split",
            "encode-uri",
            "decode-uri",
            "padding",
            "align",
            "concat",
            "replace",
        ],
    ),
    (
        DATES,
        &[
            "add",
            "add-duration",
            "date",
            "date-time",
            "day-abbreviation",
            "day-in-month",
            "day-in-week",
            "day-in-year",
            "day-name",
            "day-of-week-in-month",
            "difference",
            "duration",
            "hour-in-day",
            "leap-year",
            "minute-in-hour",
            "month-abbreviation",
            "month-in-year",
            "month-name",
            "second-in-minute",
            "seconds",
            "sum",
            "time",
            "week-in-month",
            "week-in-year",
            "year",
        ],
    ),
    (DYNAMIC, &["evaluate", "map"]),
    (
        SAXON,
        &[
            "expression",
            "eval",
            "evaluate",
            "line-number",
            "systemId",
            "node-set",
        ],
    ),
    (
        CRYPTO,
        &["md4", "md5", "sha1", "rc4_encrypt", "rc4_decrypt"],
    ),
    (LIBXSLT, &["node-set"]),
    (XT, &["node-set"]),
];

/// Whether xsltproc reports `{ns}local` as an available function.
pub fn is_available(ns: &str, local: &str) -> bool {
    AVAILABLE
        .iter()
        .any(|(n, names)| *n == ns && names.contains(&local))
}

/// What an extension function gives: a value, or new nodes. New nodes
/// are the children of a small document (libxslt's result value tree)
/// that the caller adds to the transform's documents.
pub enum Outcome {
    Value(Value),
    Fragment(Box<Document>),
}

/// Calls `{ns}local`. None if it isn't an extension function.
pub fn call(
    ns: &str,
    local: &str,
    args: &[Value],
    env: &Env,
    token: Sym,
) -> Option<Result<Outcome, XPathError>> {
    if !is_available(ns, local) {
        return None;
    }
    let result = match (ns, local) {
        (COMMON | SAXON | LIBXSLT | XT, "node-set") => node_set(args, env),
        (COMMON, "object-type") => object_type(args, env),
        (SETS, _) => sets(local, args, env),
        (STRINGS, _) => strings(local, args, env, token),
        (MATH, _) => math(local, args, env),
        (DATES, _) if super::dates::IMPLEMENTED.contains(&local) => {
            super::dates::call(local, args, env).map(Outcome::Value)
        }
        _ => Err(XPathError::runtime(format!(
            "{{{ns}}}{local}() isn't implemented in oxslt yet"
        ))),
    };
    Some(result)
}

fn arity(args: &[Value], min: usize, max: usize) -> Result<(), XPathError> {
    if args.len() < min || args.len() > max {
        Err(XPathError::runtime("Invalid number of arguments"))
    } else {
        Ok(())
    }
}

fn node_set_arg(v: &Value) -> Result<&[NodeRef], XPathError> {
    match v {
        Value::NodeSet(ns) => Ok(ns),
        _ => Err(XPathError::type_error("Invalid type")),
    }
}

/// A document holding one text node, even an empty one.
fn text_fragment(s: &str) -> Document {
    let mut b = Builder::new();
    let _ = b.text_node(s, 0);
    b.finish()
}

fn value(v: Value) -> Result<Outcome, XPathError> {
    Ok(Outcome::Value(v))
}

// ---------- common ----------

/// exsl:node-set: a node-set as it is; anything else becomes a text node.
fn node_set(args: &[Value], env: &Env) -> Result<Outcome, XPathError> {
    arity(args, 1, 1)?;
    match &args[0] {
        Value::NodeSet(ns) => value(Value::NodeSet(ns.clone())),
        v => Ok(Outcome::Fragment(Box::new(text_fragment(
            &xpath::to_string(v, env),
        )))),
    }
}

/// exsl:object-type. A result tree fragment is a node-set of one
/// fragment's root in oxslt, reported as "RTF". libexslt says "node-set"
/// once exsl:node-set() has converted it; oxslt can't tell that apart.
fn object_type(args: &[Value], env: &Env) -> Result<Outcome, XPathError> {
    arity(args, 1, 1)?;
    let t = match &args[0] {
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Boolean(_) => "boolean",
        Value::NodeSet(ns) => match ns.as_slice() {
            [n] if n.ns == 0 && env.doc(*n).fragment && n.node == env.doc(*n).root() => "RTF",
            _ => "node-set",
        },
    };
    value(Value::String(t.into()))
}

// ---------- sets ----------

fn sets(local: &str, args: &[Value], env: &Env) -> Result<Outcome, XPathError> {
    match local {
        "distinct" => {
            arity(args, 1, 1)?;
            // The first node of each string value, in document order.
            let mut seen = std::collections::HashSet::new();
            let out = node_set_arg(&args[0])?
                .iter()
                .filter(|&&n| seen.insert(env.string_value(n)))
                .copied()
                .collect();
            value(Value::NodeSet(out))
        }
        _ => {
            arity(args, 2, 2)?;
            let a = node_set_arg(&args[0])?;
            let b = node_set_arg(&args[1])?;
            let in_b = |n: &NodeRef| b.binary_search(n).is_ok();
            match local {
                "difference" => value(Value::NodeSet(
                    a.iter().filter(|n| !in_b(n)).copied().collect(),
                )),
                "intersection" => value(Value::NodeSet(
                    a.iter().filter(|n| in_b(n)).copied().collect(),
                )),
                "has-same-node" => value(Value::Boolean(a.iter().any(in_b))),
                // xmlXPathNodeLeadingSorted and ...TrailingSorted, with b's
                // first node: everything when b is empty, nothing when a
                // doesn't contain it.
                "leading" | "trailing" => {
                    let Some(first) = b.first() else {
                        return value(Value::NodeSet(a.to_vec()));
                    };
                    let Ok(at) = a.binary_search(first) else {
                        return value(Value::NodeSet(vec![]));
                    };
                    let part = if local == "leading" {
                        &a[..at]
                    } else {
                        &a[at + 1..]
                    };
                    value(Value::NodeSet(part.to_vec()))
                }
                _ => unreachable!("listed in AVAILABLE"),
            }
        }
    }
}

// ---------- strings ----------

fn strings(local: &str, args: &[Value], env: &Env, token: Sym) -> Result<Outcome, XPathError> {
    let s = |i: usize| xpath::to_string(&args[i], env);
    match local {
        "tokenize" => {
            arity(args, 1, 2)?;
            let string = s(0);
            let delimiters = if args.len() == 2 {
                s(1)
            } else {
                " \t\r\n".to_string()
            };
            let mut tokens = Vec::new();
            if delimiters.is_empty() {
                tokens.extend(string.chars().map(String::from));
            } else {
                tokens.extend(
                    string
                        .split(|c| delimiters.contains(c))
                        .filter(|t| !t.is_empty())
                        .map(String::from),
                );
            }
            Ok(Outcome::Fragment(Box::new(token_fragment(&tokens, token))))
        }
        "split" => {
            arity(args, 1, 2)?;
            let string = s(0);
            let pattern = if args.len() == 2 { s(1) } else { " ".into() };
            Ok(Outcome::Fragment(Box::new(token_fragment(
                &split(&string, &pattern),
                token,
            ))))
        }
        "concat" => {
            arity(args, 1, 1)?;
            let mut out = String::new();
            for &n in node_set_arg(&args[0])? {
                out.push_str(&env.string_value(n));
            }
            value(Value::String(out))
        }
        "padding" => {
            arity(args, 1, 2)?;
            let mut chars: Vec<char> = if args.len() == 2 {
                s(1).chars().collect()
            } else {
                vec![]
            };
            if chars.is_empty() {
                chars = vec![' '];
            }
            let mut n = xpath::to_number(&args[0], env);
            if n.is_nan() || n <= 0.0 {
                return value(Value::String(String::new()));
            }
            // libexslt has no limit; an absurd length is an error here.
            if n > 10_000_000.0 {
                return Err(XPathError::runtime("str:padding() : length too large"));
            }
            let mut out = String::new();
            let len = chars.len() as f64;
            while n >= len {
                out.extend(&chars);
                n -= len;
            }
            out.extend(chars.iter().take(n as usize));
            value(Value::String(out))
        }
        "align" => {
            arity(args, 2, 3)?;
            let string: Vec<char> = s(0).chars().collect();
            let padding: Vec<char> = s(1).chars().collect();
            let alignment = if args.len() == 3 { s(2) } else { String::new() };
            let (sl, pl) = (string.len(), padding.len());
            let out: String = if sl == pl {
                string.iter().collect()
            } else if sl > pl {
                string[..pl].iter().collect()
            } else if alignment == "right" {
                padding[..pl - sl].iter().chain(&string).collect()
            } else if alignment == "center" {
                let left = (pl - sl) / 2;
                padding[..left]
                    .iter()
                    .chain(&string)
                    .chain(&padding[left + sl..])
                    .collect()
            } else {
                string.iter().chain(&padding[sl..]).collect()
            };
            value(Value::String(out))
        }
        "replace" => {
            arity(args, 3, 3)?;
            Ok(Outcome::Fragment(Box::new(text_fragment(&replace(
                args, env,
            )))))
        }
        "encode-uri" => {
            arity(args, 2, 3)?;
            let escape_reserved = xpath::boolean(&args[1]);
            value(Value::String(encode_uri(&s(0), escape_reserved)))
        }
        "decode-uri" => {
            arity(args, 1, 2)?;
            value(Value::String(decode_uri(&s(0))))
        }
        _ => unreachable!("listed in AVAILABLE"),
    }
}

/// A document of `<token>` elements, one per string.
fn token_fragment(tokens: &[String], token: Sym) -> Document {
    let mut b = Builder::new();
    let name = QName {
        ns: None,
        local: token,
        prefix: None,
    };
    for t in tokens {
        let _ = b.start_element(name, 0);
        let _ = b.text(t, 0);
        b.end_element();
    }
    b.finish()
}

/// str:split: tokens between occurrences of `pattern`, compared ignoring
/// ASCII case as libexslt does (xmlStrncasecmp); empty tokens are dropped.
/// An empty pattern splits into characters.
fn split(string: &str, pattern: &str) -> Vec<String> {
    if pattern.is_empty() {
        return string.chars().map(String::from).collect();
    }
    let lower = string.to_ascii_lowercase();
    let pat = pattern.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < string.len() {
        if lower[i..].starts_with(&pat) {
            if i > start {
                out.push(string[start..i].to_string());
            }
            i += pat.len();
            start = i;
        } else {
            i += string[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    if start < string.len() {
        out.push(string[start..].to_string());
    }
    out
}

/// exsltStrReplaceFunction: `search` and `replace` may be node-sets, each
/// node a string; the longest match wins at each position. An empty search
/// string with a non-empty replacement is inserted between characters.
fn replace(args: &[Value], env: &Env) -> String {
    let string = xpath::to_string(&args[0], env);
    let strings = |v: &Value| -> Vec<String> {
        match v {
            Value::NodeSet(ns) => ns.iter().map(|&n| env.string_value(n)).collect(),
            v => vec![xpath::to_string(v, env)],
        }
    };
    let search = strings(&args[1]);
    if search.is_empty() {
        return string;
    }
    let replace_list = strings(&args[2]);
    let replacement = |i: usize| -> &str {
        match &args[2] {
            Value::NodeSet(_) => replace_list.get(i).map_or("", String::as_str),
            _ if i == 0 => &replace_list[0],
            _ => "",
        }
    };
    let empty = search
        .iter()
        .position(String::is_empty)
        .filter(|&i| !replacement(i).is_empty());
    let mut out = String::new();
    let mut start = 0;
    let mut src = 0;
    while src < string.len() {
        let rest = &string[src..];
        let mut best: Option<(usize, usize)> = None; // (index, length)
        for (i, s) in search.iter().enumerate() {
            if !s.is_empty()
                && rest.starts_with(s.as_str())
                && best.is_none_or(|(_, l)| s.len() > l)
            {
                best = Some((i, s.len()));
            }
        }
        match best {
            None => {
                if let Some(e) = empty
                    && start < src
                {
                    out.push_str(&string[start..src]);
                    out.push_str(replacement(e));
                    start = src;
                }
                src += rest.chars().next().map_or(1, char::len_utf8);
            }
            Some((i, len)) => {
                out.push_str(&string[start..src]);
                out.push_str(replacement(i));
                src += len;
                start = src;
            }
        }
    }
    out.push_str(&string[start..]);
    out
}

/// str:encode-uri, as xmlURIEscapeStr: percent-encoding of UTF-8 bytes
/// outside RFC 2396's unreserved characters (and, unless
/// `escape_reserved`, its reserved ones). '@' is never escaped.
fn encode_uri(s: &str, escape_reserved: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric()
            || b"-_.!~*'()@".contains(&b)
            || (!escape_reserved && b";/?:&=+$,[]".contains(&b));
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// str:decode-uri: %XX sequences to bytes; invalid UTF-8 gives "".
fn decode_uri(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hex = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(bytes[i + 1]) * 16 + hex(bytes[i + 2]));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

// ---------- math ----------

/// libexslt's constants, as strings truncated to `precision` characters.
const CONSTANTS: &[(&str, &str)] = &[
    ("PI", "3.1415926535897932384626433832795028841971693993751"),
    ("E", "2.71828182845904523536028747135266249775724709369996"),
    (
        "SQRRT2",
        "1.41421356237309504880168872420969807856967187537694",
    ),
    (
        "LN2",
        "0.69314718055994530941723212145817656807550013436025",
    ),
    ("LN10", "2.30258509299404568402"),
    ("LOG2E", "1.4426950408889634074"),
    ("SQRT1_2", "0.70710678118654752440"),
];

fn math(local: &str, args: &[Value], env: &Env) -> Result<Outcome, XPathError> {
    let num = |i: usize| xpath::to_number(&args[i], env);
    let n = |v: f64| value(Value::Number(v));
    match local {
        "min" | "max" => {
            arity(args, 1, 1)?;
            let nodes = node_set_arg(&args[0])?;
            let mut best = f64::NAN;
            for (i, &node) in nodes.iter().enumerate() {
                let v = xpath::number::from_string(&env.string_value(node));
                if v.is_nan() {
                    return n(f64::NAN);
                }
                if i == 0 || (local == "min" && v < best) || (local == "max" && v > best) {
                    best = v;
                }
            }
            n(best)
        }
        "highest" | "lowest" => {
            arity(args, 1, 1)?;
            let nodes = node_set_arg(&args[0])?;
            let mut out = Vec::new();
            let mut best = f64::NAN;
            for (i, &node) in nodes.iter().enumerate() {
                let v = xpath::number::from_string(&env.string_value(node));
                if v.is_nan() {
                    return value(Value::NodeSet(vec![]));
                }
                let better = if local == "highest" {
                    v > best
                } else {
                    v < best
                };
                if i == 0 || better {
                    best = v;
                    out.clear();
                    out.push(node);
                } else if v == best {
                    out.push(node);
                }
            }
            value(Value::NodeSet(out))
        }
        "constant" => {
            arity(args, 2, 2)?;
            let name = xpath::to_string(&args[0], env);
            let precision = num(1);
            if precision.is_nan() || precision < 1.0 {
                return n(f64::NAN);
            }
            match CONSTANTS.iter().find(|(c, _)| *c == name) {
                Some((_, digits)) => {
                    let len = if precision <= digits.len() as f64 {
                        precision as usize
                    } else {
                        digits.len()
                    };
                    n(xpath::number::from_string(&digits[..len]))
                }
                None => n(f64::NAN),
            }
        }
        "random" => {
            arity(args, 0, 0)?;
            n(random())
        }
        "atan2" => {
            arity(args, 2, 2)?;
            n(num(0).atan2(num(1)))
        }
        "power" => {
            arity(args, 2, 2)?;
            n(num(0).powf(num(1)))
        }
        _ => {
            arity(args, 1, 1)?;
            let x = num(0);
            n(match local {
                "abs" => x.abs(),
                "sqrt" => x.sqrt(),
                "log" => x.ln(),
                "exp" => x.exp(),
                "sin" => x.sin(),
                "cos" => x.cos(),
                "tan" => x.tan(),
                "asin" => x.asin(),
                "acos" => x.acos(),
                "atan" => x.atan(),
                _ => unreachable!("listed in AVAILABLE"),
            })
        }
    }
}

/// math:random: a number in [0, 1). Not reproducible, as in libexslt.
fn random() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64),
    );
    (h.finish() >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_replace_helpers() {
        assert_eq!(split("a, b,, c", ","), vec!["a", " b", " c"]);
        assert_eq!(split("aXbxc", "x"), vec!["a", "b", "c"]);
        assert_eq!(split("ab", ""), vec!["a", "b"]);
        assert_eq!(encode_uri("a b/é", true), "a%20b%2F%C3%A9");
        assert_eq!(encode_uri("a b/é", false), "a%20b/%C3%A9");
        assert_eq!(decode_uri("a%20b%C3%A9"), "a bé");
    }
}
