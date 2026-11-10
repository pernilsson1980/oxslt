//! Transform limits (`exec::Limits`): runaway stylesheets stop with a
//! controlled error, and ordinary ones aren't affected.

use std::path::Path;
use std::time::Duration;

use oxslt::catalog::ResourceLoader;
use oxslt::parse::{ParseOptions, parse_with};
use oxslt::tree::NamePool;
use oxslt::xslt::compile::compile_with;
use oxslt::xslt::exec::{ExecError, Limits, Transform};

const INPUT: &str = "<r><a>1</a><a>2</a><a>3</a><a>4</a></r>";

/// Runs `stylesheet` on INPUT with `limits`, on a large stack as the CLI
/// does. Ok(output) or the error.
fn run(stylesheet: &'static str, limits: Limits) -> Result<String, ExecError> {
    std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(move || {
            let loader = ResourceLoader::none();
            let options = ParseOptions::default();
            let mut names = NamePool::new();
            let style = parse_with(stylesheet.as_bytes(), &mut names, &options, &loader, &None)
                .expect("stylesheet parses");
            let ss = compile_with(
                &style.document,
                Path::new("limits.xsl"),
                &mut names,
                &loader,
                &options,
                &mut Vec::new(),
            )
            .expect("stylesheet compiles");
            let source = parse_with(INPUT.as_bytes(), &mut names, &options, &loader, &None)
                .expect("input parses");
            let result = Transform::run(
                &ss,
                &mut names,
                &loader,
                &options,
                source.document,
                Vec::new(),
                limits,
                &mut Vec::new(),
            )?;
            let bytes = oxslt::serialize::serialize(result, &mut names, &ss.output);
            Ok(String::from_utf8(bytes).unwrap())
        })
        .unwrap()
        .join()
        .unwrap()
}

fn work(n: u64) -> Limits {
    Limits {
        max_work: Some(n),
        ..Limits::default()
    }
}

fn assert_stopped(r: Result<String, ExecError>, what: &str) {
    match r {
        Err(ExecError::Terminated(m)) => assert!(m.contains(what), "unexpected message: {m}"),
        other => panic!("expected the transform to stop, got {other:?}"),
    }
}

/// Two calls per level, 30 levels: 2^30 calls.
const EXPONENTIAL: &str = r#"<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><xsl:call-template name="t"><xsl:with-param name="n" select="30"/></xsl:call-template></xsl:template>
  <xsl:template name="t"><xsl:param name="n"/>
    <xsl:if test="$n > 0">
      <xsl:call-template name="t"><xsl:with-param name="n" select="$n - 1"/></xsl:call-template>
      <xsl:call-template name="t"><xsl:with-param name="n" select="$n - 1"/></xsl:call-template>
    </xsl:if>
  </xsl:template>
</xsl:stylesheet>"#;

/// A string doubled 40 times: a terabyte.
const DOUBLING: &str = r#"<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><xsl:call-template name="t"><xsl:with-param name="s" select="'x'"/><xsl:with-param name="n" select="40"/></xsl:call-template></xsl:template>
  <xsl:template name="t"><xsl:param name="s"/><xsl:param name="n"/>
    <xsl:choose>
      <xsl:when test="$n > 0"><xsl:call-template name="t"><xsl:with-param name="s" select="concat($s, $s)"/><xsl:with-param name="n" select="$n - 1"/></xsl:call-template></xsl:when>
      <xsl:otherwise><xsl:value-of select="string-length($s)"/></xsl:otherwise>
    </xsl:choose>
  </xsl:template>
</xsl:stylesheet>"#;

/// Nested predicates over every node: polynomial XPath with no instructions.
const XPATH: &str = r#"<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><xsl:value-of select="count(//node()[//node()[//node()[//node()[//node()[//node()[//node()]]]]]])"/></xsl:template>
</xsl:stylesheet>"#;

const SMALL: &str = r#"<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:template match="/"><xsl:for-each select="//a"><xsl:value-of select=". * 2"/>,</xsl:for-each></xsl:template>
</xsl:stylesheet>"#;

#[test]
fn exponential_recursion_stops() {
    assert_stopped(run(EXPONENTIAL, work(1_000_000)), "work limit");
}

#[test]
fn string_doubling_stops() {
    assert_stopped(run(DOUBLING, work(1_000_000)), "work limit");
}

#[test]
fn xpath_work_counts() {
    assert_stopped(run(XPATH, work(100_000)), "work limit");
}

#[test]
fn timeout_stops() {
    let limits = Limits {
        timeout: Some(Duration::from_millis(200)),
        ..Limits::default()
    };
    let start = std::time::Instant::now();
    assert_stopped(run(EXPONENTIAL, limits), "time limit");
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn ordinary_transforms_are_unaffected() {
    assert_eq!(run(SMALL, work(1_000)).unwrap(), "2,4,6,8,");
    assert_eq!(run(SMALL, Limits::default()).unwrap(), "2,4,6,8,");
}
