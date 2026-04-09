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
    },
    Val {
        name: String,
        value: Spanned<Expression>,
        type_ann: Option<TypeAnnotation>,
    },
    Function {
        name: String,
        params: Vec<(String, Option<TypeAnnotation>)>,
        body: Spanned<Expression>,
        return_type: Option<TypeAnnotation>,
    },
    Return(Option<Spanned<Expression>>),
    ObjDef {
        name: String,
        fields: Vec<(String, TypeAnnotation, Span)>,
        methods: Vec<ObjMethod>,
        uses: Vec<String>,
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
        type_name: String,
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
    Named(String),
}

#[derive(Debug)]
pub struct ObjMethod {
    pub name: String,
    pub params: Vec<(String, Option<TypeAnnotation>)>,
    pub body: Option<Spanned<Expression>>,
    pub return_type: Option<TypeAnnotation>,
}

#[derive(Debug)]
pub enum StringPart {
    Literal(String),
    Interp(Spanned<Expression>),
}
