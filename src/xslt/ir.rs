//! The compiled stylesheet: what the executor runs. Nothing here refers back
//! to the stylesheet's XML tree.

use std::collections::HashMap;

use super::number_format::DecimalFormat;
use super::pattern::Pattern;
use crate::tree::{QName, Sym};
use crate::xpath::ast::{Expr, Name};

/// A namespace binding: prefix (None for the default namespace) and URI.
pub type Binding = (Option<Sym>, Sym);

#[derive(Debug, Default)]
pub struct Stylesheet {
    pub templates: Vec<Template>,
    pub named: HashMap<Name, usize>,
    /// Match rules per mode, for template selection.
    pub rules: HashMap<Option<Name>, Vec<Rule>>,
    /// Top-level variables and parameters, in an order where each one's
    /// dependencies come first.
    pub globals: Vec<Variable>,
    /// Names declared by a top-level xsl:variable in any module. A
    /// command-line parameter with one of these names is ignored, as libxslt
    /// ignores it.
    pub global_variable_names: std::collections::HashSet<Name>,
    pub output: Output,
    /// `xsl:strip-space` and `xsl:preserve-space` tests, in stylesheet order.
    pub space: Vec<SpaceRule>,
    pub attribute_sets: HashMap<Name, AttributeSet>,
    /// The unnamed `xsl:decimal-format` (libxslt's defaults, changed by any
    /// unnamed declarations), and the named ones.
    pub decimal_format: DecimalFormat,
    pub decimal_formats: HashMap<Name, DecimalFormat>,
    /// `xsl:key` definitions by name; several with one name add up.
    pub keys: HashMap<Name, Vec<KeyDef>>,
    /// Every definition as (name, index in `keys[name]`), in the order
    /// libxslt indexes them (xsltInitAllDocKeys): highest precedence
    /// first, then as declared.
    pub key_order: Vec<(Name, usize)>,
}

#[derive(Debug)]
pub struct KeyDef {
    pub matches: Pattern,
    pub use_: Expr,
    pub location: Location,
    /// Import precedence (a module number until assign_precedence).
    pub precedence: usize,
    /// Position among top-level declarations.
    pub order: usize,
}

#[derive(Debug)]
pub struct Template {
    pub matches: Option<Pattern>,
    pub name: Option<Name>,
    pub mode: Option<Name>,
    pub params: Vec<Variable>,
    pub body: Body,
    pub location: Location,
    /// Import precedence: higher wins. Each imported module's is lower than
    /// its importer's (a post-order walk of the import tree); an included
    /// file shares the precedence of the module that includes it.
    pub precedence: usize,
}

/// One alternative of a template's match pattern, with its priority.
#[derive(Debug, Clone)]
pub struct Rule {
    pub template: usize,
    pub alternative: usize,
    pub priority: f64,
    /// Position in the stylesheet: on equal priority the later rule wins.
    pub order: usize,
    /// The template's import precedence, which comes before priority.
    pub precedence: usize,
}

/// Where an instruction came from, for error messages like xsltproc's
/// "file style.xsl line 3 element call-template".
#[derive(Debug, Clone, Default)]
pub struct Location {
    pub file: std::rc::Rc<str>,
    pub line: u32,
    pub element: std::rc::Rc<str>,
    /// The element's base URI (xmlNodeGetBase), for `document()`.
    pub base: std::rc::Rc<str>,
    /// The namespaces in scope on the element, for QNames in strings
    /// (`element-available('xsl:if')` with a computed name).
    pub namespaces: std::rc::Rc<[Binding]>,
}

pub type Body = Vec<Instr>;

#[derive(Debug, Clone)]
pub enum Avt {
    Literal(String),
    Parts(Vec<AvtPart>),
}

