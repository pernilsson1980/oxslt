//! Recursive-descent parser from tokens to [`Expr`].
//!
//! Two limits keep hostile input from overflowing the stack:
//! - nesting deeper than [`MAX_NESTING`] (`((((…`, `a[a[a[…`) is rejected on
//!   the way in, before the parser's own recursion gets deep;
//! - every parse function returns the height of the subtree it built, and a
//!   tree taller than [`MAX_HEIGHT`] (`1+1+1+…`, `----1`) is rejected while
//!   it's being built, so neither evaluation nor dropping the tree can
//!   recurse too deep.

use super::XPathError;
use super::ast::*;
use super::lexer::{Tok, Token};
use crate::tree::{NamePool, Sym};

/// Maximum expression tree height. Evaluation recurses once per level, and
/// a debug build uses over 2 KB of stack per level, so 250 keeps the tallest
/// tree well within a 2 MB thread. Real expressions are a few dozen levels at
/// most.
pub const MAX_HEIGHT: usize = 250;

/// Maximum nesting of expressions inside parentheses, predicates and
/// function arguments. Checked on the way in, because each level recurses
/// through a dozen parse functions before the height check on the way out
/// can run. Real stylesheets rarely nest past about 10.
pub const MAX_NESTING: usize = 100;

/// Resolves a namespace prefix to its URI, from the stylesheet's in-scope
/// namespaces. `None` means the prefix isn't declared.
pub type PrefixResolver<'a> = &'a dyn Fn(&str) -> Option<Sym>;

pub struct Parser<'a> {
    toks: &'a [Token<'a>],
    i: usize,
    names: &'a mut NamePool,
    resolve: PrefixResolver<'a>,
    nesting: usize,
}

type Parsed = Result<(Expr, usize), XPathError>;

/// Builds a binary-operator node from its two operands.
type MakeBinary = fn(Box<Expr>, Box<Expr>) -> Expr;

