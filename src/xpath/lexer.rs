//! XPath 1.0 tokenizer, with the disambiguation rules of section 3.7:
//! - after anything but `@ :: ( [ ,` or an operator, `*` is multiplication and
//!   a name is an operator name (`and or mod div`);
//! - a name followed by `(` is a node type or function name;
//! - a name followed by `::` is an axis name.

use super::XPathError;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok<'a> {
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    DotDot,
    At,
    Comma,
    ColonColon,
    Slash,
    SlashSlash,
    Pipe,
    Plus,
    Minus,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Mod,
    Div,
    Multiply,
    /// `*` as a name test.
    Star,
    /// `prefix:*`
    PrefixStar(&'a str),
    /// A QName used as a name test: (prefix, local).
    Name(Option<&'a str>, &'a str),
    /// A QName followed by `(` that isn't a node type.
    FunctionName(Option<&'a str>, &'a str),
    /// `comment`, `text`, `processing-instruction` or `node` followed by `(`.
    NodeType(&'a str),
    AxisName(&'a str),
    Literal(&'a str),
    Number(f64),
    /// `$prefix:name`
    Variable(Option<&'a str>, &'a str),
}

#[derive(Clone, Debug)]
pub struct Token<'a> {
    pub tok: Tok<'a>,
    /// Byte offset in the expression, for error messages.
    pub pos: usize,
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic()
        || c == '_'
        || (!c.is_ascii() && !c.is_whitespace() && !c.is_control() && !is_xpath_punct(c))
}

fn is_name_char(c: char) -> bool {
    is_name_start(c)
        || c.is_ascii_digit()
        || c == '-'
        || c == '.'
        || c == '\u{B7}'
        || (!c.is_ascii() && c.is_numeric())
}

fn is_xpath_punct(c: char) -> bool {
    matches!(c, '\u{A0}' | '\u{2028}' | '\u{2029}' | '\u{FEFF}')
}

/// Tokens borrow their names and literals from `expr`.
pub fn tokenize<'a>(expr: &'a str) -> Result<Vec<Token<'a>>, XPathError> {
    let chars: Vec<(usize, char)> = expr.char_indices().collect();
    let mut out: Vec<Token<'a>> = Vec::new();
    let mut i = 0;
    let at = |i: usize| chars.get(i).map(|&(_, c)| c);
    let pos_of = |i: usize| chars.get(i).map_or(expr.len(), |&(p, _)| p);
    let err = |i: usize, msg: &str| XPathError::syntax(format!("{msg} at offset {}", pos_of(i)));

    // Section 3.7: whether the previous token lets `*` and names be operators.
    let operator_context = |out: &[Token<'a>]| match out.last() {
        None => false,
        Some(t) => !matches!(
            t.tok,
            Tok::At
                | Tok::ColonColon
                | Tok::LParen
                | Tok::LBracket
                | Tok::Comma
                | Tok::Slash
                | Tok::SlashSlash
                | Tok::Pipe
                | Tok::Plus
                | Tok::Minus
                | Tok::Eq
                | Tok::Ne
                | Tok::Lt
                | Tok::Le
                | Tok::Gt
                | Tok::Ge
                | Tok::And
                | Tok::Or
                | Tok::Mod
                | Tok::Div
                | Tok::Multiply
        ),
    };
    // The next non-whitespace character after position j.
    let next_non_ws = |mut j: usize| {
        while at(j).is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r')) {
            j += 1;
        }
        (j, at(j))
    };

    while let Some(c) = at(i) {
        let start = pos_of(i);
        let push = |out: &mut Vec<Token<'a>>, tok: Tok<'a>| out.push(Token { tok, pos: start });
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
                continue;
            }
            '(' => push(&mut out, Tok::LParen),
            ')' => push(&mut out, Tok::RParen),
            '[' => push(&mut out, Tok::LBracket),
            ']' => push(&mut out, Tok::RBracket),
            '@' => push(&mut out, Tok::At),
            ',' => push(&mut out, Tok::Comma),
            '|' => push(&mut out, Tok::Pipe),
            '+' => push(&mut out, Tok::Plus),
            '-' => push(&mut out, Tok::Minus),
            '=' => push(&mut out, Tok::Eq),
            '!' if at(i + 1) == Some('=') => {
                push(&mut out, Tok::Ne);
                i += 1;
            }
            '<' | '>' => {
                let eq = at(i + 1) == Some('=');
                let tok = match (c, eq) {
                    ('<', false) => Tok::Lt,
                    ('<', true) => Tok::Le,
                    ('>', false) => Tok::Gt,
                    _ => Tok::Ge,
                };
                push(&mut out, tok);
                if eq {
                    i += 1;
                }
            }
            '/' => {
                if at(i + 1) == Some('/') {
                    push(&mut out, Tok::SlashSlash);
                    i += 1;
                } else {
                    push(&mut out, Tok::Slash);
                }
            }
            ':' if at(i + 1) == Some(':') => {
                push(&mut out, Tok::ColonColon);
                i += 1;
            }
            '*' => {
                let tok = if operator_context(&out) {
                    Tok::Multiply
                } else {
                    Tok::Star
                };
                push(&mut out, tok);
            }
            '"' | '\'' => {
                let mut j = i + 1;
                while at(j).is_some_and(|d| d != c) {
                    j += 1;
                }
                if at(j).is_none() {
                    return Err(err(i, "Unfinished literal"));
                }
                push(&mut out, Tok::Literal(&expr[pos_of(i + 1)..pos_of(j)]));
                i = j;
            }
            '.' if !at(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                if at(i + 1) == Some('.') {
                    push(&mut out, Tok::DotDot);
                    i += 1;
                } else {
                    push(&mut out, Tok::Dot);
                }
            }
            '0'..='9' | '.' => {
                let bytes = expr.as_bytes();
                let (value, end) = super::number::scan_number(bytes, start)
                    .ok_or_else(|| err(i, "Invalid number"))?;
                push(&mut out, Tok::Number(value));
                while pos_of(i) < end {
                    i += 1;
                }
                continue;
            }
            '$' => {
                let (prefix, local, j) = scan_qname(expr, &chars, i + 1)
                    .ok_or_else(|| err(i, "Invalid variable name"))?;
                push(&mut out, Tok::Variable(prefix, local));
                i = j;
                continue;
            }
            c if is_name_start(c) => {
                let (ncname, mut j) = scan_ncname(expr, &chars, i);
                if operator_context(&out) {
                    let tok = match ncname {
                        "and" => Tok::And,
                        "or" => Tok::Or,
                        "mod" => Tok::Mod,
                        "div" => Tok::Div,
                        _ => return Err(err(i, "Expected an operator")),
                    };
                    push(&mut out, tok);
                    i = j;
                    continue;
                }
                // prefix:* or prefix:local (no whitespace around the colon).
                let mut prefix = None;
                let mut local = ncname;
                if at(j) == Some(':') && at(j + 1) != Some(':') {
                    if at(j + 1) == Some('*') {
                        push(&mut out, Tok::PrefixStar(local));
                        i = j + 2;
                        continue;
                    }
                    if at(j + 1).is_some_and(is_name_start) {
                        let (l, k) = scan_ncname(expr, &chars, j + 1);
                        prefix = Some(local);
                        local = l;
                        j = k;
                    } else {
                        return Err(err(j, "Invalid QName"));
                    }
                }
                let (after, next) = next_non_ws(j);
                let tok = match next {
                    Some('(')
                        if prefix.is_none()
                            && matches!(
                                local,
                                "comment" | "text" | "processing-instruction" | "node"
                            ) =>
                    {
                        Tok::NodeType(local)
                    }
                    Some('(') => Tok::FunctionName(prefix, local),
                    Some(':') if prefix.is_none() && at(after + 1) == Some(':') => {
                        Tok::AxisName(local)
                    }
                    _ => Tok::Name(prefix, local),
                };
                push(&mut out, tok);
                i = j;
                continue;
            }
            _ => return Err(err(i, "Invalid character")),
        }
        i += 1;
    }
    Ok(out)
}

