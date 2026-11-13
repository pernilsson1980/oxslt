//! The library's error types implement `std::error::Error` (and so
//! `Display` and `Debug`), as the Rust API Guidelines expect.

fn is_error<E: std::error::Error + 'static>() {}

#[test]
fn error_types_implement_error() {
    is_error::<oxslt::tree::TooLarge>();
    is_error::<oxslt::parse::ParseError>();
    is_error::<oxslt::dtd::DtdError>();
    is_error::<oxslt::xpath::XPathError>();
    is_error::<oxslt::xslt::pattern::PatternError>();
    is_error::<oxslt::xslt::output::OutputError>();
    is_error::<oxslt::xslt::compile::CompileError>();
    is_error::<oxslt::xslt::compile::StylesheetError>();
    is_error::<oxslt::xslt::exec::ExecError>();
}

/// A stylesheet's compile errors print one per line.
#[test]
fn stylesheet_errors_display() {
    let e = oxslt::xslt::compile::StylesheetError::Parse("not well-formed".into());
    assert_eq!(e.to_string(), "not well-formed");
    let e = oxslt::xslt::exec::ExecError::Terminated("stopped".into());
    assert_eq!(e.to_string(), "stopped");
}
