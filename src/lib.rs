//! oxslt: a memory-safe replacement for the `xsltproc` command-line tool,
//! targeting XSLT 1.0. See CLAUDE.md for the design rules.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod cli;
pub mod dtd;
pub mod encoding;
pub mod parse;
pub mod serialize;
pub mod tree;
pub mod uri;
pub mod xinclude;
pub mod xpath;
pub mod xslt;

use std::ffi::OsString;
use std::io::Write;

/// Exit codes, matching xsltproc's documented codes.
pub mod exit {
    pub const SUCCESS: i32 = 0;
    pub const NO_ARGUMENT: i32 = 1;
    pub const TOO_MANY_PARAMETERS: i32 = 2;
    pub const UNKNOWN_OPTION: i32 = 3;
    pub const STYLESHEET_PARSE_FAILED: i32 = 4;
    pub const STYLESHEET_ERROR: i32 = 5;
    pub const DOCUMENT_ERROR: i32 = 6;
    pub const UNSUPPORTED_OUTPUT_METHOD: i32 = 7;
    pub const STRING_PARAM_HAS_BOTH_QUOTES: i32 = 8;
    pub const INTERNAL_ERROR: i32 = 9;
    pub const TERMINATED_BY_MESSAGE: i32 = 10;
    pub const OUTPUT_WRITE_FAILED: i32 = 11;
}

/// Runs oxslt with xsltproc-style arguments (without the program name) and
/// returns the process exit code.
pub fn run(args: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    cli::run(args, stdout, stderr, false)
}

/// [`run`] for a process that exits as soon as it returns, as the `oxslt`
/// binary does: the compiled stylesheet and names aren't freed, since the
/// operating system reclaims them faster. Freeing DocBook's compiled
/// stylesheet took about 3% of a run.
pub fn run_before_exit(args: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    cli::run(args, stdout, stderr, true)
}