#[derive(Debug, Clone)]
pub enum AvtPart {
    Literal(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum VarValue {
    Select(Expr),
    /// The content builds a result tree fragment.
    Body(Body),
    /// No select and no content: the empty string.
    Empty,
}

#[derive(Debug, Clone)]
pub struct Variable {
    pub name: Name,
    pub value: VarValue,
    pub location: Location,
}

#[derive(Debug, Clone)]
pub struct Sort {
    pub select: Expr,
    pub data_type: Option<Avt>,
    pub order: Option<Avt>,
    pub case_order: Option<Avt>,
}

#[derive(Debug, Clone)]
pub enum Instr {
    /// Literal text or `xsl:text`.
    Text(String),
    /// `xsl:text disable-output-escaping="yes"`.
    UnescapedText(String),
    LiteralElement {
        name: QName,
        /// The namespace declarations written on the element itself.
        namespaces: Vec<Binding>,
        /// The template's inherited namespaces, for an element directly in a
        /// template body. libxslt adds them after the element's own name.
        inherited: Vec<Binding>,
        attributes: Vec<(QName, Avt)>,
        attribute_sets: Vec<Name>,
        body: Body,
        location: Location,
    },
    ValueOf {
        select: Expr,
        /// `disable-output-escaping="yes"`
        unescaped: bool,
        location: Location,
    },
    ApplyTemplates {
        select: Option<Expr>,
        mode: Option<Name>,
        sorts: Vec<Sort>,
        params: Vec<Variable>,
        location: Location,
    },
    CallTemplate {
        name: Name,
        params: Vec<Variable>,
        location: Location,
    },
    ForEach {
        select: Expr,
        sorts: Vec<Sort>,
        body: Body,
        location: Location,
    },
    If {
        test: Expr,
        body: Body,
        location: Location,
    },
    Choose {
        /// Each xsl:when: its test, body and location.
        whens: Vec<(Expr, Body, Location)>,
        otherwise: Option<Body>,
    },
    Variable(Variable),
    Copy {
        attribute_sets: Vec<Name>,
        body: Body,
    },
    CopyOf {
        select: Expr,
        location: Location,
    },
    Element {
        name: Avt,
        namespace: Option<Avt>,
        /// The stylesheet's in-scope namespaces, for the name's prefix.
        scope: Vec<Binding>,
        attribute_sets: Vec<Name>,
        body: Body,
        location: Location,
    },
    Attribute {
        name: Avt,
        namespace: Option<Avt>,
        scope: Vec<Binding>,
        body: Body,
        location: Location,
    },
    Comment(Body),
    ProcessingInstruction {
        name: Avt,
        body: Body,
        location: Location,
    },
    Message {
        terminate: bool,
        body: Body,
    },
    /// An instruction the PoC doesn't support yet. Running it is an error.
    /// `xsl:number`.
    Number {
        /// `value`: number(expr) is formatted, instead of counting.
        value: Option<Expr>,
        level: NumberLevel,
        count: Option<Pattern>,
        from: Option<Pattern>,
        format: Avt,
        grouping: super::number::Grouping,
        location: Location,
    },
    /// `xsl:apply-imports`: the current node, in the current mode, with
    /// the rules of lower precedence than the current template rule's.
    ApplyImports {
        location: Location,
    },
    Unsupported {
        what: String,
        location: Location,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberLevel {
    Single,
    Multiple,
    Any,
    /// An unknown value: libxslt warns and writes nothing.
    Invalid,
}

#[derive(Debug, Clone)]
pub struct AttributeSet {
    pub uses: Vec<Name>,
    pub attributes: Vec<Instr>,
}

#[derive(Debug, Clone)]
pub struct SpaceRule {
    pub strip: bool,
    pub test: SpaceTest,
    pub order: usize,
    pub precedence: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpaceTest {
    Any,
    Namespace(Option<Sym>),
    Name(Name),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Method {
    #[default]
    Unspecified,
    Xml,
    Html,
    Text,
}

/// `xsl:output`. Later declarations override earlier ones attribute by attribute.
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub method: Method,
    pub version: Option<String>,
    pub encoding: Option<String>,
    pub omit_xml_declaration: Option<bool>,
    pub standalone: Option<bool>,
    pub doctype_public: Option<String>,
    pub doctype_system: Option<String>,
    pub indent: Option<bool>,
    pub media_type: Option<String>,
    pub cdata_section_elements: Vec<Name>,
}
