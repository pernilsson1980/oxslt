//! Compiles every XPath expression in the corpus stylesheets: the `select`,
//! `test`, `match`, `count`, `from`, `use` and `value` attributes of XSLT
//! elements, with prefixes resolved from each element's in-scope namespaces.
//! Patterns (`match`, `count`, `from`) are a subset of expression syntax, so
//! they compile as expressions too. Skips with a note when the conformance
//! suite hasn't been fetched.

use std::fs;
use std::path::{Path, PathBuf};

use oxslt::parse::{ParseOptions, parse};
use oxslt::tree::{Document, NamePool, NodeId, NodeKind, Sym};
use oxslt::xpath;

const XSLT_NS: &str = "http://www.w3.org/1999/XSL/Transform";
const ATTRS: &[&str] = &["select", "test", "match", "count", "from", "use", "value"];

/// Expressions xsltproc also rejects, by file: these tests check error handling.
const EXPECTED_ERRORS: &[&str] = &["cases/12-malformed-stylesheet/style.xsl"];

fn xsl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            xsl_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "xsl") {
            out.push(path);
        }
    }
}

/// The URI bound to `prefix` at element `el`, from its and its ancestors' declarations.
fn resolve_at(doc: &Document, names: &NamePool, el: NodeId, prefix: &str) -> Option<Sym> {
    if prefix == "xml" {
        return names.get("http://www.w3.org/XML/1998/namespace");
    }
    let mut cur = Some(el);
    while let Some(e) = cur {
        for d in doc.ns_decls(e) {
            if d.prefix.is_some_and(|p| names.str(p) == prefix) {
                return Some(d.uri);
            }
        }
        cur = doc.parent(e);
    }
    None
}

#[test]
fn every_corpus_expression_compiles() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files = Vec::new();
    for dir in ["conformance/xalan", "real", "cases"] {
        xsl_files(&root.join(dir), &mut files);
    }
    files.sort();

    let (mut compiled, mut failures) = (0, Vec::new());
    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut names = NamePool::new();
        names.intern("http://www.w3.org/XML/1998/namespace");
        let Ok(doc) = parse(
            &fs::read(path).unwrap(),
            &mut names,
            &ParseOptions::default(),
        ) else {
            continue; // parse_corpus.rs covers files that don't parse
        };
        let Some(xslt) = names.get(XSLT_NS) else {
            continue;
        };

        // Collect (element, expression) first: compiling needs the pool mutably.
        let mut exprs = Vec::new();
        for el in doc.all_nodes() {
            let NodeKind::Element { name, .. } = doc.kind(el) else {
                continue;
            };
            if name.ns != Some(xslt) {
                continue;
            }
            for a in doc.attributes(el) {
                let q = doc.name(a).unwrap();
                if q.ns.is_none() && ATTRS.contains(&names.str(q.local)) {
                    exprs.push((el, doc.value(a).unwrap().to_string()));
                }
            }
        }
        let mut file_failures = Vec::new();
        for (el, expr) in exprs {
            // Compile into a scratch pool: the stylesheet's pool is borrowed by
            // the resolver. Mixing the two is fine for a compile-only check.
            let resolve = |p: &str| resolve_at(&doc, &names, el, p);
            let mut scratch = NamePool::new();
            match xpath::compile(&expr, &mut scratch, &resolve) {
                Ok(_) => compiled += 1,
                Err(e) => file_failures.push(format!("{rel}: {expr:?}: {e}")),
            }
        }
        let expected = EXPECTED_ERRORS.contains(&rel.as_str());
        match (file_failures.is_empty(), expected) {
            (true, false) | (false, true) => {}
            (false, false) => failures.extend(file_failures),
            (true, true) => {
                failures.push(format!("{rel}: now compiles, remove from EXPECTED_ERRORS"))
            }
        }
    }
    println!(
        "compiled {compiled} expressions from {} stylesheets, {} problems",
        files.len(),
        failures.len()
    );
    for f in &failures {
        println!("  {f}");
    }
    assert!(failures.is_empty(), "{} problems", failures.len());
}
