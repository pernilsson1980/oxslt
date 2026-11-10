//! Xalan XSLT 1.0 conformance tests, judged against xsltproc's output.
//!
//! The suite, its manifest and xsltproc's recorded output are fetched by
//! `scripts/fetch-conformance.sh` and git-ignored. Without them this test
//! prints a note and passes.
//!
//! The PoC goal is 80% of the tests the manifest marks `poc`. `passing.txt`
//! (committed) lists every test that passes. The test fails if a listed test
//! stops passing; tests that newly pass are reported. Run with
//! `OXSLT_UPDATE_CONFORMANCE=1` to rewrite `passing.txt` from the results.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{lines, run_and_compare};

const POC_TARGET_PERCENT: usize = 80;

struct Test {
    id: String,
    category: String,
    stylesheet: String,
    input: String,
    poc: bool,
    /// Why a test is out of PoC scope: "number", "import", "key", …
    reasons: String,
}

fn conf_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance")
}

fn manifest() -> Option<Vec<Test>> {
    let text = fs::read_to_string(conf_dir().join("manifest.tsv")).ok()?;
    let tests = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Test {
                id: f[0].into(),
                category: f[1].into(),
                stylesheet: f[2].into(),
                input: f[3].into(),
                poc: f[4] == "poc",
                reasons: f.get(5).unwrap_or(&"").to_string(),
            }
        })
        .collect();
    Some(tests)
}

#[test]
fn conformance() {
    let Some(tests) = manifest() else {
        println!("conformance suite not fetched; run scripts/fetch-conformance.sh");
        return;
    };
    let passing_path = conf_dir().join("passing.txt");
    let listed: std::collections::BTreeSet<String> = lines(&passing_path)
        .into_iter()
        .filter(|l| !l.starts_with('#'))
        .collect();

    // OXSLT_CONFORMANCE_VERBOSE=1 lists every failing PoC test;
    // OXSLT_CONFORMANCE_VERBOSE=later lists the failing later tests too.
    let verbose = std::env::var_os("OXSLT_CONFORMANCE_VERBOSE");
    let verbose_later = verbose.as_deref() == Some("later".as_ref());
    let verbose = verbose.is_some();
    let mut passed = Vec::new();
    let mut regressions = Vec::new();
    let mut new_passes = Vec::new();
    for t in &tests {
        let dir = conf_dir().join("xalan").join(&t.category);
        // Third-party stylesheets never write files (CLAUDE.md, Security).
        let args = [
            "--nowrite".to_string(),
            t.stylesheet.clone(),
            t.input.clone(),
        ];
        let problems = run_and_compare(&dir, &args, &[], &conf_dir().join("expected").join(&t.id));
        let ok = problems.is_empty();
        if !ok && t.poc && verbose {
            println!("FAIL {}\t{}", t.id, problems.join("; "));
        }
        if !ok && !t.poc && verbose_later {
            println!("LATER {}\t[{}]\t{}", t.id, t.reasons, problems.join("; "));
        }
        match (ok, listed.contains(&t.id)) {
            (true, false) => new_passes.push(t.id.clone()),
            (false, true) => regressions.push(t.id.clone()),
            _ => {}
        }
        if ok {
            passed.push(t);
        }
    }

    let poc_total = tests.iter().filter(|t| t.poc).count();
    let poc_passed = passed.iter().filter(|t| t.poc).count();
    let later_total = tests.len() - poc_total;
    let target = (poc_total * POC_TARGET_PERCENT).div_ceil(100);
    println!(
        "PoC scope: {poc_passed} of {poc_total} pass ({:.1}%); the {POC_TARGET_PERCENT}% target is {target}",
        100.0 * poc_passed as f64 / poc_total.max(1) as f64
    );
    println!(
        "later:     {} of {later_total} pass",
        passed.len() - poc_passed
    );
    if !new_passes.is_empty() {
        println!(
            "newly passing ({}): {}",
            new_passes.len(),
            new_passes.join(" ")
        );
    }

    if std::env::var_os("OXSLT_UPDATE_CONFORMANCE").is_some() {
        let mut text = String::from(
            "# Conformance tests that pass. Rewritten by OXSLT_UPDATE_CONFORMANCE=1 cargo test.\n",
        );
        for t in &passed {
            text.push_str(&t.id);
            text.push('\n');
        }
        fs::write(&passing_path, text).unwrap();
        println!("rewrote {}", passing_path.display());
        return;
    }
    assert!(
        regressions.is_empty(),
        "{} previously passing tests now fail: {}",
        regressions.len(),
        regressions.join(" ")
    );
}
