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

/// xsltproc's argument handling, recorded from Debian's xsltproc
/// (tests/cases can't express arguments after the stylesheet).
mod arguments {
    use super::*;

    fn identity() -> String {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases/01-identity");
        std::fs::read_to_string(dir.join("expected/stdout")).unwrap()
    }

    /// `--version` prints the library versions to standard output and goes
    /// on; with nothing else to do, exit 0.
    #[test]
    fn version() {
        let (code, out) = oxslt(&["--version"]);
        assert_eq!(code, 0);
        assert!(out.starts_with("Using libxml 20914, libxslt 10135 and libexslt 820\n"));
        assert_eq!(out.lines().count(), 4);
        let (code, out) = oxslt(&["-V", "style.xsl", "in.xml"]);
        assert_eq!(code, 0);
        assert!(out.ends_with(&identity()));
    }

    /// Options alone, without a stylesheet, do nothing: exit 0.
    #[test]
    fn options_without_stylesheet() {
        assert_eq!(oxslt(&["--nonet"]), (0, String::new()));
        assert_eq!(oxslt(&["-o"]), (0, String::new()));
        assert_eq!(oxslt(&["--stringparam", "p"]), (0, String::new()));
    }

    /// An unknown option prints the usage to standard output: exit 3.
    #[test]
    fn unknown_option() {
        let (code, out) = oxslt(&["style.xsl", "in.xml", "--bogus"]);
        assert_eq!(code, 3);
        assert!(out.contains("[options] stylesheet file [file ...]"));
        assert!(out.contains("\t--xincludestyle : do XInclude processing on stylesheets\n"));
    }

    /// Every argument after the stylesheet is an input, options included:
    /// they're also read as options, then fail to parse as files (exit 6).
    #[test]
    fn options_after_the_stylesheet_are_inputs() {
        // --noout still applies: no output.
        let (code, out) = oxslt(&["style.xsl", "in.xml", "--noout"]);
        assert_eq!((code, out), (6, String::new()));
        let (code, out) = oxslt(&["style.xsl", "--stringparam", "p", "v", "in.xml"]);
        assert_eq!((code, out), (6, identity()));
        // "-" stops the options; it's the stylesheet here.
        assert_eq!(oxslt(&["--noout", "-", "style.xsl"]).0, 4);
    }

    /// `--noout` writes no result, but `-o` still writes its file.
    #[test]
    fn noout() {
        assert_eq!(
            oxslt(&["--noout", "style.xsl", "in.xml"]),
            (0, String::new())
        );
        let dir = std::env::temp_dir().join(format!("oxslt-noout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("out.xml");
        let f = file.to_str().unwrap();
        assert_eq!(
            oxslt(&["-o", f, "--noout", "style.xsl", "in.xml"]),
            (0, String::new())
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), identity());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A missing `--maxdepth` value is exit 2, as is a 32nd parameter
    /// (xsltproc's MAX_PARAMETERS counts names and values: 64).
    #[test]
    fn exit_2() {
        assert_eq!(oxslt(&["style.xsl", "in.xml", "--maxdepth"]).0, 2);
        assert_eq!(oxslt(&["--maxparserdepth"]).0, 2);
        let mut args = Vec::new();
        for _ in 0..31 {
            args.extend(["--stringparam", "p", "v"]);
        }
        args.extend(["style.xsl", "in.xml"]);
        // 31 parameters of one name: accepted, but duplicates (exit 9).
        assert_eq!(oxslt(&args).0, 9);
        args.splice(0..0, ["--stringparam", "p", "v"]);
        assert_eq!(oxslt(&args).0, 2);
    }

    /// With `--encoding`, a byte-order mark isn't skipped: it's content
    /// before the root, even when it matches the encoding.
    #[test]
    fn forced_encoding_keeps_the_bom() {
        let dir = std::env::temp_dir().join(format!("oxslt-bom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bom.xml");
        std::fs::write(&file, b"\xef\xbb\xbf<doc/>").unwrap();
        let f = file.to_str().unwrap();
        assert_eq!(oxslt(&["style.xsl", f]).0, 0);
        assert_eq!(oxslt(&["--encoding", "UTF-8", "style.xsl", f]).0, 6);
        assert_eq!(oxslt(&["--encoding", "nonsense", "style.xsl", f]).0, 6);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// With a declared html method and a DOCTYPE, libxslt's result always holds
/// the DOCTYPE, so `-o` writes the file even when nothing else is output: an
/// empty file (mplayer's chunked builds). Other methods write nothing.
#[test]
fn empty_html_result_with_doctype_still_writes_the_file() {
    let dir = std::env::temp_dir().join(format!("oxslt-html-doctype-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let case = |method: &str| {
        let style = dir.join(format!("{method}.xsl"));
        std::fs::write(
            &style,
            format!(
                "<xsl:stylesheet version='1.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'>\
                 <xsl:output method='{method}' doctype-system='x.dtd'/>\
                 <xsl:template match='/'/></xsl:stylesheet>"
            ),
        )
        .unwrap();
        let out = dir.join(format!("{method}.out"));
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases/01-identity/in.xml");
        let code = oxslt(&[
            "-o",
            out.to_str().unwrap(),
            style.to_str().unwrap(),
            input.to_str().unwrap(),
        ])
        .0;
        (code, std::fs::read(&out).ok().map(|b| b.len()))
    };
    assert_eq!(case("html"), (0, Some(0)));
    assert_eq!(case("xml"), (0, None));
    std::fs::remove_dir_all(&dir).unwrap();
}
