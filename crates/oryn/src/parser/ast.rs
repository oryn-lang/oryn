use std::ops::Range;

use chumsky::prelude::SimpleSpan;

/// Byte-offset span in the source.
pub type Span = Range<usize>;

/// An AST node paired with its source span.
#[derive(Debug)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: SimpleSpan) -> Self {
        Self {
            node,
            span: span.start..span.end,
        }
    }
}

/// A top-level statement in the AST.
#[derive(Debug)]
pub enum Statement {
    Let {
        name: String,
        value: Spanned<Expression>,
        type_ann: Option<TypeAnnotation>,
        is_pub: bool,
    },
    Val {
        name: String,
        value: Spanned<Expression>,
        type_ann: Option<TypeAnnotation>,
        is_pub: bool,
    },
    Function {
        name: String,
        params: Vec<(String, Option<TypeAnnotation>)>,
        body: Spanned<Expression>,
        return_type: Option<TypeAnnotation>,
        is_pub: bool,
    },
    Return(Option<Spanned<Expression>>),
    ObjDef {
        name: String,
        fields: Vec<ObjField>,
        methods: Vec<ObjMethod>,
        uses: Vec<String>,
        is_pub: bool,
    },
    FieldAssignment {
        object: Spanned<Expression>,
        field: String,
        value: Spanned<Expression>,
    },
    Assignment {
        name: String,
        value: Spanned<Expression>,
    },
    If {
        condition: Spanned<Expression>,
        body: Spanned<Expression>,
        else_body: Option<Spanned<Expression>>,
    },
    While {
        condition: Spanned<Expression>,
        body: Spanned<Expression>,
    },
    For {
        name: String,
        iterable: Spanned<Expression>,
        body: Spanned<Expression>,
    },
    /// `import foo.bar.baz` — load a module by dotted path. The path
    /// resolves to `<project root>/foo/bar/baz.on` and registers the
    /// module under the same dotted key in the compiler's module table.
    Import {
        path: Vec<String>,
    },
    Break,
    Continue,
    Expression(Spanned<Expression>),
}

/// An expression node in the AST.
#[derive(Debug)]
pub enum Expression {
    True,
    False,
    Float(f32),
    Int(i32),
    String(String),
    StringInterp(Vec<StringPart>),
    Ident(String),
    ObjLiteral {
        /// Dotted path to the type. A bare `Vec2` is `vec!["Vec2"]`,
        /// a qualified `math.Vec2` is `vec!["math", "Vec2"]`.
        type_name: Vec<String>,
        fields: Vec<(String, Spanned<Expression>)>,
    },
    FieldAccess {
        object: Box<Spanned<Expression>>,
        field: String,
    },
    MethodCall {
        object: Box<Spanned<Expression>>,
        method: String,
        args: Vec<Spanned<Expression>>,
    },
    BinaryOp {
        op: BinOp,
        left: Box<Spanned<Expression>>,
        right: Box<Spanned<Expression>>,
    },
    Range {
        start: Box<Spanned<Expression>>,
        end: Box<Spanned<Expression>>,
        inclusive: bool,
    },
    UnaryOp {
        op: UnaryOp,
        expr: Box<Spanned<Expression>>,
    },
    Call {
        name: String,
        args: Vec<Spanned<Expression>>,
    },
    Block(Vec<Spanned<Statement>>),
}

/// A binary operator.
#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Equals,
    NotEquals,
    LessThan,
    GreaterThan,
    LessThanEquals,
    GreaterThanEquals,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
}

/// A unary operator.
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Not,
    Negate,
}

#[derive(Debug, Clone)]
pub enum TypeAnnotation {
    /// Type name as a dotted path. A bare `Vec2` is `vec!["Vec2"]`,
    /// a qualified `math.Vec2` is `vec!["math", "Vec2"]`.
    Named(Vec<String>),
}

/// A field declared inside an `obj` body. The `is_pub` flag controls
/// whether code in other modules can read or write the field directly.
#[derive(Debug)]
pub struct ObjField {
    pub name: String,
    pub type_ann: TypeAnnotation,
    pub span: Span,
    pub is_pub: bool,
}

/// A method declared inside an `obj` body. `body` is `None` for required
/// signatures (declarations without a body) used by `use` composition.
/// `is_pub` controls cross-module visibility independent of the parent
/// object's `is_pub` flag.
#[derive(Debug)]
pub struct ObjMethod {
    pub name: String,
    pub params: Vec<(String, Option<TypeAnnotation>)>,
    pub body: Option<Spanned<Expression>>,
    pub return_type: Option<TypeAnnotation>,
    pub is_pub: bool,
}

#[derive(Debug)]
pub enum StringPart {
    Literal(String),
    Interp(Spanned<Expression>),
}
