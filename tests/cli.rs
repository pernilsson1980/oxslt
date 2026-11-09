//! Command-line behaviour the differential cases can't express, checked
//! against what Debian's xsltproc does (recorded by hand in the comments).

use std::path::PathBuf;
use std::process::Command;

fn oxslt(args: &[&str]) -> (i32, String) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases/01-identity");
    let out = Command::new(env!("CARGO_BIN_EXE_oxslt"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("oxslt runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// `xsltproc style.xsl` with no input compiles the stylesheet, transforms
/// nothing and exits 0. Builds use it to check that a stylesheet exists.
#[test]
fn stylesheet_without_inputs() {
    assert_eq!(oxslt(&["style.xsl"]), (0, String::new()));
}

/// Compile errors count in the exit code only while processing an input:
/// xsltproc prints them and exits 0 when there's none, 5 when there is.
#[test]
fn compile_errors_without_inputs() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases/12-malformed-stylesheet");
    let style = dir.join("style.xsl");
    let style = style.to_str().unwrap();
    assert_eq!(oxslt(&[style]).0, 0);
    let input = dir.join("in.xml");
    assert_eq!(oxslt(&[style, input.to_str().unwrap()]).0, 5);
}

/// A missing stylesheet is exit 4 (failed to parse), with or without inputs;
/// no arguments at all is exit 1.
#[test]
fn missing_stylesheet_and_no_arguments() {
    assert_eq!(oxslt(&["missing.xsl"]).0, 4);
    assert_eq!(oxslt(&["missing.xsl", "in.xml"]).0, 4);
    assert_eq!(oxslt(&[]).0, 1);
}
