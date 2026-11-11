//! Any input must parse as HTML (libxml2's HTML parser never fails) without
//! panicking or hanging, and give a consistent tree.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxslt::tree::{NamePool, NodeKind};

fuzz_target!(|data: &[u8]| {
    let mut names = NamePool::new();
    let doc = oxslt::html::parse(data, &mut names, "fuzz.html").document;
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
