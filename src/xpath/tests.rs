//! Unit tests for what the xsltproc oracle (tests/xpath_oracle.rs) can't
//! cover: errors, limits on hostile input, and the variable and function
//! hooks XSLT uses.

use super::*;
use crate::parse::{ParseOptions, parse};

fn setup(xml: &str) -> (NamePool, Vec<crate::tree::Document>) {
    let mut names = NamePool::new();
    let doc = parse(xml.as_bytes(), &mut names, &ParseOptions::default()).unwrap();
    (names, vec![doc])
}

fn no_prefixes(_: &str) -> Option<crate::tree::Sym> {
    None
}

fn compile_err(expr: &str) -> XPathError {
    let mut names = NamePool::new();
    compile(expr, &mut names, &no_prefixes).unwrap_err()
}

fn eval_err(expr: &str) -> XPathError {
    let (mut names, docs) = setup("<d><e>1</e></d>");
    let ast = compile(expr, &mut names, &no_prefixes).unwrap();
    let env = Env {
        docs: &docs,
        names: &names,
        vars: &Empty,
        funcs: &Empty,
    };
    evaluate(&ast, &env, &Focus::single(NodeRef::new(0, docs[0].root()))).unwrap_err()
}

#[test]
fn syntax_errors() {
    for bad in [
        "", "1 +", "(1", "a[", "//", "@", "f(1,", "1 2", "child::", "foo::a", "'x", "a/", "$",
        "..a", "a:b:c",
    ] {
        let e = compile_err(bad);
        assert_eq!(e.kind, ErrorKind::Syntax, "{bad:?}: {e}");
    }
    assert!(
        compile_err("p:x")
            .message
            .contains("Undefined namespace prefix p")
    );
    assert!(
        compile_err("p:*")
            .message
            .contains("Undefined namespace prefix p")
    );
    assert!(
        compile_err("$p:v")
            .message
            .contains("Undefined namespace prefix p")
    );
}

#[test]
fn hostile_nesting_is_rejected_without_overflowing_the_stack() {
    let deep_parens = "(".repeat(100_000) + "1" + &")".repeat(100_000);
    let long_chain = vec!["1"; 100_000].join("+");
    let negations = "-".repeat(100_000) + "1";
    let predicates = "a".to_string() + &"[a".repeat(5_000) + &"]".repeat(5_000);
    for expr in [deep_parens, long_chain, negations, predicates] {
        assert_eq!(compile_err(&expr).message, "Expression too deeply nested");
    }
    // Long but shallow expressions are fine: path steps aren't nested.
    let mut names = NamePool::new();
    let long_path = vec!["a"; 10_000].join("/");
    assert!(compile(&long_path, &mut names, &no_prefixes).is_ok());
    let nested_ok = "(".repeat(MAX_NESTING - 1) + "1" + &")".repeat(MAX_NESTING - 1);
    assert!(compile(&nested_ok, &mut names, &no_prefixes).is_ok());

    // The tallest allowed tree must also evaluate without overflowing a test
    // thread's 2 MB stack, in a debug build.
    let (mut names, docs) = setup("<d/>");
    let tallest = vec!["1"; MAX_HEIGHT].join("+");
    let ast = compile(&tallest, &mut names, &no_prefixes).unwrap();
    let env = Env {
        docs: &docs,
        names: &names,
        vars: &Empty,
        funcs: &Empty,
    };
    let value = evaluate(&ast, &env, &Focus::single(NodeRef::new(0, docs[0].root()))).unwrap();
    assert_eq!(value, Value::Number(MAX_HEIGHT as f64));
}

#[test]
fn evaluation_errors() {
    assert_eq!(eval_err("$v").kind, ErrorKind::Runtime);
    assert!(eval_err("$v").message.contains("Undefined variable v"));
    assert!(
        eval_err("nosuch()")
            .message
            .contains("Unregistered function")
    );
    assert_eq!(eval_err("1 | 2").kind, ErrorKind::Type);
    assert_eq!(eval_err("count(1)").kind, ErrorKind::Type);
    assert_eq!(eval_err("sum('a')").kind, ErrorKind::Type);
    assert_eq!(eval_err("'a'[1]").kind, ErrorKind::Type);
    assert_eq!(eval_err("(1)/a").kind, ErrorKind::Type);
    for arity in [
        "count()",
        "concat('a')",
        "true(1)",
        "substring('a')",
        "not()",
        "last(1)",
    ] {
        assert!(
            eval_err(arity)
                .message
                .contains("Invalid number of arguments"),
            "{arity}"
        );
    }
}

#[test]
fn variables_and_extension_functions() {
    struct Vars(crate::tree::Sym);
    impl Variables for Vars {
        fn get(&self, name: ast::Name) -> Option<Value> {
            (name == (None, self.0)).then_some(Value::Number(41.0))
        }
    }
    struct Funcs(crate::tree::Sym, crate::tree::Sym);
    impl Functions for Funcs {
        fn call(
            &self,
            name: ast::Name,
            args: Vec<Value>,
            env: &Env,
            focus: &Focus,
        ) -> Option<Result<Value, XPathError>> {
            if name != (Some(self.0), self.1) {
                return None;
            }
            // ext:twice(x) = 2 * number(x), and it can see the focus.
            let x = to_number(&args[0], env);
            Some(Ok(Value::Number(2.0 * x + focus.position as f64 - 1.0)))
        }
    }
    let (mut names, docs) = setup("<d><e>1</e><e>2</e></d>");
    let ext = names.intern("urn:ext");
    let resolve = move |p: &str| (p == "ext").then_some(ext);
    let ast = compile("ext:twice($v) + sum(//e)", &mut names, &resolve).unwrap();
    let (v, twice) = (names.get("v").unwrap(), names.get("twice").unwrap());
    let env = Env {
        docs: &docs,
        names: &names,
        vars: &Vars(v),
        funcs: &Funcs(ext, twice),
    };
    let value = evaluate(&ast, &env, &Focus::single(NodeRef::new(0, docs[0].root()))).unwrap();
    assert_eq!(value, Value::Number(85.0));
}

#[test]
fn node_sets_from_several_documents_sort_by_document() {
    let mut names = NamePool::new();
    let a = parse(b"<a/>", &mut names, &ParseOptions::default()).unwrap();
    let b = parse(b"<b/>", &mut names, &ParseOptions::default()).unwrap();
    let docs = [a, b];
    let mut nodes = vec![
        NodeRef::new(1, docs[1].root()),
        NodeRef::new(0, docs[0].root()),
        NodeRef::new(1, docs[1].root()),
    ];
    sort_dedup(&mut nodes);
    assert_eq!(
        nodes,
        vec![
            NodeRef::new(0, docs[0].root()),
            NodeRef::new(1, docs[1].root())
        ]
    );
}
