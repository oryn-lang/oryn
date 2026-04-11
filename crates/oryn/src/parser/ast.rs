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
        params: Vec<Param>,
        body: Spanned<Expression>,
        return_type: Option<TypeAnnotation>,
        is_pub: bool,
    },
    Return(Option<Spanned<Expression>>),
    ObjDef {
        name: String,
        fields: Vec<ObjField>,
        methods: Vec<ObjMethod>,
        /// Each `use` clause is a dotted path. A bare `use Health` is
        /// `vec!["Health"]`; a qualified `use combat.Health` is
        /// `vec!["combat", "Health"]`. The compiler resolves single-segment
        /// paths against the local obj table and multi-segment paths
        /// against imported module exports.
        uses: Vec<Vec<String>>,
        is_pub: bool,
    },
    /// `enum Name { Variant1, Variant2 { field: T, ... }, ... }` —
    /// a tagged-union (sum) type. Each variant is either nullary
    /// (a bare name) or carries named-field payloads with the same
    /// shape as obj fields. Top-level only; cannot be nested in
    /// functions or methods. See WARTS.md (enums section) and
    /// `examples/11_enums.on` for the user-facing form.
    ///
    /// The `is_error` flag is set when the declaration is prefixed
    /// with the `error` keyword (`error enum Foo { ... }` or
    /// `pub error enum Foo { ... }`). Values of an error enum are
    /// valid on the error side of any `error T` union — this is
    /// the mechanism that replaces the old string-backed
    /// `Error("msg")` builtin.
    EnumDef {
        name: String,
        variants: Vec<EnumVariant>,
        is_pub: bool,
        is_error: bool,
    },
    FieldAssignment {
        object: Spanned<Expression>,
        field: String,
        value: Spanned<Expression>,
    },
    /// `object[index] = value` — list element assignment.
    IndexAssignment {
        object: Spanned<Expression>,
        index: Spanned<Expression>,
        value: Spanned<Expression>,
    },
    Assignment {
        name: String,
        value: Spanned<Expression>,
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
    /// `test "name" { ... }` — a named test block at module level. The
    /// body is compiled like a zero-arity function; the runner invokes
    /// each test in isolation.
    Test {
        name: String,
        body: Spanned<Expression>,
    },
    /// `assert(expr)` — fail the enclosing test (or trap at runtime) if
    /// the condition evaluates to `false`. The condition must be boolean.
    Assert {
        condition: Spanned<Expression>,
    },
}

