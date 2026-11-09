//! XPath oracle: every expression in tests/xpath/expressions.txt, evaluated
//! on tests/xpath/doc.xml, must give exactly what xsltproc gave
//! (tests/xpath/expected.txt, recorded by scripts/record-xpath-oracle.sh).

use std::fs;
use std::path::PathBuf;

use oxslt::parse::{ParseOptions, parse};
use oxslt::tree::NamePool;
use oxslt::xpath::{self, Empty, Env, Focus, NodeRef, Value};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/xpath")
}

#[test]
fn xpath_matches_xsltproc() {
    let mut names = NamePool::new();
    let doc = parse(
        &fs::read(dir().join("doc.xml")).unwrap(),
        &mut names,
        &ParseOptions::default(),
    )
    .unwrap();
    let prefixes = [
        ("a", names.intern("urn:a")),
        ("c", names.intern("urn:c")),
        ("d", names.intern("urn:default")),
        ("xml", names.intern("http://www.w3.org/XML/1998/namespace")),
    ];
    let resolve = |p: &str| prefixes.iter().find(|(q, _)| *q == p).map(|&(_, s)| s);

    let expected = fs::read_to_string(dir().join("expected.txt")).unwrap();
    let lines: Vec<String> = fs::read_to_string(dir().join("expressions.txt"))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(str::to_owned)
        .collect();

    // Compile everything first: compiling needs the name pool mutably.
    let compiled: Vec<_> = lines
        .iter()
        .map(|line| {
            let expr = line.strip_prefix("n ").unwrap_or(line);
            (
                line.starts_with("n "),
                expr.to_string(),
                xpath::compile(expr, &mut names, &resolve),
            )
        })
        .collect();

    let docs = [doc];
    let env = Env {
        docs: &docs,
        names: &names,
        vars: &Empty,
        funcs: &Empty,
    };
    let focus = Focus::single(NodeRef::new(0, docs[0].root()));

    let mut failures = Vec::new();
    let mut rest = expected.as_str();
    for (k, (list_nodes, expr, compiled)) in compiled.iter().enumerate() {
        let k = k + 1;
        let header = format!("[{k}] {expr}\n");
        let next_header = format!("[{}] ", k + 1);
        assert!(
            rest.starts_with(&header),
            "expected.txt is out of date at entry {k}; rerun scripts/record-xpath-oracle.sh"
        );
        let end = if k == lines.len() {
            rest.len()
        } else {
            rest.find(&format!("\n{next_header}"))
                .map_or(rest.len(), |i| i + 1)
        };
        let want = &rest[header.len()..end];
        rest = &rest[end..];

        let got = match compiled {
            Err(e) => format!("compile error: {e}\n"),
            Ok(ast) => match xpath::evaluate(ast, &env, &focus) {
                Err(e) => format!("evaluation error: {e}\n"),
                Ok(value) => {
                    let mut s = format!("= {}\n", xpath::to_string(&value, &env));
                    if *list_nodes {
                        s.push_str("nodes:");
                        if let Value::NodeSet(nodes) = &value {
                            for &n in nodes {
                                let name = env.qualified_name(n);
                                let len = env.string_value(n).chars().count();
                                s.push_str(&format!(" {name}/{len}"));
                            }
                        }
                        s.push('\n');
                    }
                    s
                }
            },
        };
        if got != want {
            failures.push(format!(
                "[{k}] {expr}\n  xsltproc: {want:?}\n  oxslt:    {got:?}"
            ));
        }
    }
    println!(
        "{} of {} expressions match xsltproc",
        compiled.len() - failures.len(),
        compiled.len()
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
