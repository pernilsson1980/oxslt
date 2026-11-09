//! XSLT patterns: compiled from XPath syntax, matched right to left.
//!
//! A pattern is a union of location path patterns. Each alternative is a list
//! of steps (child or attribute axis), separated by `/` or `//`, optionally
//! anchored at the root or at an `id()`/`key()` call.

use crate::xpath::ast::{Axis, Expr, NodeTest, Path, PathStart, Step};

#[derive(Debug, Clone)]
pub struct Pattern {
    pub alternatives: Vec<Alternative>,
}

#[derive(Debug, Clone)]
pub struct Alternative {
    pub anchor: Anchor,
    /// Left to right.
    pub steps: Vec<PatternStep>,
    pub default_priority: f64,
    /// libxslt can't match this alternative, so it never matches. See
    /// `never_matches_in_libxslt`.
    pub never: bool,
}

#[derive(Debug, Clone)]
pub enum Anchor {
    /// A relative pattern like `a/b`.
    None,
    /// `/…`
    Root,
    /// `id('x')/…` or `key('k', 'v')/…`
    Call(Expr),
}

#[derive(Debug, Clone)]
pub struct PatternStep {
    /// `//` before this step: any ancestor may match what comes before.
    pub descendant: bool,
    pub axis: Axis,
    pub test: NodeTest,
    pub predicates: Vec<Expr>,
}

#[derive(Debug)]
pub struct PatternError(pub String);

/// Converts a compiled XPath expression into a pattern, rejecting anything
/// patterns don't allow.
/// `allow_variables`: xsl:number's count and from patterns may use
/// variables; template and key patterns may not (libxslt compiles those
/// with XML_XPATH_NOVAR).
pub fn from_expr(expr: Expr, allow_variables: bool) -> Result<Pattern, PatternError> {
    // Variables are forbidden in template and key patterns (xsltproc:
    // "Forbidden variable").
    if !allow_variables && contains_variable(&expr) {
        return Err(PatternError("Forbidden variable in pattern".into()));
    }
    let mut paths = Vec::new();
    flatten_union(expr, &mut paths)?;
    let alternatives = paths
        .into_iter()
        .map(alternative)
        .collect::<Result<_, _>>()?;
    Ok(Pattern { alternatives })
}

fn flatten_union(expr: Expr, out: &mut Vec<Expr>) -> Result<(), PatternError> {
    match expr {
        Expr::Union(a, b) => {
            flatten_union(*a, out)?;
            flatten_union(*b, out)
        }
        e => {
            out.push(e);
            Ok(())
        }
    }
}

pub fn contains_variable(e: &Expr) -> bool {
    match e {
        Expr::Variable(_) => true,
        Expr::Or(a, b)
        | Expr::And(a, b)
        | Expr::Compare(_, a, b)
        | Expr::Arith(_, a, b)
        | Expr::Union(a, b) => contains_variable(a) || contains_variable(b),
        Expr::Negate(a) => contains_variable(a),
        Expr::Call(_, args) => args.iter().any(contains_variable),
        Expr::Filter(p, preds) => contains_variable(p) || preds.iter().any(contains_variable),
        Expr::Path(p) => {
            matches!(&p.start, PathStart::Expr(e) if contains_variable(e))
                || p.steps
                    .iter()
                    .any(|s| s.predicates.iter().any(contains_variable))
        }
        Expr::Literal(_) | Expr::Number(_) => false,
    }
}

fn is_id_or_key(e: &Expr) -> bool {
    use crate::xpath::ast::{CoreFn, Function};
    match e {
        Expr::Call(Function::Core(CoreFn::Id), args) => {
            args.len() == 1 && matches!(args[0], Expr::Literal(_))
        }
        // key() is an XSLT function; its name has no namespace.
        Expr::Call(Function::Other((None, _)), args) => {
            args.len() == 2 && matches!(args[0], Expr::Literal(_))
        }
        _ => false,
    }
}

fn alternative(expr: Expr) -> Result<Alternative, PatternError> {
    let (anchor, steps) = match expr {
        Expr::Path(Path { start, steps }) => match start {
            PathStart::Root => (Anchor::Root, steps),
            PathStart::Context => (Anchor::None, steps),
            PathStart::Expr(e) if is_id_or_key(&e) => (Anchor::Call(*e), steps),
            PathStart::Expr(_) => return Err(PatternError("Invalid pattern".into())),
        },
        e if is_id_or_key(&e) => (Anchor::Call(e), Vec::new()),
        _ => return Err(PatternError("Invalid pattern".into())),
    };
    let mut out = Vec::new();
    let mut descendant = false;
    for Step {
        axis,
        test,
        predicates,
    } in steps
    {
        if axis == Axis::DescendantOrSelf && test == NodeTest::Node && predicates.is_empty() {
            if descendant {
                return Err(PatternError("Invalid pattern".into()));
            }
            descendant = true;
            continue;
        }
        if !matches!(axis, Axis::Child | Axis::Attribute) {
            return Err(PatternError(
                "Invalid pattern: only the child and attribute axes are allowed".into(),
            ));
        }
        out.push(PatternStep {
            descendant,
            axis,
            test,
            predicates,
        });
        descendant = false;
    }
    if descendant {
        return Err(PatternError("Invalid pattern".into()));
    }
    let default_priority = default_priority(&anchor, &out);
    let never = never_matches_in_libxslt(&anchor, &out);
    Ok(Alternative {
        anchor,
        steps: out,
        default_priority,
        never,
    })
}

/// libxslt matches a pattern "directly" when a step has two predicates, or
/// a predicate on a step that isn't an element name or `*`
/// (xsltCompilePatternInternal sets `direct`). It then evaluates "//" plus
/// the pattern text as XPath and looks for the node in the result. With an
/// id() or key() anchor that text, "//key(…)…", isn't valid XPath: libxslt
/// reports an XPath error and the pattern doesn't match.
fn never_matches_in_libxslt(anchor: &Anchor, steps: &[PatternStep]) -> bool {
    use crate::xpath::ast::Axis;
    matches!(anchor, Anchor::Call(_))
        && steps.iter().any(|s| {
            s.predicates.len() > 1
                || (!s.predicates.is_empty()
                    && (s.axis == Axis::Attribute
                        || !matches!(s.test, NodeTest::Name(_) | NodeTest::Any)))
        })
}

/// XSLT 1.0 section 5.5.
fn default_priority(anchor: &Anchor, steps: &[PatternStep]) -> f64 {
    if !matches!(anchor, Anchor::None) || steps.len() != 1 {
        return 0.5;
    }
    let step = &steps[0];
    if step.descendant || !step.predicates.is_empty() {
        return 0.5;
    }
    match &step.test {
        NodeTest::Name(_) | NodeTest::Pi(Some(_)) => 0.0,
        NodeTest::Namespace(_) => -0.25,
        NodeTest::Any
        | NodeTest::Node
        | NodeTest::Text
        | NodeTest::Comment
        | NodeTest::Pi(None) => -0.5,
    }
}
