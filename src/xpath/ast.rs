//! The compiled form of an XPath expression. Names are resolved to interned
//! namespace URIs at compile time, so evaluation never sees prefixes.

use crate::tree::Sym;

/// An expanded name: namespace URI (None for no namespace) and local name.
pub type Name = (Option<Sym>, Sym);

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Compare(CompareOp, Box<Expr>, Box<Expr>),
    Arith(ArithOp, Box<Expr>, Box<Expr>),
    Negate(Box<Expr>),
    Union(Box<Expr>, Box<Expr>),
    Literal(String),
    Number(f64),
    Variable(Name),
    Call(Function, Vec<Expr>),
    Path(Path),
    /// A primary expression with predicates: `$v[1]`, `(a|b)[last()]`.
    Filter(Box<Expr>, Vec<Expr>),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Function {
    Core(CoreFn),
    /// Any other function: XSLT's (`current`, `generate-id`, …) or an
    /// extension. Resolved at evaluation time through [`super::Functions`].
    Other(Name),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CoreFn {
    Last,
    Position,
    Count,
    Id,
    LocalName,
    NamespaceUri,
    Name,
    String,
    Concat,
    StartsWith,
    Contains,
    SubstringBefore,
    SubstringAfter,
    Substring,
    StringLength,
    NormalizeSpace,
    Translate,
    Boolean,
    Not,
    True,
    False,
    Lang,
    Number,
    Sum,
    Floor,
    Ceiling,
    Round,
}

impl CoreFn {
    pub fn from_name(name: &str) -> Option<CoreFn> {
        use CoreFn::*;
        Some(match name {
            "last" => Last,
            "position" => Position,
            "count" => Count,
            "id" => Id,
            "local-name" => LocalName,
            "namespace-uri" => NamespaceUri,
            "name" => Name,
            "string" => String,
            "concat" => Concat,
            "starts-with" => StartsWith,
            "contains" => Contains,
            "substring-before" => SubstringBefore,
            "substring-after" => SubstringAfter,
            "substring" => Substring,
            "string-length" => StringLength,
            "normalize-space" => NormalizeSpace,
            "translate" => Translate,
            "boolean" => Boolean,
            "not" => Not,
            "true" => True,
            "false" => False,
            "lang" => Lang,
            "number" => Number,
            "sum" => Sum,
            "floor" => Floor,
            "ceiling" => Ceiling,
            "round" => Round,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub start: PathStart,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PathStart {
    /// `/…`: the root of the context node's document.
    Root,
    /// A relative path: the context node.
    Context,
    /// A filter expression followed by steps: `$v/a`, `id('x')//b`.
    Expr(Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub axis: Axis,
    pub test: NodeTest,
    pub predicates: Vec<Expr>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis {
    Ancestor,
    AncestorOrSelf,
    Attribute,
    Child,
    Descendant,
    DescendantOrSelf,
    Following,
    FollowingSibling,
    Namespace,
    Parent,
    Preceding,
    PrecedingSibling,
    SelfAxis,
}

impl Axis {
    pub fn from_name(name: &str) -> Option<Axis> {
        use Axis::*;
        Some(match name {
            "ancestor" => Ancestor,
            "ancestor-or-self" => AncestorOrSelf,
            "attribute" => Attribute,
            "child" => Child,
            "descendant" => Descendant,
            "descendant-or-self" => DescendantOrSelf,
            "following" => Following,
            "following-sibling" => FollowingSibling,
            "namespace" => Namespace,
            "parent" => Parent,
            "preceding" => Preceding,
            "preceding-sibling" => PrecedingSibling,
            "self" => SelfAxis,
            _ => return None,
        })
    }

    /// Reverse axes number their nodes from the context node outwards.
    pub fn is_reverse(self) -> bool {
        matches!(
            self,
            Axis::Ancestor | Axis::AncestorOrSelf | Axis::Preceding | Axis::PrecedingSibling
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeTest {
    /// `name` or `prefix:name`: the axis's principal node type with this name.
    Name(Name),
    /// `prefix:*`: the principal node type in this namespace.
    Namespace(Option<Sym>),
    /// `*`: any node of the principal node type.
    Any,
    /// `node()`
    Node,
    /// `text()`
    Text,
    /// `comment()`
    Comment,
    /// `processing-instruction()` or `processing-instruction('target')`
    Pi(Option<String>),
}