impl<'a> Parser<'a> {
    pub fn new(
        toks: &'a [Token<'a>],
        names: &'a mut NamePool,
        resolve: PrefixResolver<'a>,
    ) -> Self {
        Parser {
            toks,
            i: 0,
            names,
            resolve,
            nesting: 0,
        }
    }

    pub fn parse(mut self) -> Result<Expr, XPathError> {
        if self.toks.is_empty() {
            return Err(XPathError::syntax("Empty expression"));
        }
        let (e, _) = self.expr()?;
        if let Some(t) = self.toks.get(self.i) {
            return Err(XPathError::syntax(format!(
                "Unexpected token at offset {}",
                t.pos
            )));
        }
        Ok(e)
    }

    fn peek(&self) -> Option<&Tok<'a>> {
        self.toks.get(self.i).map(|t| &t.tok)
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.peek() == Some(tok) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: &Tok, what: &str) -> Result<(), XPathError> {
        if self.eat(tok) {
            Ok(())
        } else {
            Err(self.error(&format!("Expected {what}")))
        }
    }

    fn error(&self, msg: &str) -> XPathError {
        match self.toks.get(self.i) {
            Some(t) => XPathError::syntax(format!("{msg} at offset {}", t.pos)),
            None => XPathError::syntax(format!("{msg} at end of expression")),
        }
    }

    fn check(height: usize) -> Result<usize, XPathError> {
        if height > MAX_HEIGHT {
            Err(XPathError::syntax("Expression too deeply nested"))
        } else {
            Ok(height)
        }
    }

    fn name(&mut self, prefix: Option<&str>, local: &str) -> Result<Name, XPathError> {
        let ns = match prefix {
            None => None,
            Some(p) => Some(
                (self.resolve)(p)
                    .ok_or_else(|| XPathError::syntax(format!("Undefined namespace prefix {p}")))?,
            ),
        };
        Ok((ns, self.names.intern(local)))
    }

    fn expr(&mut self) -> Parsed {
        self.nesting += 1;
        if self.nesting > MAX_NESTING {
            return Err(XPathError::syntax("Expression too deeply nested"));
        }
        let result = self.or_expr();
        self.nesting -= 1;
        result
    }

    /// A left-associative chain of binary operators.
    fn binary(
        &mut self,
        next: fn(&mut Self) -> Parsed,
        op: fn(&Tok) -> Option<MakeBinary>,
    ) -> Parsed {
        let (mut left, mut height) = next(self)?;
        while let Some(make) = self.peek().and_then(op) {
            self.i += 1;
            let (right, rh) = next(self)?;
            height = Self::check(height.max(rh) + 1)?;
            left = make(Box::new(left), Box::new(right));
        }
        Ok((left, height))
    }

    fn or_expr(&mut self) -> Parsed {
        self.binary_from(1)
    }

    /// Binary operators by precedence climbing: or (1), and (2), equality
    /// (3), relational (4), additive (5), multiplicative (6), all left-
    /// associative over unary expressions. One loop instead of a function
    /// per level, which a simple name went through six times.
    fn binary_from(&mut self, min: u8) -> Parsed {
        let (mut left, mut height) = self.unary_expr()?;
        while let Some((prec, make)) = self.peek().and_then(binary_op) {
            if prec < min {
                break;
            }
            self.i += 1;
            let (right, rh) = self.binary_from(prec + 1)?;
            height = Self::check(height.max(rh) + 1)?;
            left = make(Box::new(left), Box::new(right));
        }
        Ok((left, height))
    }

    fn unary_expr(&mut self) -> Parsed {
        let mut negations = 0usize;
        while self.eat(&Tok::Minus) {
            negations += 1;
            Self::check(negations)?;
        }
        let (mut e, mut height) = self.union_expr()?;
        for _ in 0..negations {
            height = Self::check(height + 1)?;
            e = Expr::Negate(Box::new(e));
        }
        Ok((e, height))
    }

    fn union_expr(&mut self) -> Parsed {
        self.binary(Self::path_expr, |t| {
            (*t == Tok::Pipe).then_some(Expr::Union as MakeBinary)
        })
    }

    fn starts_filter(&self) -> bool {
        matches!(
            self.peek(),
            Some(
                Tok::Variable(..)
                    | Tok::LParen
                    | Tok::Literal(_)
                    | Tok::Number(_)
                    | Tok::FunctionName(..)
            )
        )
    }

    fn starts_step(&self) -> bool {
        matches!(
            self.peek(),
            Some(
                Tok::Name(..)
                    | Tok::Star
                    | Tok::PrefixStar(_)
                    | Tok::NodeType(_)
                    | Tok::AxisName(_)
                    | Tok::At
                    | Tok::Dot
                    | Tok::DotDot
            )
        )
    }

    fn path_expr(&mut self) -> Parsed {
        if self.starts_filter() {
            let (primary, mut height) = self.primary_expr()?;
            let (predicates, ph) = self.predicates()?;
            height = height.max(ph);
            let filter = if predicates.is_empty() {
                primary
            } else {
                height = Self::check(height + 1)?;
                Expr::Filter(Box::new(primary), predicates)
            };
            if !matches!(self.peek(), Some(Tok::Slash | Tok::SlashSlash)) {
                return Ok((filter, height));
            }
            let mut steps = Vec::new();
            let sh = self.relative_steps(&mut steps)?;
            let height = Self::check(height.max(sh) + 1)?;
            return Ok((
                Expr::Path(Path {
                    start: PathStart::Expr(Box::new(filter)),
                    steps,
                }),
                height,
            ));
        }

        let mut steps = Vec::new();
        let start = match self.peek() {
            Some(Tok::Slash) => {
                self.i += 1;
                if !self.starts_step() {
                    return Ok((
                        Expr::Path(Path {
                            start: PathStart::Root,
                            steps,
                        }),
                        1,
                    ));
                }
                let h = self.steps(&mut steps)?;
                return Ok((
                    Expr::Path(Path {
                        start: PathStart::Root,
                        steps,
                    }),
                    Self::check(h + 1)?,
                ));
            }
            Some(Tok::SlashSlash) => {
                self.i += 1;
                steps.push(descendant_or_self());
                PathStart::Root
            }
            _ => PathStart::Context,
        };
        let h = self.steps(&mut steps)?;
        Ok((Expr::Path(Path { start, steps }), Self::check(h + 1)?))
    }

    /// `/step` and `//step` repeated: the rest of a path after its first part.
    fn relative_steps(&mut self, steps: &mut Vec<Step>) -> Result<usize, XPathError> {
        let mut height = 0;
        while let Some(sep) = self.peek().cloned() {
            match sep {
                Tok::Slash => self.i += 1,
                Tok::SlashSlash => {
                    self.i += 1;
                    steps.push(descendant_or_self());
                }
                _ => break,
            }
            height = height.max(self.step(steps)?);
        }
        Ok(height)
    }

    /// A relative location path: step (('/' | '//') step)*.
    fn steps(&mut self, steps: &mut Vec<Step>) -> Result<usize, XPathError> {
        let mut height = self.step(steps)?;
        height = height.max(self.relative_steps(steps)?);
        Ok(height)
    }

    fn step(&mut self, steps: &mut Vec<Step>) -> Result<usize, XPathError> {
        match self.peek() {
            Some(Tok::Dot) => {
                self.i += 1;
                steps.push(Step {
                    axis: Axis::SelfAxis,
                    test: NodeTest::Node,
                    predicates: vec![],
                });
                return Ok(0);
            }
            Some(Tok::DotDot) => {
                self.i += 1;
                steps.push(Step {
                    axis: Axis::Parent,
                    test: NodeTest::Node,
                    predicates: vec![],
                });
                return Ok(0);
            }
            _ => {}
        }
        let axis = match self.peek().cloned() {
            Some(Tok::AxisName(name)) => {
                self.i += 1;
                self.expect(&Tok::ColonColon, "'::'")?;
                Axis::from_name(name)
                    .ok_or_else(|| XPathError::syntax(format!("Unknown axis {name}")))?
            }
            Some(Tok::At) => {
                self.i += 1;
                Axis::Attribute
            }
            _ => Axis::Child,
        };
        let test = self.node_test()?;
        let (predicates, height) = self.predicates()?;
        steps.push(Step {
            axis,
            test,
            predicates,
        });
        Ok(height)
    }

    fn node_test(&mut self) -> Result<NodeTest, XPathError> {
        let tok = self
            .peek()
            .cloned()
            .ok_or_else(|| self.error("Expected a node test"))?;
        self.i += 1;
        match tok {
            Tok::Star => Ok(NodeTest::Any),
            Tok::PrefixStar(p) => {
                let ns = (self.resolve)(p)
                    .ok_or_else(|| XPathError::syntax(format!("Undefined namespace prefix {p}")))?;
                Ok(NodeTest::Namespace(Some(ns)))
            }
            Tok::Name(prefix, local) => Ok(NodeTest::Name(self.name(prefix, local)?)),
            Tok::NodeType(kind) => {
                self.expect(&Tok::LParen, "'('")?;
                let test = match kind {
                    "node" => NodeTest::Node,
                    "text" => NodeTest::Text,
                    "comment" => NodeTest::Comment,
                    _ => match self.peek().cloned() {
                        Some(Tok::Literal(target)) => {
                            self.i += 1;
                            NodeTest::Pi(Some(target.to_string()))
                        }
                        _ => NodeTest::Pi(None),
                    },
                };
                self.expect(&Tok::RParen, "')'")?;
                Ok(test)
            }
            _ => {
                self.i -= 1;
                Err(self.error("Expected a node test"))
            }
        }
    }

    fn predicates(&mut self) -> Result<(Vec<Expr>, usize), XPathError> {
        let mut preds = Vec::new();
        let mut height = 0;
        while self.eat(&Tok::LBracket) {
            let (e, h) = self.expr()?;
            self.expect(&Tok::RBracket, "']'")?;
            height = height.max(Self::check(h + 1)?);
            preds.push(e);
        }
        Ok((preds, height))
    }

    fn primary_expr(&mut self) -> Parsed {
        let tok = self
            .peek()
            .cloned()
            .ok_or_else(|| self.error("Expected an expression"))?;
        self.i += 1;
        match tok {
            Tok::Variable(prefix, local) => Ok((Expr::Variable(self.name(prefix, local)?), 1)),
            Tok::Literal(s) => Ok((Expr::Literal(s.to_string()), 1)),
            Tok::Number(n) => Ok((Expr::Number(n), 1)),
            Tok::LParen => {
                let (e, h) = self.expr()?;
                self.expect(&Tok::RParen, "')'")?;
                Ok((e, Self::check(h + 1)?))
            }
            Tok::FunctionName(prefix, local) => {
                let function = match (prefix, CoreFn::from_name(local)) {
                    (None, Some(core)) => Function::Core(core),
                    _ => Function::Other(self.name(prefix, local)?),
                };
                self.expect(&Tok::LParen, "'('")?;
                let mut args = Vec::new();
                let mut height = 0;
                if !self.eat(&Tok::RParen) {
                    loop {
                        let (arg, h) = self.expr()?;
                        height = height.max(h);
                        args.push(arg);
                        if self.eat(&Tok::RParen) {
                            break;
                        }
                        self.expect(&Tok::Comma, "',' or ')'")?;
                    }
                }
                Ok((Expr::Call(function, args), Self::check(height + 1)?))
            }
            _ => {
                self.i -= 1;
                Err(self.error("Expected an expression"))
            }
        }
    }
}

