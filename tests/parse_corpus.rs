//! Every XML file in the fetched conformance suite and real-stylesheet cases
//! must parse, as libxml2 does. Where `scripts/record-tree-dumps.sh` has
//! recorded libxml2's view of a file, our tree must match it node for node.
//! Skips with a note when nothing has been fetched.

use std::fs;
use std::path::{Path, PathBuf};

use oxslt::catalog::{Catalog, ResourceLoader};
use oxslt::parse::{ParseOptions, parse_with};

/// The catalogs the recorded libxml2 trees were made with: the reference
/// container's /etc/xml/catalog. On a Mac with Homebrew's docbook packages
/// the same DTDs are under /opt/homebrew.
fn loader() -> ResourceLoader {
    let roots = match std::env::var("XML_CATALOG_FILES") {
        Ok(v) => v.split_ascii_whitespace().map(str::to_string).collect(),
        Err(_) => ["/opt/homebrew/etc/xml/catalog", "/etc/xml/catalog"]
            .iter()
            .filter(|p| std::path::Path::new(p).exists())
            .map(|p| p.to_string())
            .collect(),
    };
    ResourceLoader {
        catalog: Catalog::new(roots),
        paths: Vec::new(),
        refuse_all: false,
    }
}
use oxslt::tree::{Document, NamePool, NodeId, NodeKind, QName};

/// The listing tests/tree-dump.xsl produces with xsltproc, from our tree.
fn dump(doc: &Document, names: &NamePool) -> String {
    fn qname(names: &NamePool, q: QName) -> String {
        let ns = q.ns.map_or("", |n| names.str(n));
        format!("{{{ns}}}{}", names.str(q.local))
    }
    fn walk(doc: &Document, names: &NamePool, id: NodeId, out: &mut String) {
        match doc.kind(id) {
            NodeKind::Root => doc.children(id).for_each(|c| walk(doc, names, c, out)),
            NodeKind::Element { name, .. } => {
                out.push_str(&format!("E {}\n", qname(names, name)));
                for a in doc.attributes(id) {
                    let n = qname(names, doc.name(a).unwrap());
                    out.push_str(&format!("A {n}=[{}]\n", doc.value(a).unwrap()));
                }
                doc.children(id).for_each(|c| walk(doc, names, c, out));
            }
            NodeKind::Text(_) => out.push_str(&format!("T[{}]\n", doc.value(id).unwrap())),
            NodeKind::Comment(_) => out.push_str(&format!("C[{}]\n", doc.value(id).unwrap())),
            NodeKind::Pi { target, .. } => out.push_str(&format!(
                "P {}[{}]\n",
                names.str(target),
                doc.value(id).unwrap()
            )),
            NodeKind::Attribute { .. } => {}
        }
    }
    let mut out = String::new();
    walk(doc, names, doc.root(), &mut out);
    out
}

/// Files our parser rejects, each with the reason.
const EXPECTED_REJECTIONS: &[(&str, &str)] = &[
    (
        "conformance/xalan/copy/ent22.xml",
        "not a document: the external entity copy21/22 include",
    ),
    (
        "conformance/xalan/output/wml_11.xml",
        "not a document: a DTD with a .xml name",
    ),
];

fn xml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            xml_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("xml" | "xsl")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn every_corpus_file_parses() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files = Vec::new();
    for dir in ["conformance/xalan", "real", "cases", "docbook"] {
        xml_files(&root.join(dir), &mut files);
    }
    // Cases run with --html have HTML inputs, not XML.
    files.retain(|f| {
        let args = f.parent().map(|d| d.join("args"));
        !args.is_some_and(|a| fs::read_to_string(a).is_ok_and(|s| s.lines().any(|l| l == "--html")))
    });
    files.sort();
    if files.len() < 100 {
        println!(
            "conformance suite not fetched; checked {} files",
            files.len()
        );
    }

    let loader = loader();
    let mut failures = Vec::new();
    let mut tree_diffs = Vec::new();
    let mut compared = 0;
    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let bytes = fs::read(path).unwrap();
        let mut names = NamePool::new();
        let result = parse_with(
            &bytes,
            &mut names,
            &ParseOptions::default(),
            &loader,
            &Some(path.clone()),
        )
        .map(|p| p.document);
        // scripts/fetch-docbook.sh keeps a dump only where libxml2 accepted
        // the file, so in docbook/ a missing dump means a rejection: the
        // login.defs.d/*.xml entity fragments have several top-level elements.
        let expected_reject = EXPECTED_REJECTIONS.iter().any(|(f, _)| *f == rel)
            || (rel.starts_with("docbook/")
                && !root.join("tree-dumps").join(format!("{rel}.txt")).exists());
        match (result, expected_reject) {
            (Ok(doc), false) => {
                let recorded = root.join("tree-dumps").join(format!("{rel}.txt"));
                if let Ok(want) = fs::read_to_string(&recorded) {
                    compared += 1;
                    let got = dump(&doc, &names);
                    if got != want {
                        let line = got.lines().zip(want.lines()).position(|(a, b)| a != b);
                        let at = line.unwrap_or(got.lines().count().min(want.lines().count()));
                        let show = |s: &str| {
                            s.lines()
                                .nth(at)
                                .unwrap_or("<end>")
                                .chars()
                                .take(100)
                                .collect::<String>()
                        };
                        tree_diffs.push(format!(
                            "{rel}: entry {}: got {:?}, libxml2 {:?}",
                            at + 1,
                            show(&got),
                            show(&want)
                        ));
                    }
                }
            }
            (Err(_), true) => {}
            (Err(e), false) => failures.push(format!("{rel}: {e}")),
            (Ok(_), true) => failures.push(format!(
                "{rel}: now parses, remove from EXPECTED_REJECTIONS"
            )),
        }
    }
    println!("parsed {} files, {} problems", files.len(), failures.len());
    println!(
        "compared {compared} trees with libxml2, {} differ",
        tree_diffs.len()
    );
    for d in &tree_diffs {
        println!("  {d}");
    }
    failures.extend(tree_diffs);
    assert!(
        failures.is_empty(),
        "{} problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