/// An expression node in the AST.
#[derive(Debug)]
pub enum Expression {
    /// The `nil` literal.
    Nil,
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
    /// `try expr` — propagate error from `!T`.
    Try(Box<Spanned<Expression>>),
    /// `!expr` — unwrap `!T`, trap on error.
    UnwrapError(Box<Spanned<Expression>>),
    /// `a orelse b` — nil coalescing.
    Coalesce {
        left: Box<Spanned<Expression>>,
        right: Box<Spanned<Expression>>,
    },
    /// `[a, b, c]` — a list literal. Must contain at least one element;
    /// empty literals have no context-free element type and are rejected
    /// at the compiler level with a clearer error than the parser could give.
    ListLiteral(Vec<Spanned<Expression>>),
    /// `{ key: value }` — a homogeneous map literal. Empty literals
    /// have no context-free key/value types and are reconciled against
    /// annotations by the compiler.
    MapLiteral(Vec<(Spanned<Expression>, Spanned<Expression>)>),
    /// `object[index]` — list or map indexing. Also parses on other receivers
    /// but the compiler rejects those with a type error.
    Index {
        object: Box<Spanned<Expression>>,
        index: Box<Spanned<Expression>>,
    },
    Block(Vec<Spanned<Statement>>),
    /// `if cond { body } else { else_body }` (or `elif` chains).
    ///
    /// Slice 5 W26 lift: `if` is now an expression. In expression
    /// position both `body` and `else_body` must produce the same
    /// type and the whole expression's value is the matched
    /// branch's value. In statement position (wrapped in
    /// [`Statement::Expression`]), the value is discarded by the
    /// usual expression-statement Pop. The `else_body` is `None`
    /// only when used in statement position; the compiler rejects
    /// the no-else form when the result is bound to a let or
    /// returned.
    If {
        condition: Box<Spanned<Expression>>,
        body: Box<Spanned<Expression>>,
        else_body: Option<Box<Spanned<Expression>>>,
    },
    /// `if let x = maybe_int { ... } else { ... }` — unwrap a
    /// nillable into the body branch's local `x`. Same expression /
    /// statement position rules as `If`.
    IfLet {
        name: String,
        value: Box<Spanned<Expression>>,
        body: Box<Spanned<Expression>>,
        else_body: Option<Box<Spanned<Expression>>>,
    },
    /// `match scrutinee { pattern => body, ... }` — pattern match on
    /// an enum value. The arms are tried in order; the first arm
    /// whose pattern matches the scrutinee fires. Match is *always*
    /// an expression — its value is the value of the matched arm's
    /// body. All arms must produce the same type. Used as a
    /// statement, the value is discarded via the existing
    /// expression-statement form.
    Match {
        scrutinee: Box<Spanned<Expression>>,
        arms: Vec<MatchArm>,
    },
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
    /// `T?` — a nillable type.
    Nillable(Box<TypeAnnotation>),
    /// `error T` (loose) or `error of E T` (precise) — an error union
    /// type. `error_enum` is `None` for the loose form (any error
    /// enum may appear on the error side) and `Some(path)` for the
    /// precise form where `path` is the dotted name of the specific
    /// error enum allowed on the error side.
    ErrorUnion {
        /// Dotted name of the error enum for the precise form
        /// (`error of math.errors.Fault int`), or `None` for the
        /// loose form (`error int`).
        error_enum: Option<Vec<String>>,
        inner: Box<TypeAnnotation>,
    },
    /// `[T]` — a homogeneous list whose element type is tracked statically
    /// but erased at runtime.
    List(Box<TypeAnnotation>),
    /// `{K: V}` — a homogeneous map whose key and value types are tracked
    /// statically but erased at runtime.
    Map(Box<TypeAnnotation>, Box<TypeAnnotation>),
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
///
/// Whether the method *mutates* `self` is determined by the `self`
/// parameter's `is_mut` flag, accessed via [`ObjMethod::is_mut`]. A
/// method declared `fn bump(mut self)` mutates self; a method declared
/// `fn read(self)` does not. Static methods (no `self` param) are never
/// mutating.
#[derive(Debug)]
pub struct ObjMethod {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Option<Spanned<Expression>>,
    pub return_type: Option<TypeAnnotation>,
    pub is_pub: bool,
    /// Byte range covering the method declaration (from `pub`/`fn`
    /// through the end of the body or signature). Used by the LSP to
    /// look up doc comments directly above the method.
    pub span: Span,
}

impl ObjMethod {
    /// `true` if this method declares its `self` parameter as
    /// `mut self`. Reads the `self` parameter's `is_mut` flag if
    /// present; static methods (no `self` param) always return
    /// `false`. This is the canonical "is this method mutating"
    /// check used by the compiler and the override sig comparison.
    pub fn is_mut(&self) -> bool {
        self.params
            .iter()
            .find(|p| p.name == "self")
            .is_some_and(|p| p.is_mut)
    }
}

/// A parameter in a function or method declaration. `is_mut` is true
/// when the parameter was declared `mut x: T` — it permits the function
/// body to mutate the parameter's fields, indexed elements, and call
/// mutating methods on it. Without `mut`, parameters are immutable in
/// Oryn (no opt-out at the call site). `self` parameters never carry
/// `is_mut` directly; whether `self` is mutable is decided by the
/// enclosing method's `is_mut` flag.
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub type_ann: Option<TypeAnnotation>,
    pub is_mut: bool,
}

impl Param {
    /// Convenience constructor for the common case of a parameter
    /// declared without `mut`. Used by the parser hot path.
    pub fn new(name: String, type_ann: Option<TypeAnnotation>) -> Self {
        Self {
            name,
            type_ann,
            is_mut: false,
        }
    }
}

