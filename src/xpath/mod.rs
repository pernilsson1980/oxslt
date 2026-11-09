//! XPath 1.0: tokenizer, parser to an AST, and evaluator.
//!
//! [`compile`] turns an expression into an [`ast::Expr`], resolving namespace
//! prefixes and interning names. [`evaluate`] runs it against documents.
//! Results follow libxml2 2.9, the library behind xsltproc, wherever it
//! departs from the spec (number formatting, some axis edge cases).

pub mod ast;
mod eval;
mod lexer;
pub mod number;
mod parser;

pub use eval::{
    Empty, Env, Focus, Functions, Variables, boolean, evaluate, evaluate_nodes, is_attribute,
    is_position_independent, is_root, node_matches, round, sort_dedup, step_nodes, to_number,
    to_string,
};
pub use parser::{MAX_HEIGHT, MAX_NESTING, PrefixResolver};

use crate::tree::{NamePool, NodeId};

/// A node in one of the documents of a transform. Ordering is document order
/// within a document; documents are ordered by `doc`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NodeRef {
    /// Index into [`Env::docs`].
    pub doc: u32,
    pub node: NodeId,
    /// 0 for the node itself; `i` for the element's `i`-th namespace node,
    /// which sorts after the element and before its attributes.
    pub ns: u32,
}

impl NodeRef {
    pub fn new(doc: u32, node: NodeId) -> NodeRef {
        NodeRef { doc, node, ns: 0 }
    }
}

/// An XPath value. Node-sets are kept sorted in document order, without
/// duplicates.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    NodeSet(Vec<NodeRef>),
    Boolean(bool),
    Number(f64),
    String(String),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The expression doesn't parse (xsltproc: a stylesheet compilation error).
    Syntax,
    /// A value had the wrong type, such as a union of numbers.
    Type,
    /// Any other evaluation error: unknown function or variable, wrong arity.
    Runtime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XPathError {
    pub kind: ErrorKind,
    pub message: String,
}

impl XPathError {
    pub fn syntax(message: impl Into<String>) -> Self {
        XPathError {
            kind: ErrorKind::Syntax,
            message: message.into(),
        }
    }
    pub fn type_error(message: impl Into<String>) -> Self {
        XPathError {
            kind: ErrorKind::Type,
            message: message.into(),
        }
    }
    pub fn runtime(message: impl Into<String>) -> Self {
        XPathError {
            kind: ErrorKind::Runtime,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for XPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for XPathError {}

/// Compiles an XPath expression. `resolve` maps namespace prefixes to URIs.
pub fn compile(
    expr: &str,
    names: &mut NamePool,
    resolve: PrefixResolver,
) -> Result<ast::Expr, XPathError> {
    let tokens = lexer::tokenize(expr)?;
    parser::Parser::new(&tokens, names, resolve).parse()
}

#[cfg(test)]
mod tests;
