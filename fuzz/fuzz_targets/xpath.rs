//! Any string must compile or give an error, and any compiled expression must
//! evaluate or give an error: never panic or overflow the stack. Node-sets
//! must come back sorted and without duplicates.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxslt::parse::{ParseOptions, parse};
use oxslt::tree::NamePool;
use oxslt::xpath::{self, Empty, Env, Focus, NodeRef, Value};

const DOC: &[u8] = br#"<r xmlns:a="urn:a" xml:lang="en"><p n="1" xml:id="i">x<b>y</b><!--c--><?pi d?></p><a:q a:k="2">3</a:q><p n="2">4</p></r>"#;

fuzz_target!(|data: &[u8]| {
    let Ok(expr) = std::str::from_utf8(data) else { return };
    let mut names = NamePool::new();
    let doc = parse(DOC, &mut names, &ParseOptions::default()).unwrap();
    let a = names.intern("urn:a");
    let resolve = move |p: &str| (p == "a").then_some(a);
    let Ok(ast) = xpath::compile(expr, &mut names, &resolve) else { return };
    let docs = [doc];
    let env = Env { docs: &docs, names: &names, vars: &Empty, funcs: &Empty };
    let root = NodeRef::new(0, docs[0].root());
    if let Ok(Value::NodeSet(nodes)) = xpath::evaluate(&ast, &env, &Focus::single(root)) {
        assert!(nodes.windows(2).all(|w| w[0] < w[1]), "node-set not sorted or has duplicates");
        // Evaluating relative to each result node exercises every axis from it.
        for &n in nodes.iter().take(8) {
            let _ = xpath::evaluate(&ast, &env, &Focus::single(n));
        }
    }
});
