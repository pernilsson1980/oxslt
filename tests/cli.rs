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

/// A stylesheet with compile errors is exit 5, with or without inputs
/// (xsltproc checks `cur->errors` before looking at the inputs).
#[test]
fn compile_errors_without_inputs() {
    for case in [
        "12-malformed-stylesheet",
        "42-global-redefinition",
        "44-pattern-extra-slashes",
    ] {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/cases")
            .join(case);
        let style = dir.join("style.xsl");
        let style = style.to_str().unwrap();
        assert_eq!(oxslt(&[style]).0, 5, "{case}");
        let input = dir.join("in.xml");
        assert_eq!(oxslt(&[style, input.to_str().unwrap()]).0, 5, "{case}");
    }
}

/// "-" reads the input from standard input, as libxml2's xmlFileOpen does:
/// `echo '<a>stdin</a>' | xsltproc style.xsl -` copies it.
#[test]
fn input_from_stdin() {
    use std::io::Write;
    use std::process::Stdio;
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases/01-identity");
    let mut child = Command::new(env!("CARGO_BIN_EXE_oxslt"))
        .current_dir(dir)
        .args(["style.xsl", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("oxslt runs");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"<a>stdin</a>")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "<?xml version=\"1.0\"?>\n<a>stdin</a>\n"
    );
}

/// `--catalogs` without $SGML_CATALOG_FILES only warns
/// ("Variable $SGML_CATALOG_FILES not set") and goes on with the default
/// catalogs.
#[test]
fn catalogs_without_variable() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases/01-identity");
    let out = Command::new(env!("CARGO_BIN_EXE_oxslt"))
        .current_dir(dir)
        .env_remove("SGML_CATALOG_FILES")
        .args(["--catalogs", "style.xsl", "in.xml"])
        .output()
        .expect("oxslt runs");
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Variable $SGML_CATALOG_FILES not set"));
}

/// A missing stylesheet is exit 4 (failed to parse), with or without inputs;
/// no arguments at all is exit 1.
#[test]
fn missing_stylesheet_and_no_arguments() {
    assert_eq!(oxslt(&["missing.xsl"]).0, 4);
    assert_eq!(oxslt(&["missing.xsl", "in.xml"]).0, 4);
    assert_eq!(oxslt(&[]).0, 1);
}

/// After an error libxslt only reports (its error state), the transform
/// goes on: a later xsl:message still prints, then exit 9 and no output.
/// After a failed XPath evaluation it stops (exit 10): nothing more runs.
#[test]
fn errors_that_go_on_and_errors_that_stop() {
    let run = |case: &str| {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/cases")
            .join(case);
        Command::new(env!("CARGO_BIN_EXE_oxslt"))
            .current_dir(dir)
            .args(["style.xsl", "in.xml"])
            .output()
            .expect("oxslt runs")
    };
    let goes_on = run("52-exit-error-goes-on");
    assert_eq!(goes_on.status.code(), Some(9));
    assert!(goes_on.stdout.is_empty());
    assert!(String::from_utf8_lossy(&goes_on.stderr).contains("still running"));
    let stops = run("50-exit-xpath-stops");
    assert_eq!(stops.status.code(), Some(10));
    assert!(stops.stdout.is_empty());
}
