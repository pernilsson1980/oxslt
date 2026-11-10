//! Whole transforms: any stylesheet and input must transform and
//! serialize, or fail with an error, without panicking, overflowing the
//! stack or running away. The input is the stylesheet, a NUL byte and the
//! source document; without a NUL, a small built-in document is the source.
//!
//! A work budget bounds each run, and the loader refuses every file, so
//! xsl:include, document() and external DTDs can't read the disk.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxslt::catalog::ResourceLoader;
use oxslt::parse::{ParseOptions, parse_with};
use oxslt::tree::NamePool;
use oxslt::xslt::compile::compile_with;
use oxslt::xslt::exec::{Limits, Transform};

const DOC: &[u8] = br#"<r xmlns:a="urn:a" xml:lang="en"><p n="1" xml:id="i">x<b>y</b><!--c--><?pi d?></p><a:q a:k="2">3</a:q><p n="2">4</p></r>"#;

fn transform(data: &[u8]) {
    let (style, input) = match data.iter().position(|&b| b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, DOC),
    };
    let loader = ResourceLoader::none();
    let options = ParseOptions::default();
    let mut names = NamePool::new();
    let path = Some(std::path::PathBuf::from("fuzz.xsl"));
    let Ok(style_doc) = parse_with(style, &mut names, &options, &loader, &path) else {
        return;
    };
    let mut warnings = Vec::new();
    let Ok(ss) = compile_with(
        &style_doc.document,
        std::path::Path::new("fuzz.xsl"),
        &mut names,
        &loader,
        &options,
        &mut warnings,
    ) else {
        return;
    };
    let Ok(source) = parse_with(input, &mut names, &options, &loader, &None) else {
        return;
    };
    let limits = Limits {
        max_work: Some(200_000),
        timeout: None,
        ..Limits::default()
    };
    let mut messages = Vec::new();
    if let Ok(result) = Transform::run(
        &ss,
        &mut names,
        &loader,
        &options,
        source.document,
        Vec::new(),
        limits,
        &mut messages,
    ) {
        let _ = oxslt::serialize::serialize(result, &mut names, &ss.output);
    }
}

fuzz_target!(|data: &[u8]| {
    // The CLI runs transforms on a thread with a large stack; so does this.
    let data = data.to_vec();
    std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(move || transform(&data))
        .expect("spawn")
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
});
