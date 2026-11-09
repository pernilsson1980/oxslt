//! Any input must either parse or return an error: never panic, hang or
//! use unbounded memory. When it parses, the tree must be consistent.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxslt::parse::{ParseOptions, parse};
use oxslt::tree::{NamePool, NodeKind};

fuzz_target!(|data: &[u8]| {
    let mut names = NamePool::new();
    let Ok(doc) = parse(data, &mut names, &ParseOptions::default()) else {
        return;
    };
    // Walk the whole tree: every child's parent is the node it came from, and
    // children come after their parent in document order.
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        for a in doc.attributes(id) {
            assert_eq!(doc.parent(a), Some(id));
            assert!(matches!(doc.kind(a), NodeKind::Attribute { .. }));
        }
        for c in doc.children(id) {
            assert_eq!(doc.parent(c), Some(id));
            assert!(c > id);
            stack.push(c);
        }
        let _ = doc.string_value(id);
    }
});