/// A variant of an enum declaration. Variants are either nullary
/// (no payload, `fields` is empty) or carry **named** payload fields
/// with the same shape as obj fields. The reuse of `ObjField` is
/// deliberate: enum payloads parse, type-check, and store the same
/// way obj fields do, so we share the type to keep the two paths
/// in lockstep.
#[derive(Debug)]
pub struct EnumVariant {
    pub name: String,
    /// Empty for nullary variants. Otherwise the payload fields in
    /// declaration order. Each field carries its name, type, span,
    /// and `is_pub` (always `false` for now — per-variant payload
    /// visibility isn't a thing).
    pub fields: Vec<ObjField>,
    /// Byte range covering the variant declaration. Used for
    /// per-variant error reporting (e.g. duplicate variant names).
    pub span: Span,
}

/// A single arm of a `match` expression: `pattern => body`.
/// The body is any expression — single value, function call, block
/// expression, nested match. The arm's value is the body's value;
/// all arms in a match must produce the same type.
#[derive(Debug)]
pub struct MatchArm {
    pub pattern: Spanned<Pattern>,
    pub body: Spanned<Expression>,
    pub span: Span,
}

/// A pattern that appears on the left of `=>` in a match arm.
///
/// As of Slice 3, patterns are:
///   * `EnumName.Variant` — tag-only match, no payload binding.
///   * `EnumName.Variant { field, … }` — tag match plus payload
///     destructuring. Each binding is either shorthand (`field` —
///     bind a local with the same name as the field) or explicit
///     (`field: name` — bind a local under a different name). Both
///     forms can mix in the same brace block. Partial destructuring
///     is allowed: unlisted fields are simply not bound.
///   * `_` — wildcard, matches anything. Used as a catch-all in
///     non-exhaustive matches.
#[derive(Debug)]
pub enum Pattern {
    /// `EnumName.VariantName` — matches when the scrutinee is the
    /// named variant, regardless of payload contents. The compiler
    /// resolves the names against the enum table to verify the
    /// variant exists and to compute the discriminant for codegen.
    ///
    /// `bindings` is `None` for a tag-only match (no `{}` block at
    /// all). It is `Some(vec![..])` when the user wrote a brace
    /// block, even if that block is empty — the empty form
    /// `Variant { }` is preserved here so the compiler can
    /// produce a precise "remove the empty `{}`" error rather
    /// than silently treating it as tag-only.
    Variant {
        enum_name: String,
        variant_name: String,
        bindings: Option<Vec<PatternBinding>>,
    },
    /// `_` — matches anything. Used as a catch-all in non-exhaustive
    /// matches. Does not bind a name.
    Wildcard,
    /// `ok name` — matches the success side of an `error T` union
    /// scrutinee and binds the unwrapped success value to `name`
    /// in the arm body. Only valid when the match scrutinee has
    /// type `error T` (loose or precise); rejected in a plain
    /// enum match.
    Ok {
        /// The local name introduced in the arm body, bound to
        /// the unwrapped success value.
        name: String,
    },
}

/// A single payload field binding inside a `Variant { ... }` pattern.
/// Shorthand form `field` produces `PatternBinding { field: "field",
/// name: "field" }`; explicit form `field: name` produces
/// `PatternBinding { field: "field", name: "name" }`. The `field`
/// half is resolved against the variant's declared field names; the
/// `name` half is what the arm body sees as a local.
#[derive(Debug, Clone)]
pub struct PatternBinding {
    /// The payload field to extract from the matched variant. Must
    /// match a declared field on that variant.
    pub field: String,
    /// The local name introduced into the arm body. Equal to `field`
    /// for the shorthand form.
    pub name: String,
    /// The byte-offset span of the binding in source. Covers the
    /// shorthand `field` or the entire `field: name` pair.
    pub span: Span,
}

#[derive(Debug)]
pub enum StringPart {
    Literal(String),
    Interp(Spanned<Expression>),
}