/// An NCName starting at char index `i`: (name, index after it).
fn scan_ncname<'a>(expr: &'a str, chars: &[(usize, char)], mut i: usize) -> (&'a str, usize) {
    let start = chars.get(i).map_or(expr.len(), |&(p, _)| p);
    while chars.get(i).is_some_and(|&(_, c)| is_name_char(c)) {
        i += 1;
    }
    let end = chars.get(i).map_or(expr.len(), |&(p, _)| p);
    (&expr[start..end], i)
}

fn scan_qname<'a>(
    expr: &'a str,
    chars: &[(usize, char)],
    i: usize,
) -> Option<(Option<&'a str>, &'a str, usize)> {
    let at = |i: usize| chars.get(i).map(|&(_, c)| c);
    if !at(i).is_some_and(is_name_start) {
        return None;
    }
    let (first, j) = scan_ncname(expr, chars, i);
    if at(j) == Some(':') && at(j + 1).is_some_and(is_name_start) {
        let (local, k) = scan_ncname(expr, chars, j + 1);
        return Some((Some(first), local, k));
    }
    Some((None, first, j))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok<'_>> {
        tokenize(s).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn star_and_operator_names_depend_on_context() {
        assert_eq!(toks("*"), vec![Tok::Star]);
        assert_eq!(
            toks("2*3"),
            vec![Tok::Number(2.0), Tok::Multiply, Tok::Number(3.0)]
        );
        assert_eq!(
            toks("a/*"),
            vec![Tok::Name(None, "a"), Tok::Slash, Tok::Star]
        );
        assert_eq!(
            toks("div div div"),
            vec![Tok::Name(None, "div"), Tok::Div, Tok::Name(None, "div")]
        );
        assert_eq!(toks("and"), vec![Tok::Name(None, "and")]);
    }

    #[test]
    fn names_functions_node_types_and_axes() {
        assert_eq!(
            toks("text ()"),
            vec![Tok::NodeType("text"), Tok::LParen, Tok::RParen]
        );
        assert_eq!(toks("f:g(1)")[0], Tok::FunctionName(Some("f"), "g"));
        assert_eq!(toks("child :: x")[0], Tok::AxisName("child"));
        assert_eq!(toks("p:*"), vec![Tok::PrefixStar("p")]);
        assert_eq!(toks("$p:v"), vec![Tok::Variable(Some("p"), "v")]);
        assert_eq!(toks("a-b"), vec![Tok::Name(None, "a-b")]);
        assert_eq!(
            toks("a - b"),
            vec![Tok::Name(None, "a"), Tok::Minus, Tok::Name(None, "b")]
        );
    }

    #[test]
    fn literals_numbers_and_dots() {
        assert_eq!(toks("'a\"b'"), vec![Tok::Literal("a\"b")]);
        assert_eq!(
            toks(".5 .. ."),
            vec![Tok::Number(0.5), Tok::DotDot, Tok::Dot]
        );
        assert_eq!(toks("1.5e1"), vec![Tok::Number(15.0)]);
        assert!(tokenize("'open").is_err());
        assert!(tokenize("a # b").is_err());
    }
}
