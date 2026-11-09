//! Roadmap step 3's gate: DocBook XSL's single-page HTML stylesheet
//! (`html/docbook.xsl`) over every DocBook document in `tests/docbook/`,
//! compared byte for byte, with the exit code, against xsltproc's output
//! recorded by `scripts/record-docbook-html.sh`.
//!
//! The stylesheets, DTDs and catalogs are the reference container's own,
//! copied by `scripts/fetch-docbook-xsl.sh`. Without them, or without the
//! recorded output, the test prints a note and passes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|x| x == "exit") {
            out.push(p);
        }
    }
}

#[test]
fn docbook_html_matches_xsltproc() {
    let tests = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let expected = tests.join("docbook-html");
    let xsl_root = tests.join("docbook-xsl");
    let stylesheet = xsl_root.join("usr/share/xml/docbook/stylesheet/docbook-xsl/html/docbook.xsl");
    let catalog = xsl_root.join("etc/xml/catalog");
    let mut exits = Vec::new();
    files(&expected, &mut exits);
    exits.sort();
    if exits.is_empty() || !stylesheet.exists() {
        println!(
            "DocBook HTML output not recorded; run scripts/fetch-docbook.sh, \
             scripts/fetch-docbook-xsl.sh and scripts/record-docbook-html.sh"
        );
        return;
    }

    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunks: Vec<Vec<PathBuf>> = (0..threads)
        .map(|t| exits.iter().skip(t).step_by(threads).cloned().collect())
        .collect();
    let (tests, expected, stylesheet, catalog) = (&tests, &expected, &stylesheet, &catalog);
    let problems: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                s.spawn(move || {
                    let mut problems = Vec::new();
                    for exit_file in chunk {
                        // tests/docbook-html/<rel>.xml.exit → tests/docbook/<rel>.xml
                        let rel = exit_file
                            .strip_prefix(expected)
                            .unwrap()
                            .with_extension("");
                        let input = tests.join("docbook").join(&rel);
                        let output = Command::new(env!("CARGO_BIN_EXE_oxslt"))
                            .current_dir(input.parent().unwrap())
                            .env("XML_CATALOG_FILES", catalog)
                            .arg("--nonet")
                            .arg(stylesheet)
                            .arg(input.file_name().unwrap())
                            .stdin(Stdio::null())
                            .stderr(Stdio::null())
                            .output()
                            .expect("failed to run oxslt");
                        let want_exit = fs::read_to_string(exit_file).unwrap_or_default();
                        let want = fs::read(exit_file.with_extension("html")).unwrap_or_default();
                        let exit = output.status.code().unwrap_or(-1).to_string();
                        let rel = rel.display();
                        if exit != want_exit.trim() {
                            problems.push(format!("{rel}: exit {exit}, xsltproc {}", want_exit.trim()));
                        } else if output.stdout != want {
                            let at = output
                                .stdout
                                .iter()
                                .zip(&want)
                                .position(|(a, b)| a != b)
                                .unwrap_or(output.stdout.len().min(want.len()));
                            let show = |b: &[u8]| {
                                String::from_utf8_lossy(&b[at.saturating_sub(40)..(at + 60).min(b.len())])
                                    .into_owned()
                            };
                            problems.push(format!(
                                "{rel}: differs at byte {at}:\n    got      {:?}\n    xsltproc {:?}",
                                show(&output.stdout),
                                show(&want)
                            ));
                        }
                    }
                    problems
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    println!(
        "DocBook HTML: {} of {} documents identical to xsltproc",
        exits.len() - problems.len(),
        exits.len()
    );
    assert!(
        problems.is_empty(),
        "{} differ:\n{}",
        problems.len(),
        problems.join("\n")
    );
}