/// A binary operator's precedence and constructor (see `binary_from`).
fn binary_op(t: &Tok) -> Option<(u8, MakeBinary)> {
    Some(match t {
        Tok::Or => (1, Expr::Or as MakeBinary),
        Tok::And => (2, Expr::And as MakeBinary),
        Tok::Eq => (3, |a, b| Expr::Compare(CompareOp::Eq, a, b)),
        Tok::Ne => (3, |a, b| Expr::Compare(CompareOp::Ne, a, b)),
        Tok::Lt => (4, |a, b| Expr::Compare(CompareOp::Lt, a, b)),
        Tok::Le => (4, |a, b| Expr::Compare(CompareOp::Le, a, b)),
        Tok::Gt => (4, |a, b| Expr::Compare(CompareOp::Gt, a, b)),
        Tok::Ge => (4, |a, b| Expr::Compare(CompareOp::Ge, a, b)),
        Tok::Plus => (5, |a, b| Expr::Arith(ArithOp::Add, a, b)),
        Tok::Minus => (5, |a, b| Expr::Arith(ArithOp::Sub, a, b)),
        Tok::Multiply => (6, |a, b| Expr::Arith(ArithOp::Mul, a, b)),
        Tok::Div => (6, |a, b| Expr::Arith(ArithOp::Div, a, b)),
        Tok::Mod => (6, |a, b| Expr::Arith(ArithOp::Mod, a, b)),
        _ => return None,
    })
}

fn descendant_or_self() -> Step {
    Step {
        axis: Axis::DescendantOrSelf,
        test: NodeTest::Node,
        predicates: vec![],
    }
}
