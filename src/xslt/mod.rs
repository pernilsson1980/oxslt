//! XSLT 1.0: compiling stylesheets and running transforms.
//!
//! [`compile::compile`] turns a parsed stylesheet into the IR in [`ir`];
//! [`exec::Transform`] runs it. Result trees are built by [`output`].

pub mod compile;
pub mod dates;
pub mod exec;
pub mod exslt;
pub mod ir;
pub mod number;
pub mod number_format;
pub mod output;
pub mod pattern;
