//! XPath 1.0's core function library, as libxml2 implements it: the node-set,
//! string, boolean and number functions.

use super::*;

// ---------- functions ----------

fn arity(f: CoreFn, n: usize, min: usize, max: usize) -> R<()> {
    if n < min || n > max {
        return Err(XPathError::runtime(format!(
            "Invalid number of arguments to {f:?}"
        )));
    }
    Ok(())
}

pub(super) fn call(f: &Function, args: &[Expr], env: &Env, focus: &Focus) -> R<Value> {
    let mut values = Vec::with_capacity(args.len());
    for a in args {
        values.push(evaluate(a, env, focus)?);
    }
    match f {
        Function::Core(core) => core_function(*core, values, env, focus),
        Function::Other(name) => env
            .funcs
            .call(*name, values, env, focus)
            .unwrap_or_else(|| {
                let ns = name.0.map_or("", |n| env.names.str(n));
                Err(XPathError::runtime(format!(
                    "Unregistered function {{{ns}}}{}",
                    env.names.str(name.1)
                )))
            }),
    }
}

/// The first node of an optional node-set argument, defaulting to the
/// context node. An empty node-set gives `None`.
fn node_arg(f: CoreFn, args: &[Value], focus: &Focus) -> R<Option<NodeRef>> {
    match args.first() {
        None => Ok(Some(focus.node)),
        Some(v) => match v.nodes_or_tree() {
            Some(n) => Ok(n.first().copied()),
            None => Err(XPathError::type_error(format!("{f:?} needs a node-set"))),
        },
    }
}

fn string_arg(args: &[Value], env: &Env, focus: &Focus) -> String {
    match args.first() {
        Some(v) => to_string(v, env),
        None => env.string_value(focus.node),
    }
}

