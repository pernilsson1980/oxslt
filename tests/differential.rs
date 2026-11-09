//! Differential tests: oxslt must match xsltproc's output bytes and exit code.
//!
//! Each case is a directory under `tests/cases/` (ours, committed) or
//! `tests/real/` (third-party stylesheets fetched by
//! `scripts/fetch-real-stylesheets.sh`, never committed), holding:
//!
//! - `style.xsl` and `in.xml`: the stylesheet and input
//! - `args`: optional extra arguments, one per line, placed before `style.xsl`
//! - `env`: optional `KEY=VALUE` defaults; a variable already set wins
//! - `requires`: optional paths, one per line, relative to the case, that
//!   must exist for the case to run, such as fetched DocBook stylesheets.
//!   When one is missing the case is skipped with a note, as the other
//!   comparison tests do without their fetched data.
//! - `expected/stdout`, `expected/exit`: xsltproc's result, recorded by
//!   `scripts/record-expected.sh`
//!
//! Cases listed in `tests/known-failures.txt` are expected to fail. The test
//! fails if any other case fails, and also if a listed case starts passing,
//! so the list stays accurate. Run with `OXSLT_UPDATE_KNOWN_FAILURES=1` to
//! rewrite the list from the current results.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use common::{lines, run_and_compare};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn case_dirs() -> Vec<(String, PathBuf)> {
    let mut cases = Vec::new();
    for group in ["cases", "real"] {
        let Ok(entries) = fs::read_dir(root().join("tests").join(group)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("style.xsl").is_file() {
                let name = format!("{group}/{}", entry.file_name().to_string_lossy());
                cases.push((name, path));
            }
        }
    }
    cases.sort();
    cases
}

#[test]
fn differential() {
    let known_path = root().join("tests/known-failures.txt");
    let known: BTreeSet<String> = lines(&known_path)
        .into_iter()
        .filter(|l| !l.starts_with('#'))
        .collect();

    let mut failing = Vec::new();
    let mut unexpected_failures = Vec::new();
    let mut unexpected_passes = Vec::new();
    let cases = case_dirs();
    assert!(!cases.is_empty(), "no test cases found");

    let mut skipped = 0;
    for (name, dir) in &cases {
        if let Some(missing) = lines(&dir.join("requires"))
            .into_iter()
            .find(|p| !dir.join(p).exists())
        {
            println!("{:>14}  {name}  needs {missing}", "skip");
            skipped += 1;
            continue;
        }
        let mut args = lines(&dir.join("args"));
        args.extend(["style.xsl".to_string(), "in.xml".to_string()]);
        let problems = run_and_compare(dir, &args, &lines(&dir.join("env")), &dir.join("expected"));
        let passed = problems.is_empty();
        let mark = match (passed, known.contains(name)) {
            (true, false) => "pass",
            (false, true) => "known failure",
            (false, false) => {
                unexpected_failures.push(name.clone());
                "FAIL"
            }
            (true, true) => {
                unexpected_passes.push(name.clone());
                "PASS (listed as known failure)"
            }
        };
        if !passed {
            failing.push(name.clone());
        }
        println!("{mark:>14}  {name}  {}", problems.join("; "));
    }
    println!(
        "\n{} of {} cases pass, {skipped} skipped",
        cases.len() - skipped - failing.len(),
        cases.len() - skipped
    );

    if std::env::var_os("OXSLT_UPDATE_KNOWN_FAILURES").is_some() {
        let mut text = String::from(
            "# Cases expected to fail. Rewritten by OXSLT_UPDATE_KNOWN_FAILURES=1 cargo test.\n",
        );
        for name in &failing {
            text.push_str(name);
            text.push('\n');
        }
        fs::write(&known_path, text).unwrap();
        println!("rewrote {}", known_path.display());
        return;
    }
    assert!(
        unexpected_failures.is_empty() && unexpected_passes.is_empty(),
        "unexpected failures: {unexpected_failures:?}; now passing, remove from known-failures.txt: {unexpected_passes:?}"
    );
}