fn core_function(f: CoreFn, args: Vec<Value>, env: &Env, focus: &Focus) -> R<Value> {
    use CoreFn::*;
    let n = args.len();
    let s = |i: usize| to_str(&args[i], env);
    Ok(match f {
        Last => {
            arity(f, n, 0, 0)?;
            Value::Number(focus.size as f64)
        }
        Position => {
            arity(f, n, 0, 0)?;
            Value::Number(focus.position as f64)
        }
        Count => {
            arity(f, n, 1, 1)?;
            match args[0].nodes_or_tree() {
                Some(nodes) => Value::Number(nodes.len() as f64),
                None => return Err(XPathError::type_error("count() needs a node-set")),
            }
        }
        Id => {
            arity(f, n, 1, 1)?;
            let tokens: Vec<std::string::String> = match args[0].nodes_or_tree() {
                Some(nodes) => nodes
                    .iter()
                    .flat_map(|&n| {
                        env.string_value(n)
                            .split_ascii_whitespace()
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .collect(),
                None => to_string(&args[0], env)
                    .split_ascii_whitespace()
                    .map(str::to_owned)
                    .collect(),
            };
            Value::NodeSet(find_ids(&tokens, focus.node, env))
        }
        LocalName | NamespaceUri | Name => {
            arity(f, n, 0, 1)?;
            Value::String(match node_arg(f, &args, focus)? {
                None => std::string::String::new(),
                Some(node) => match f {
                    LocalName => env.local_name(node),
                    NamespaceUri => env.namespace_uri(node),
                    _ => env.qualified_name(node),
                },
            })
        }
        String => {
            arity(f, n, 0, 1)?;
            Value::String(string_arg(&args, env, focus))
        }
        Concat => {
            arity(f, n, 2, usize::MAX)?;
            let joined: std::string::String = (0..n).map(s).collect();
            // Doubling a string in a recursive template is cheap in calls
            // and expensive in bytes.
            env.funcs.tick(joined.len() as u64 / 64)?;
            Value::String(joined)
        }
        StartsWith => {
            arity(f, n, 2, 2)?;
            Value::Boolean(s(0).starts_with(&*s(1)))
        }
        Contains => {
            arity(f, n, 2, 2)?;
            Value::Boolean(s(0).contains(&*s(1)))
        }
        SubstringBefore => {
            arity(f, n, 2, 2)?;
            let (a, b) = (s(0), s(1));
            Value::String(
                a.find(&*b)
                    .map_or(std::string::String::new(), |i| a[..i].to_string()),
            )
        }
        SubstringAfter => {
            arity(f, n, 2, 2)?;
            let (a, b) = (s(0), s(1));
            Value::String(
                a.find(&*b)
                    .map_or(std::string::String::new(), |i| a[i + b.len()..].to_string()),
            )
        }
        Substring => {
            arity(f, n, 2, 3)?;
            let len = if n == 3 {
                Some(to_number(&args[2], env))
            } else {
                None
            };
            Value::String(substring(&s(0), to_number(&args[1], env), len))
        }
        StringLength => {
            arity(f, n, 0, 1)?;
            Value::Number(string_arg(&args, env, focus).chars().count() as f64)
        }
        NormalizeSpace => {
            arity(f, n, 0, 1)?;
            let text = string_arg(&args, env, focus);
            let words: Vec<&str> = text
                .split([' ', '\t', '\n', '\r'])
                .filter(|w| !w.is_empty())
                .collect();
            Value::String(words.join(" "))
        }
        Translate => {
            arity(f, n, 3, 3)?;
            let (from, to): (Vec<char>, Vec<char>) =
                (s(1).chars().collect(), s(2).chars().collect());
            let out = s(0)
                .chars()
                .filter_map(|c| match from.iter().position(|&x| x == c) {
                    Some(i) => to.get(i).copied(),
                    None => Some(c),
                })
                .collect();
            Value::String(out)
        }
        Boolean => {
            arity(f, n, 1, 1)?;
            Value::Boolean(boolean(&args[0]))
        }
        Not => {
            arity(f, n, 1, 1)?;
            Value::Boolean(!boolean(&args[0]))
        }
        True | False => {
            arity(f, n, 0, 0)?;
            Value::Boolean(f == True)
        }
        Lang => {
            arity(f, n, 1, 1)?;
            Value::Boolean(lang_matches(&s(0), focus.node, env))
        }
        Number => {
            arity(f, n, 0, 1)?;
            Value::Number(match args.first() {
                Some(v) => to_number(v, env),
                None => number::from_string(&env.string_value(focus.node)),
            })
        }
        Sum => {
            arity(f, n, 1, 1)?;
            match args[0].nodes_or_tree() {
                Some(nodes) => Value::Number(
                    nodes
                        .iter()
                        .map(|&n| number::from_string(&env.string_value(n)))
                        .sum(),
                ),
                None => return Err(XPathError::type_error("sum() needs a node-set")),
            }
        }
        Floor => {
            arity(f, n, 1, 1)?;
            Value::Number(to_number(&args[0], env).floor())
        }
        Ceiling => {
            arity(f, n, 1, 1)?;
            Value::Number(to_number(&args[0], env).ceil())
        }
        Round => {
            arity(f, n, 1, 1)?;
            Value::Number(round(to_number(&args[0], env)))
        }
    })
}

/// libxml2's `round()`: values in [-0.5, 0.5) keep their sign as zero.
pub fn round(x: f64) -> f64 {
    if (-0.5..0.5).contains(&x) {
        return x * 0.0;
    }
    let r = x.floor();
    if x - r >= 0.5 { r + 1.0 } else { r }
}

/// libxml2's `substring()` (xmlXPathSubstringFunction): positions are
/// rounded like `round()`, and NaN or infinite arguments are handled by the
/// same comparisons libxml2 makes.
// The negated comparisons are deliberate: `!(x < y)` is true for NaN, which
// is how libxml2 sends NaN down the same branch as a too-large value.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn substring(s: &str, start: f64, len: Option<f64>) -> String {
    const INT_MAX: f64 = i32::MAX as f64;
    let mut i: i64 = 1;
    let mut j: i64 = i32::MAX as i64;
    if !(start < INT_MAX) {
        i = i32::MAX as i64;
    } else if start >= 1.0 {
        i = start as i64;
        if start - start.floor() >= 0.5 {
            i += 1;
        }
    }
    if let Some(le) = len {
        let mut rin = start.floor();
        if start - rin >= 0.5 {
            rin += 1.0;
        }
        let mut rle = le.floor();
        if le - rle >= 0.5 {
            rle += 1.0;
        }
        let end = rin + rle;
        if !(end >= 1.0) {
            j = 1;
        } else if end < INT_MAX {
            j = end as i64;
        }
    }
    if i >= j {
        return String::new();
    }
    s.chars()
        .skip((i - 1) as usize)
        .take((j - i) as usize)
        .collect()
}

/// `lang()`: the nearest `xml:lang` on the context node or an ancestor
/// matches the argument, ignoring case, up to a `-` suffix.
fn lang_matches(want: &str, node: NodeRef, env: &Env) -> bool {
    let (xml, lang) = (crate::tree::NamePool::XML_NS, crate::tree::NamePool::LANG);
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n.ns == 0 {
            let doc = env.doc(n);
            for a in doc.attributes(n.node) {
                if doc.name(a).map(|q| q.expanded()) == Some((Some(xml), lang)) {
                    let value = doc.value(a).unwrap_or("");
                    let head = value.get(..want.len());
                    return head.is_some_and(|h| h.eq_ignore_ascii_case(want))
                        && matches!(value.as_bytes().get(want.len()), None | Some(b'-'));
                }
            }
        }
        cur = env.parent(n);
    }
    false
}

/// Elements whose ID is one of `tokens`, in document order: the ID values
/// the parser registered (DTD-declared ID attributes and xml:id).
fn find_ids(tokens: &[String], context: NodeRef, env: &Env) -> Vec<NodeRef> {
    let doc = env.doc(context);
    let mut out: Vec<NodeRef> = tokens
        .iter()
        .filter_map(|t| doc.element_by_id(t))
        .map(|n| env.at(context, n))
        .collect();
    sort_dedup(&mut out);
    out
}
