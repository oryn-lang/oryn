use chumsky::IterParser as _;
use chumsky::Parser;
use chumsky::input::{Input as _, MappedInput};
use chumsky::prelude::{Rich, SimpleSpan, choice, extra, just, recursive, select};
use chumsky::recursive::Recursive;

use crate::errors::OrynError;
use crate::lexer::{self, Token};

use super::ast::*;

// Chumsky needs tokens paired with their source spans.
type TokenSpanned = (Token, SimpleSpan);
// The input type chumsky operates on. A slice of `TokenSpanned` tokens.
type TokenInput<'src> = MappedInput<'src, Token, SimpleSpan, &'src [TokenSpanned]>;

/// What kind of suffix follows a postfix step. Built once per postfix
/// match; the foldl callback decides whether to construct a FieldAccess,
/// MethodCall, ObjLiteral, or Index expression.
enum PostfixSuffix {
    Field(String),
    Call(String, Vec<Spanned<Expression>>),
    ObjLit(String, Vec<(String, Spanned<Expression>)>),
    Index(Spanned<Expression>),
}

/// Walk a chain of `Expression::FieldAccess` rooted in `Expression::Ident`
/// and append each segment name to `out` in source order. Used to recover
/// the dotted type path when promoting a `.field` chain to an ObjLiteral.
fn collect_path(expr: &Expression, out: &mut Vec<String>) {
    match expr {
        Expression::Ident(name) => out.push(name.clone()),
        Expression::FieldAccess { object, field } => {
            collect_path(&object.node, out);
            out.push(field.clone());
        }
        _ => {}
    }
}

/// Parses a token stream into an AST. Returns the statements and any
/// [`OrynError::Parser`] errors. Partial output is returned even when
/// there are errors.
///
/// ```
/// let (tokens, _) = oryn::lex("let x = 5");
/// let (ast, errors) = oryn::parse(tokens);
///
/// assert!(errors.is_empty());
/// assert_eq!(ast.len(), 1);
/// ```
pub fn parse(
    tokens: Vec<(Token, std::ops::Range<usize>)>,
) -> (Vec<Spanned<Statement>>, Vec<OrynError>) {
    // Convert lexer spans (Range<usize>) into chumsky's SimpleSpan type.
    let tokens: Vec<TokenSpanned> = tokens
        .into_iter()
        .map(|(t, s)| (t, SimpleSpan::from(s)))
        .collect();

    // Chumsky needs a span representing "end of input" for error reporting.
    let end = tokens.last().map(|(_, s)| s.end).unwrap_or(0);
    let end = SimpleSpan::from(end..end);

    // split_token_span gives chumsky a stream it can pull (token, span) pairs from.
    let input = tokens.as_slice().split_token_span(end);
    // into_output_errors returns both partial output and collected errors,
    // rather than failing on the first error.
    let (output, errors) = program().parse(input).into_output_errors();

    (
        output.unwrap_or_default(),
        errors
            .into_iter()
            .map(|e| OrynError::Parser {
                span: e.span().start..e.span().end,
                message: format!("{e}"),
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

// Atoms are the smallest, indivisible expressions: literals, identifiers,
// function calls, object literals, and parenthesized sub-expressions.
//
// `block` is the statement-block parser (`{ stmt; stmt; ... }`),
// passed in so this function can build `if`-as-expression and
// `if let`-as-expression primaries that take block bodies. The
// block parser depends on `stmt` which depends on `expr`, so the
// caller breaks the mutual recursion via `Recursive::declare()`.
fn atom<'src>(
    expr: impl Parser<
        'src,
        TokenInput<'src>,
        Spanned<Expression>,
        extra::Err<Rich<'src, Token, SimpleSpan>>,
    > + Clone
    + 'src,
    block: impl Parser<
        'src,
        TokenInput<'src>,
        Spanned<Expression>,
        extra::Err<Rich<'src, Token, SimpleSpan>>,
    > + Clone
    + 'src,
) -> impl Parser<'src, TokenInput<'src>, Spanned<Expression>, extra::Err<Rich<'src, Token, SimpleSpan>>>
+ Clone {
    let bool_lit = select! { Token::True => Expression::True, Token::False => Expression::False };
    let nil_lit = just(Token::Nil).map(|_| Expression::Nil);
    let float = select! { Token::Float(n) => Expression::Float(n) };
    let int = select! { Token::Int(n) => Expression::Int(n) };
    let string = select! { Token::String(s) => s }.map_with(|s, extra| {
        let span: SimpleSpan = extra.span();
        // +1 for the opening quote character stripped by the lexer.
        parse_string_content(s, span.start + 1)
    });

    // Object literal field: "name: expr"
    let obj_field_value = select! { Token::Ident(name) => name }
        .then_ignore(just(Token::Colon))
        .then(expr.clone());

    // Newlines inside object literal braces (zero or more).
    let nl = just(Token::Newline).repeated();

    // An identifier optionally followed by (args) or { fields }.
    // Ident + (args) = function call, Ident + { fields } = object literal,
    // bare Ident = variable reference.
    let ident_or_call = select! { Token::Ident(name) => name }
        .then(
            expr.clone()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LeftParen), just(Token::RightParen))
                .or_not(),
        )
        .then(
            obj_field_value
                .separated_by(just(Token::Comma).then(nl.clone()))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LeftCurly).then(nl.clone()),
                    nl.clone().then(just(Token::RightCurly)),
                )
                .or_not(),
        )
        .map(
            |((name, call_args), obj_fields)| match (call_args, obj_fields) {
                (Some(args), _) => Expression::Call { name, args },
                (_, Some(fields)) => Expression::ObjLiteral {
                    type_name: vec![name],
                    fields,
                },
                _ => Expression::Ident(name),
            },
        );

    let paren = expr
        .clone()
        .delimited_by(just(Token::LeftParen), just(Token::RightParen))
        .map(|spanned| spanned.node);

    // [a, b, c] — a list literal. Trailing commas are allowed for
    // multi-line literals. The compiler rejects empty literals because
    // an empty `[]` has no context-free element type.
    let list_literal = expr
        .clone()
        .separated_by(just(Token::Comma).then(nl.clone()))
        .allow_trailing()
        .collect::<Vec<Spanned<Expression>>>()
        .delimited_by(
            just(Token::LeftBracket).then(nl.clone()),
            nl.clone().then(just(Token::RightBracket)),
        )
        .map(Expression::ListLiteral);

    // { key: value, ... } — a map literal. Empty `{}` is allowed; the
    // compiler reconciles it against a declared `{K: V}` annotation.
    let map_entry = expr
        .clone()
        .then_ignore(just(Token::Colon))
        .then(expr.clone());
    let map_literal = map_entry
        .separated_by(just(Token::Comma).then(nl.clone()))
        .allow_trailing()
        .collect::<Vec<(Spanned<Expression>, Spanned<Expression>)>>()
        .delimited_by(
            just(Token::LeftCurly).then(nl.clone()),
            nl.clone().then(just(Token::RightCurly)),
        )
        .map(Expression::MapLiteral);

    // `match scrutinee { pattern => body, ... }` — pattern match
    // on an enum value. The match keyword introduces an
    // expression (Slice 1 makes match the first expression-form
    // control flow in Oryn; if/while remain statements — see
    // WARTS.md W26).
    //
    // Patterns (Slice 3):
    //   * `EnumName.Variant`                   — tag-only match
    //   * `EnumName.Variant { field, ... }`    — tag + payload bindings
    //   * `_`                                  — wildcard
    //
    // Each binding inside the brace block is either shorthand
    // (`field` — bind a local with the same name as the payload
    // field) or explicit (`field: name` — bind under a different
    // name). Both forms can mix freely. Partial destructuring is
    // allowed: unlisted fields are simply not bound. The empty
    // form `Variant { }` is rejected at compile time as
    // category-confused with a nullary variant pattern.
    let pattern = {
        let wildcard = select! { Token::Ident(name) if name == "_" => () }.map_with(|_, extra| {
            let s: SimpleSpan = extra.span();
            Spanned::new(Pattern::Wildcard, s)
        });

        // A single binding inside `{ ... }`. Try the explicit
        // `field: name` form first; fall back to shorthand `field`.
        let binding = select! { Token::Ident(name) => name }
            .then(
                just(Token::Colon)
                    .ignore_then(select! { Token::Ident(name) => name })
                    .or_not(),
            )
            .map_with(|(field, rename), extra| {
                let s: SimpleSpan = extra.span();
                let name = rename.unwrap_or_else(|| field.clone());
                PatternBinding {
                    field,
                    name,
                    span: s.start..s.end,
                }
            });

        // The full payload binding block: `{ binding, binding, ... }`.
        // Trailing comma allowed; whitespace/newlines tolerated
        // anywhere inside.
        let bindings_block = just(Token::LeftCurly)
            .ignore_then(nl.clone().or_not())
            .ignore_then(
                binding
                    .separated_by(just(Token::Comma).then(nl.clone().or_not()))
                    .allow_trailing()
                    .collect::<Vec<PatternBinding>>(),
            )
            .then_ignore(nl.clone().or_not())
            .then_ignore(just(Token::RightCurly));

        let variant_path = select! { Token::Ident(name) => name }
            .then_ignore(just(Token::Dot))
            .then(select! { Token::Ident(name) => name })
            .then(bindings_block.or_not())
            .map_with(|((enum_name, variant_name), bindings), extra| {
                let s: SimpleSpan = extra.span();
                Spanned::new(
                    Pattern::Variant {
                        enum_name,
                        variant_name,
                        bindings,
                    },
                    s,
                )
            });
        // `ok <name>` — success-side pattern for a match arm over an
        // error union scrutinee. The bound name is introduced into
        // the arm body as an immutable local holding the unwrapped
        // success value. Compiler rejects this pattern in plain-enum
        // matches (see `compile_match_expression`).
        let ok_pattern = just(Token::Ok)
            .ignore_then(select! { Token::Ident(name) => name })
            .map_with(|name, extra| {
                let s: SimpleSpan = extra.span();
                Spanned::new(Pattern::Ok { name }, s)
            });
        choice((wildcard, ok_pattern, variant_path))
    };

    let match_arm = pattern
        .then_ignore(just(Token::FatArrow))
        .then(expr.clone())
        .map_with(|(pattern, body), extra| {
            let s: SimpleSpan = extra.span();
            MatchArm {
                pattern,
                body,
                span: s.start..s.end,
            }
        });
    // Arm separator: comma or newline, same as obj fields and
    // enum variants. Trailing separator allowed.
    let arm_sep = just(Token::Comma)
        .then(nl.clone().or_not())
        .ignored()
        .or(nl.clone().at_least(1).ignored());

    let match_expr = just(Token::Match)
        .ignore_then(expr.clone())
        .then(
            match_arm
                .separated_by(arm_sep)
                .allow_trailing()
                .collect::<Vec<MatchArm>>()
                .delimited_by(
                    just(Token::LeftCurly).then(nl.clone()),
                    nl.clone().then(just(Token::RightCurly)),
                ),
        )
        .map(|(scrutinee, arms)| Expression::Match {
            scrutinee: Box::new(scrutinee),
            arms,
        });

    // `if cond { body }` / `if cond { body } else { else_body }` /
    // `if cond { body } elif cond2 { body2 } else { ... }` —
    // Slice 5 W26: `if` is now an expression. Used in expression
    // position, both branches must produce the same type and the
    // whole expression's value is the matched branch's value. In
    // statement position the value is discarded by the wrapping
    // `Statement::Expression` Pop.
    //
    // The recursive inner closure handles `elif` chains by
    // desugaring each `elif` into an `else` block containing a
    // single nested `if` expression statement.
    let if_expr = just(Token::If)
        .ignore_then(recursive(|if_body| {
            let else_branch = just(Token::Else)
                .ignore_then(block.clone())
                .or(just(Token::Elif).ignore_then(if_body).map_with(
                    |elif_expr: Spanned<Expression>, extra| {
                        // Wrap the elif's `if` expression in a single-
                        // statement block so it slots into the else
                        // position uniformly.
                        Spanned::new(
                            Expression::Block(vec![Spanned::new(
                                Statement::Expression(elif_expr),
                                extra.span(),
                            )]),
                            extra.span(),
                        )
                    },
                ));

            expr.clone()
                .then(block.clone())
                .then(else_branch.or_not())
                .map_with(|((condition, body), else_body), extra| {
                    Spanned::new(
                        Expression::If {
                            condition: Box::new(condition),
                            body: Box::new(body),
                            else_body: else_body.map(Box::new),
                        },
                        extra.span(),
                    )
                })
        }))
        .map(|spanned| spanned.node)
        .boxed();

    // `if let x = expr { body } else { else_body }` — same expression
    // shape as `if`, but the body branch is entered only when
    // `expr` is non-nil and binds the unwrapped value to `x`.
    let if_let_expr = just(Token::If)
        .ignore_then(just(Token::Let))
        .ignore_then(select! { Token::Ident(name) => name })
        .then_ignore(just(Token::Equals))
        .then(expr.clone())
        .then(block.clone())
        .then(just(Token::Else).ignore_then(block.clone()).or_not())
        .map(|(((name, value), body), else_body)| Expression::IfLet {
            name,
            value: Box::new(value),
            body: Box::new(body),
            else_body: else_body.map(Box::new),
        })
        .boxed();

    bool_lit
        .or(nil_lit)
        .or(float)
        .or(int)
        .or(string)
        .or(match_expr)
        // Try `if let` BEFORE `if`: both start with `Token::If` so
        // a bare `if let ...` would otherwise fall into the
        // condition-expression slot of the plain `if` parser and
        // produce a confusing error.
        .or(if_let_expr)
        .or(if_expr)
        .or(ident_or_call)
        .or(paren)
        .or(list_literal)
        .or(map_literal)
        .map_with(|node, extra| Spanned::new(node, extra.span()))
        .labelled("expression")
}

fn program<'src>() -> impl Parser<
    'src,
    TokenInput<'src>,
    Vec<Spanned<Statement>>,
    extra::Err<Rich<'src, Token, SimpleSpan>>,
> {
    // -- Expression precedence chain --
    //
    // Each layer wraps the previous one, from tightest to loosest:
    //   atom -> postfix (.field, .method()) -> negate (-) -> product (* /)
    //   -> sum (+ -) -> comparison (== != < > <= >=) -> not
    //   -> and -> or
    //
    // Slice 5 W26 lift: `if`/`if let` are now expressions, so the
    // primary expression chain has to be able to parse a block
    // (`{ stmt; stmt; ... }`) for their body. Blocks contain
    // statements which contain expressions, so we have a mutual
    // recursion: expr ⇄ stmt ⇄ block. We break it via
    // `Recursive::declare()` for the block parser, build `expr`
    // using the declared block, then build `stmt` (which uses
    // `expr.clone()`), then define `block` from `stmt`.
    let mut block = Recursive::declare();

    let expr = recursive(|expr| {
        let atom = atom(expr.clone(), block.clone());

        // Field-value pair used inside object literal braces. Same shape
        // as the one in `atom()`, but redefined here for postfix scope.
        let obj_field_value = select! { Token::Ident(name) => name }
            .then_ignore(just(Token::Colon))
            .then(expr.clone());

        // Postfix step: `.field`, `.method(args)`, or `.Type { fields }`
        // (the last one promotes the whole accumulated chain to an
        // ObjLiteral with a qualified type path).
        // Newlines inside postfix object literal braces (zero or more).
        let pnl = just(Token::Newline).repeated();

        let postfix_obj_fields = obj_field_value
            .clone()
            .separated_by(just(Token::Comma).then(pnl.clone()))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(
                just(Token::LeftCurly).then(pnl.clone()),
                pnl.clone().then(just(Token::RightCurly)),
            );

        let postfix_call_args = expr
            .clone()
            .separated_by(just(Token::Comma))
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LeftParen), just(Token::RightParen));

        // Each postfix step yields one of four suffix variants. A step
        // is either a `.ident(...)`-style dotted access or a bracket
        // `[expr]` index; the two grammars are kept in separate
        // `choice` branches because only the first starts with `Dot`.
        enum DottedTail {
            Call(Vec<Spanned<Expression>>),
            ObjLit(Vec<(String, Spanned<Expression>)>),
            Field,
        }

        let dotted_suffix = just(Token::Dot)
            .ignore_then(select! { Token::Ident(name) => name })
            .then(choice((
                postfix_call_args.map(DottedTail::Call),
                postfix_obj_fields.map(DottedTail::ObjLit),
                chumsky::primitive::empty().map(|_| DottedTail::Field),
            )))
            .map(|(name, tail)| match tail {
                DottedTail::Call(args) => PostfixSuffix::Call(name, args),
                DottedTail::ObjLit(fields) => PostfixSuffix::ObjLit(name, fields),
                DottedTail::Field => PostfixSuffix::Field(name),
            });

        let index_suffix = expr
            .clone()
            .delimited_by(just(Token::LeftBracket), just(Token::RightBracket))
            .map(PostfixSuffix::Index);

        let postfix_step = choice((dotted_suffix, index_suffix));

        let postfix = atom
            .clone()
            .foldl(postfix_step.repeated(), |object, suffix| {
                let span = object.span.clone();
                match suffix {
                    PostfixSuffix::Call(name, args) => Spanned {
                        node: Expression::MethodCall {
                            object: Box::new(object),
                            method: name,
                            args,
                        },
                        span,
                    },
                    PostfixSuffix::Field(name) => Spanned {
                        node: Expression::FieldAccess {
                            object: Box::new(object),
                            field: name,
                        },
                        span,
                    },
                    PostfixSuffix::ObjLit(name, fields) => {
                        // Walk the accumulated chain back to the root Ident
                        // to build the qualified type path. The current
                        // `name` (just consumed) is the type name; the chain
                        // in `object` provides the module prefix segments.
                        let mut path = Vec::new();
                        collect_path(&object.node, &mut path);
                        path.push(name);
                        Spanned {
                            node: Expression::ObjLiteral {
                                type_name: path,
                                fields,
                            },
                            span,
                        }
                    }
                    PostfixSuffix::Index(index) => Spanned {
                        node: Expression::Index {
                            object: Box::new(object),
                            index: Box::new(index),
                        },
                        span,
                    },
                }
            });

        // Unary minus: tighter than *, so -2 * 3 is (-2) * 3.
        // .boxed() erases the deeply nested combinator type to speed up compilation.
        let negate = just(Token::Minus)
            .repeated()
            .foldr(postfix.boxed(), |_op, expr| {
                let span = expr.span.clone();
                Spanned {
                    node: Expression::UnaryOp {
                        op: UnaryOp::Negate,
                        expr: Box::new(expr),
                    },
                    span,
                }
            });

        // Helper: build a left-associative binary operator chain.
        // Each precedence level follows the same foldl pattern.
        let binop_fold = |left: Spanned<Expression>, (op, right): (BinOp, Spanned<Expression>)| {
            let span = left.span.start..right.span.end;
            Spanned {
                node: Expression::BinaryOp {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            }
        };

        // * and /
        let product = negate.clone().boxed().foldl(
            choice((
                just(Token::Multiply).to(BinOp::Mul),
                just(Token::Divide).to(BinOp::Div),
            ))
            .then(negate)
            .repeated(),
            binop_fold,
        );

        // + and -
        let sum = product
            .clone()
            .foldl(
                choice((
                    just(Token::Plus).to(BinOp::Add),
                    just(Token::Minus).to(BinOp::Sub),
                ))
                .then(product)
                .repeated(),
                binop_fold,
            )
            .boxed();

        // ..
        let range = sum
            .clone()
            .then(
                choice((
                    just(Token::DotDotEquals).to(true),
                    just(Token::DotDot).to(false),
                ))
                .then(sum.clone())
                .or_not(),
            )
            .map(|(start, end)| match end {
                Some((inclusive, end)) => {
                    let span = start.span.start..end.span.end;
                    Spanned {
                        node: Expression::Range {
                            start: Box::new(start),
                            end: Box::new(end),
                            inclusive,
                        },
                        span,
                    }
                }
                None => start,
            })
            .boxed();

        // == != < > <= >=
        let comparison = range.clone().foldl(
            choice((
                just(Token::EqualsEquals).to(BinOp::Equals),
                just(Token::NotEquals).to(BinOp::NotEquals),
                just(Token::LessThan).to(BinOp::LessThan),
                just(Token::GreaterThan).to(BinOp::GreaterThan),
                just(Token::LessThanEquals).to(BinOp::LessThanEquals),
                just(Token::GreaterThanEquals).to(BinOp::GreaterThanEquals),
            ))
            .then(range)
            .repeated(),
            binop_fold,
        );

        // not / try / !expr (prefix, right-associative)
        #[derive(Clone)]
        enum PrefixOp {
            Not,
            Try,
            UnwrapError,
        }

        let prefix_unary = choice((
            just(Token::Not).to(PrefixOp::Not),
            just(Token::Try).to(PrefixOp::Try),
            just(Token::Must).to(PrefixOp::UnwrapError),
        ))
        .repeated()
        .foldr(comparison.boxed(), |op, expr| {
            let span = expr.span.clone();
            match op {
                PrefixOp::Not => Spanned {
                    node: Expression::UnaryOp {
                        op: UnaryOp::Not,
                        expr: Box::new(expr),
                    },
                    span,
                },
                PrefixOp::Try => Spanned {
                    node: Expression::Try(Box::new(expr)),
                    span,
                },
                PrefixOp::UnwrapError => Spanned {
                    node: Expression::UnwrapError(Box::new(expr)),
                    span,
                },
            }
        })
        .boxed();

        // and
        let and = prefix_unary.clone().foldl(
            just(Token::And)
                .to(BinOp::And)
                .then(prefix_unary)
                .repeated(),
            binop_fold,
        );

        // or
        let or = and.clone().foldl(
            just(Token::Or).to(BinOp::Or).then(and).repeated(),
            binop_fold,
        );

        // orelse (nil coalescing — loosest binary operator).
        //
        // Right-associative: `a orelse b orelse c` parses as
        // `a orelse (b orelse c)`. Left-associative would type-error
        // because `(a orelse b)` produces a non-nillable T, and the
        // outer `orelse` would then have a non-nillable left operand.
        // The natural reading of a fallback chain is right-to-left,
        // and so is its type rule.
        //
        // Implementation: parse `(or orelse)*` pairs followed by a
        // final `or`, then `foldr` from the right so each new
        // Coalesce wraps the accumulated rest as its right child.
        let coalesce = or
            .clone()
            .then_ignore(just(Token::Orelse))
            .repeated()
            .foldr(or.clone().boxed(), |left, acc| {
                let span = left.span.start..acc.span.end;
                Spanned {
                    node: Expression::Coalesce {
                        left: Box::new(left),
                        right: Box::new(acc),
                    },
                    span,
                }
            });

        coalesce.labelled("expression").boxed()
    });

    // -- Statement parsers --

    let newlines = just(Token::Newline).repeated();

    // The stmt parser used to be `recursive(|stmt| ...)` because the
    // inner `block` parser referenced it. After the Slice 5 W26 lift,
    // `block` lives outside `stmt` and is defined post-hoc via the
    // outer `Recursive::declare()`, so the closure parameter is no
    // longer used inside. We keep the `recursive` shape to preserve
    // boxed-parser identity (and the chumsky `Recursive` lifetime
    // story); the parameter is intentionally unused.
    let stmt = recursive(|_stmt| {
        // import <ident> or import <ident>.<ident>.<ident>...
        let import_stmt = just(Token::Import)
            .ignore_then(
                select! { Token::Ident(name) => name }
                    .separated_by(just(Token::Dot))
                    .at_least(1)
                    .collect::<Vec<String>>(),
            )
            .map_with(|path, extra| Spanned::new(Statement::Import { path }, extra.span()))
            .labelled("import statement")
            .boxed();

        // Recursive type annotation parser supporting:
        //   T, mod.T        → TypeAnnotation::Named
        //   T?              → TypeAnnotation::Nillable
        //   !T              → TypeAnnotation::ErrorUnion
        //   [T]             → TypeAnnotation::List
        //   {K: V}          → TypeAnnotation::Map
        //   !(T?), (!T)?    → allowed with parentheses
        //   !T?             → rejected (ambiguous without parentheses)
        // Type grammar (Slice 5 — words-everywhere policy):
        //
        //   maybe T          → nillable
        //   error T          → error union
        //   maybe error T    → nillable error union (composition reads
        //                       left-to-right as prefix operators)
        //   error maybe T    → error union of nillable
        //   [T]              → list
        //   {K: V}           → map
        //   (T)              → parens
        //   T                → bare named type / dotted path
        //
        // Both `maybe` and `error` are prefix keywords; they compose
        // freely without parentheses because there's no parser
        // ambiguity between two prefix forms.
        let type_ann_parser = recursive(|type_ann_rec| {
            let dotted_name = select! { Token::Ident(name) => name }
                .separated_by(just(Token::Dot))
                .at_least(1)
                .collect::<Vec<String>>()
                .map(TypeAnnotation::Named);

            let paren_type = type_ann_rec
                .clone()
                .delimited_by(just(Token::LeftParen), just(Token::RightParen));

            let list_type = type_ann_rec
                .clone()
                .delimited_by(just(Token::LeftBracket), just(Token::RightBracket))
                .map(|inner| TypeAnnotation::List(Box::new(inner)));

            let map_type = type_ann_rec
                .clone()
                .then_ignore(just(Token::Colon))
                .then(type_ann_rec.clone())
                .delimited_by(just(Token::LeftCurly), just(Token::RightCurly))
                .map(|(key, value)| TypeAnnotation::Map(Box::new(key), Box::new(value)));

            let base_type = paren_type.or(list_type).or(map_type).or(dotted_name);

            // `maybe T` — prefix nillable.
            let nillable = just(Token::Maybe)
                .ignore_then(type_ann_rec.clone())
                .map(|inner| TypeAnnotation::Nillable(Box::new(inner)));

            // `error T` (loose) or `error T of E` (precise) — prefix
            // error union. The optional postfix `of <dotted_name>`
            // pins a specific error enum that the function is
            // allowed to return on the error side; omitting it
            // keeps the loose semantics where any `error enum`
            // value may appear. The `of` clause attaches to the
            // most recent `error` keyword, and no other type rule
            // knows `of`, so the greedy type_ann parse followed by
            // an optional `of` suffix is unambiguous.
            let precise_error_enum = just(Token::Of)
                .ignore_then(
                    select! { Token::Ident(name) => name }
                        .separated_by(just(Token::Dot))
                        .at_least(1)
                        .collect::<Vec<String>>(),
                )
                .or_not();
            let error_union = just(Token::Error)
                .ignore_then(type_ann_rec.clone())
                .then(precise_error_enum)
                .map(|(inner, error_enum)| TypeAnnotation::ErrorUnion {
                    error_enum,
                    inner: Box::new(inner),
                });

            // Try the prefix forms first so `maybe`/`error` are
            // recognized as type modifiers, not consumed as
            // identifiers by the bare-type fallthrough.
            choice((nillable, error_union, base_type))
        });

        let type_annotation = just(Token::Colon).ignore_then(type_ann_parser.clone());

        // The block parser is the outer `Recursive::declare()`-d one
        // (see the top of `program()` for the rationale). It's
        // captured by reference here so we can call `.clone()` to
        // use it in nested parsers, and it gets its body filled in
        // by `block.define(...)` after `stmt` is built.

        // -- Bindings --

        // Shared helper: parses `<keyword> <name> [: <type>] = <expr>` into a
        // Statement::Let or Statement::Val depending on `mutable`.
        let binding_stmt = |keyword: Token, label: &'static str, mutable: bool| {
            just(Token::Pub)
                .or_not()
                .map(|t| t.is_some())
                .then_ignore(just(keyword))
                .then(select! { Token::Ident(name) => name }.labelled("variable name"))
                .then(type_annotation.clone().or_not())
                .then_ignore(just(Token::Equals))
                .then(expr.clone())
                .map_with(move |(((is_pub, name), type_ann), value), extra| {
                    Spanned::new(
                        if mutable {
                            Statement::Let {
                                name,
                                type_ann,
                                value,
                                is_pub,
                            }
                        } else {
                            Statement::Val {
                                name,
                                type_ann,
                                value,
                                is_pub,
                            }
                        },
                        extra.span(),
                    )
                })
                .labelled(label)
        };

        let let_stmt = binding_stmt(Token::Let, "let statement", true).boxed();
        let val_stmt = binding_stmt(Token::Val, "val statement", false).boxed();

        // -- Assignments --

        // v.x = expr or v.x.y = expr (must be tried before plain assignment)
        let field_assign_stmt = select! { Token::Ident(name) => name }
            .then(
                just(Token::Dot)
                    .ignore_then(select! { Token::Ident(field) => field })
                    .repeated()
                    .at_least(1)
                    .collect::<Vec<_>>(),
            )
            .then_ignore(just(Token::Equals))
            .then(expr.clone())
            .map_with(|((root, segments), value), extra| {
                let span = extra.span();
                let last_idx = segments.len() - 1;
                let field = segments[last_idx].clone();
                let object = segments.into_iter().take(last_idx).fold(
                    Spanned::new(Expression::Ident(root), span),
                    |object, field| {
                        Spanned::new(
                            Expression::FieldAccess {
                                object: Box::new(object),
                                field,
                            },
                            span,
                        )
                    },
                );
                Spanned::new(
                    Statement::FieldAssignment {
                        object,
                        field,
                        value,
                    },
                    span,
                )
            })
            .boxed();

        // x[i] = expr, x.y[i] = expr, or x[i].y = expr.
        let index_assign_stmt = select! { Token::Ident(name) => name }
            .then(
                just(Token::Dot)
                    .ignore_then(select! { Token::Ident(field) => field })
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .then(
                expr.clone()
                    .delimited_by(just(Token::LeftBracket), just(Token::RightBracket)),
            )
            .then(
                just(Token::Dot)
                    .ignore_then(select! { Token::Ident(field) => field })
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .then_ignore(just(Token::Equals))
            .then(expr.clone())
            .map_with(
                |((((name, pre_fields), index), post_fields), value), extra| {
                    let s: SimpleSpan = extra.span();
                    let object = pre_fields.into_iter().fold(
                        Spanned::new(Expression::Ident(name), s),
                        |object, field| {
                            Spanned::new(
                                Expression::FieldAccess {
                                    object: Box::new(object),
                                    field,
                                },
                                s,
                            )
                        },
                    );

                    if post_fields.is_empty() {
                        Spanned::new(
                            Statement::IndexAssignment {
                                object,
                                index,
                                value,
                            },
                            s,
                        )
                    } else {
                        let indexed = Spanned::new(
                            Expression::Index {
                                object: Box::new(object),
                                index: Box::new(index),
                            },
                            s,
                        );
                        let last_idx = post_fields.len() - 1;
                        let field = post_fields[last_idx].clone();
                        let object = post_fields.into_iter().take(last_idx).fold(
                            indexed,
                            |object, field| {
                                Spanned::new(
                                    Expression::FieldAccess {
                                        object: Box::new(object),
                                        field,
                                    },
                                    s,
                                )
                            },
                        );

                        Spanned::new(
                            Statement::FieldAssignment {
                                object,
                                field,
                                value,
                            },
                            s,
                        )
                    }
                },
            )
            .boxed();

        // x = expr
        let assign_stmt = select! { Token::Ident(name) => name }
            .then_ignore(just(Token::Equals))
            .then(expr.clone())
            .map_with(|(name, value), extra| {
                Spanned::new(Statement::Assignment { name, value }, extra.span())
            })
            .labelled("assign statement")
            .boxed();

        // -- Objects --

        // Optional `pub` modifier on a field or method.
        let pub_prefix = just(Token::Pub).or_not().map(|t| t.is_some());

        let obj_field = pub_prefix
            .clone()
            .then(select! { Token::Ident(name) => name })
            .then_ignore(just(Token::Colon))
            .then(type_ann_parser.clone())
            .map_with(|((is_pub, name), ty), extra| {
                let s: SimpleSpan = extra.span();
                ObjField {
                    name,
                    type_ann: ty,
                    span: s.start..s.end,
                    is_pub,
                }
            });

        // A parameter is `mut? ident type_annotation?`. The leading
        // `mut` opts the parameter into mutability per the W12
        // mutability cluster — without it, parameters are immutable
        // in Oryn (no opt-out at the call site).
        let param_list = just(Token::Mut)
            .or_not()
            .map(|t| t.is_some())
            .then(select! { Token::Ident(name) => name })
            .then(type_annotation.clone().or_not())
            .map(|((is_mut, name), type_ann)| Param {
                name,
                type_ann,
                is_mut,
            })
            .separated_by(just(Token::Comma))
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LeftParen), just(Token::RightParen));

        let return_type_ann = just(Token::Arrow)
            .ignore_then(type_ann_parser.clone())
            .or_not();

        // Shared header: `fn <name> (<params>) -> <return_type>`
        // Used by both obj_method (optional body) and fn_stmt (required body).
        let fn_header = just(Token::Fn)
            .ignore_then(select! { Token::Ident(name) => name })
            .then(param_list.clone())
            .then(return_type_ann.clone());

        // Methods are mutating iff their `self` parameter is declared
        // `mut self`. The `mut` keyword lives in the parameter list,
        // alongside the `mut` opt-in for non-self parameters — same
        // word, same meaning, same position. Plain `self` methods can
        // read `self` but cannot mutate it. There's no separate
        // method-level mut keyword; `is_mut` on the method is
        // *derived* from the `self` parameter at compile time.
        let obj_method = pub_prefix
            .clone()
            .then(fn_header.clone())
            .then(block.clone().or_not())
            .map_with(|((is_pub, ((name, params), return_type)), body), extra| {
                let s: SimpleSpan = extra.span();
                ObjMethod {
                    name,
                    params,
                    body,
                    return_type,
                    is_pub,
                    span: s.start..s.end,
                }
            });

        enum ObjItem {
            Field(ObjField),
            Method(ObjMethod),
            Use(Vec<String>),
        }

        // `use Foo` or `use math.shapes.Foo` — the dotted path lets
        // obj declarations compose types from other modules.
        let use_item = just(Token::Use).ignore_then(
            select! { Token::Ident(name) => name }
                .separated_by(just(Token::Dot))
                .at_least(1)
                .collect::<Vec<String>>(),
        );

        let obj_item = obj_method
            .map(ObjItem::Method)
            .or(use_item.map(ObjItem::Use))
            .or(obj_field.clone().map(ObjItem::Field));

        // obj <name> { <field>, <field> } or { <field> \n <field> }
        let field_sep = just(Token::Comma)
            .then(newlines.clone().or_not())
            .or(newlines.clone().map(|n| (Token::Newline, Some(n))))
            .ignored();

        let obj_stmt = just(Token::Pub)
            .or_not()
            .map(|t| t.is_some())
            .then_ignore(just(Token::Struct))
            .then(select! { Token::Ident(name) => name })
            .then(
                obj_item
                    .separated_by(field_sep.clone())
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(
                        just(Token::LeftCurly).then(newlines.clone().or_not()),
                        newlines.clone().or_not().then(just(Token::RightCurly)),
                    ),
            )
            .map_with(|((is_pub, name), items), extra| {
                let mut fields = Vec::new();
                let mut methods = Vec::new();
                let mut uses = Vec::new();

                for item in items {
                    match item {
                        ObjItem::Field(f) => fields.push(f),
                        ObjItem::Method(m) => methods.push(m),
                        ObjItem::Use(name) => uses.push(name),
                    }
                }

                Spanned::new(
                    Statement::ObjDef {
                        name,
                        fields,
                        methods,
                        uses,
                        is_pub,
                    },
                    extra.span(),
                )
            })
            .labelled("object definition")
            .boxed();

        // -- Enums --
        //
        // `enum Name { Variant1, Variant2 { field: T, ... }, ... }`
        // Variants are bare names for nullary cases or have a
        // braced payload field list using the same shape as obj
        // fields. Variant separator follows the same comma-or-
        // newline rule as obj fields.
        //
        // Variant payloads parse as `Vec<ObjField>` directly so the
        // compiler can apply the same field-resolution machinery
        // to both shapes.
        let variant_payload_fields = obj_field
            .clone()
            .separated_by(field_sep.clone())
            .allow_trailing()
            .collect::<Vec<ObjField>>()
            .delimited_by(
                just(Token::LeftCurly).then(newlines.clone().or_not()),
                newlines.clone().or_not().then(just(Token::RightCurly)),
            );

        let enum_variant = select! { Token::Ident(name) => name }
            .then(variant_payload_fields.or_not())
            .map_with(|(name, fields), extra| {
                let s: SimpleSpan = extra.span();
                EnumVariant {
                    name,
                    fields: fields.unwrap_or_default(),
                    span: s.start..s.end,
                }
            })
            .labelled("enum variant");

        // `[pub] [error] enum Name { ... }` — optional `pub` visibility
        // modifier followed by optional `error` marker. The canonical
        // order is `pub error enum`; `error pub enum` is not accepted.
        // An `error enum` declaration's values are valid on the error
        // side of any `error T` union (replaces the old string-backed
        // `Error("msg")` builtin).
        let enum_stmt = just(Token::Pub)
            .or_not()
            .map(|t| t.is_some())
            .then(just(Token::Error).or_not().map(|t| t.is_some()))
            .then_ignore(just(Token::Enum))
            .then(select! { Token::Ident(name) => name })
            .then(
                enum_variant
                    .separated_by(field_sep)
                    .allow_trailing()
                    .collect::<Vec<EnumVariant>>()
                    .delimited_by(
                        just(Token::LeftCurly).then(newlines.clone().or_not()),
                        newlines.clone().or_not().then(just(Token::RightCurly)),
                    ),
            )
            .map_with(|(((is_pub, is_error), name), variants), extra| {
                Spanned::new(
                    Statement::EnumDef {
                        name,
                        variants,
                        is_pub,
                        is_error,
                    },
                    extra.span(),
                )
            })
            .labelled("enum definition")
            .boxed();

        // -- Functions --

        let fn_stmt = just(Token::Pub)
            .or_not()
            .map(|t| t.is_some())
            .then(fn_header.clone())
            .then(block.clone())
            .map_with(|((is_pub, ((name, params), return_type)), body), extra| {
                Spanned::new(
                    Statement::Function {
                        name,
                        params,
                        body,
                        return_type,
                        is_pub,
                    },
                    extra.span(),
                )
            })
            .labelled("function")
            .boxed();

        let return_stmt = just(Token::Return)
            .ignore_then(expr.clone().or_not())
            .map_with(|value, extra| Spanned::new(Statement::Return(value), extra.span()))
            .labelled("return");

        // -- Control flow --
        //
        // Slice 5 W26: `if` and `if let` are now expressions, parsed
        // by `atom()`. A bare `if cond { body }` line at statement
        // position parses as an expression and gets wrapped in
        // `Statement::Expression` by the `expr_stmt` fallthrough.

        // `test "name" { ... }` — module-level test block. The name is a
        // required string literal; identifier names are rejected by the
        // parser so test reports always render a readable sentence.
        let test_stmt = just(Token::Test)
            .ignore_then(select! { Token::String(s) => s })
            .then(block.clone())
            .map_with(|(name, body), extra| {
                Spanned::new(Statement::Test { name, body }, extra.span())
            })
            .labelled("test statement")
            .boxed();

        // `assert(expr)` — parses with required parentheses and exactly
        // one expression. The expression's span is propagated to the
        // `Statement::Assert` node so the CLI can render the asserted
        // source snippet in failure messages.
        let assert_stmt = just(Token::Assert)
            .ignore_then(just(Token::LeftParen))
            .ignore_then(expr.clone())
            .then_ignore(just(Token::RightParen))
            .map_with(|condition, extra| {
                Spanned::new(Statement::Assert { condition }, extra.span())
            })
            .labelled("assert statement")
            .boxed();

        let while_stmt = just(Token::While)
            .ignore_then(expr.clone())
            .then(block.clone())
            .map_with(|(condition, body), extra| {
                Spanned::new(Statement::While { condition, body }, extra.span())
            })
            .labelled("while statement");

        let for_stmt = just(Token::For)
            .ignore_then(select! { Token::Ident(name) => name })
            .then_ignore(just(Token::In))
            .then(expr.clone())
            .then(block.clone())
            .map_with(|((name, iterable), body), extra| {
                Spanned::new(
                    Statement::For {
                        name,
                        iterable,
                        body,
                    },
                    extra.span(),
                )
            })
            .labelled("for statement")
            .boxed();

        let break_stmt =
            just(Token::Break).map_with(|_, extra| Spanned::new(Statement::Break, extra.span()));

        let continue_stmt = just(Token::Continue)
            .map_with(|_, extra| Spanned::new(Statement::Continue, extra.span()));

        let block_stmt = block
            .clone()
            .map_with(|expr, extra| Spanned::new(Statement::Expression(expr), extra.span()));

        // A bare expression as a statement (e.g. a function call).
        let expr_stmt = expr
            .clone()
            .map_with(|expr, extra| Spanned::new(Statement::Expression(expr), extra.span()));

        // -- Statement dispatch (order matters for ambiguity resolution) --

        import_stmt
            .or(let_stmt)
            .or(field_assign_stmt)
            .or(index_assign_stmt)
            .or(assign_stmt)
            .or(val_stmt)
            .or(obj_stmt)
            .or(enum_stmt)
            .or(fn_stmt)
            .or(test_stmt)
            .or(assert_stmt)
            .or(return_stmt)
            .or(while_stmt)
            .or(for_stmt)
            .or(break_stmt)
            .or(continue_stmt)
            .or(block_stmt)
            .or(expr_stmt)
    })
    .labelled("statement");

    // Define the block parser now that `stmt` is built. This closes
    // the mutual recursion: `block` was declared upfront so the
    // expression parser could reference it for `if`/`if let` bodies.
    {
        let block_newlines = just(Token::Newline).repeated();
        block.define(
            stmt.clone()
                .separated_by(block_newlines.clone())
                .allow_trailing()
                .collect::<Vec<Spanned<Statement>>>()
                .delimited_by(
                    just(Token::LeftCurly).then(block_newlines.clone()),
                    block_newlines.or_not().then(just(Token::RightCurly)),
                )
                .map_with(|stmts, extra| Spanned::new(Expression::Block(stmts), extra.span())),
        );
    }

    // Top level: newline-separated statements.
    let newlines = just(Token::Newline).repeated().at_least(1);

    newlines
        .clone()
        .or_not()
        .ignore_then(stmt.separated_by(newlines).allow_trailing().collect())
}

/// Split a string token's content into literal and interpolation parts.
/// `base_offset` is the byte offset in the original source where the
/// string content begins (after the opening `"`), used to produce
/// correct spans for interpolated expressions.
fn parse_string_content(input: String, base_offset: usize) -> Expression {
    let mut parts = Vec::new();
    let mut literal_buffer = String::new();
    let mut expr_buffer = String::new();
    let mut brace_depth: u32 = 0;
    // Byte position within `input`, used to compute source offsets.
    let mut byte_pos: usize = 0;
    // Byte position where the current interpolation's content starts
    // (after the opening `{`).
    let mut expr_start: usize = 0;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        let char_len = c.len_utf8();

        if brace_depth > 0 {
            // Inside an interpolation expression.
            if c == '{' {
                brace_depth += 1;
                expr_buffer.push(c);
            } else if c == '}' {
                brace_depth -= 1;
                if brace_depth == 0 {
                    // End of interpolation — parse the collected expression.
                    let offset = base_offset + expr_start;
                    if let Some(expr) = parse_interp_expression(&expr_buffer, offset) {
                        parts.push(StringPart::Interp(expr));
                    } else {
                        // Failed to parse — preserve the original text as a literal
                        // so nothing silently disappears.
                        literal_buffer.push('{');
                        literal_buffer.push_str(&expr_buffer);
                        literal_buffer.push('}');
                    }
                    expr_buffer.clear();
                } else {
                    expr_buffer.push(c);
                }
            } else {
                expr_buffer.push(c);
            }
        } else if c == '\\' {
            // Escape sequences in literal mode.
            if let Some(&next) = chars.peek() {
                let next_len = next.len_utf8();
                match next {
                    '\\' | '{' | '}' => {
                        literal_buffer.push(next);
                        chars.next();
                        byte_pos += next_len;
                    }
                    _ => {
                        // Unknown escape — preserve both characters verbatim.
                        literal_buffer.push('\\');
                        literal_buffer.push(next);
                        chars.next();
                        byte_pos += next_len;
                    }
                }
            } else {
                // Trailing backslash at end of string — preserve it.
                literal_buffer.push('\\');
            }
        } else if c == '{' {
            // Start of interpolation.
            if !literal_buffer.is_empty() {
                parts.push(StringPart::Literal(std::mem::take(&mut literal_buffer)));
            }
            brace_depth = 1;
            // The expression content starts at the next byte after `{`.
            expr_start = byte_pos + char_len;
        } else {
            literal_buffer.push(c);
        }

        byte_pos += char_len;
    }

    // Unclosed interpolation — preserve as literal text so nothing disappears.
    if brace_depth > 0 {
        literal_buffer.push('{');
        literal_buffer.push_str(&expr_buffer);
    }

    if !literal_buffer.is_empty() {
        parts.push(StringPart::Literal(literal_buffer));
    }

    if parts.len() == 1
        && let StringPart::Literal(s) = &parts[0]
    {
        return Expression::String(s.clone());
    }

    Expression::StringInterp(parts)
}

/// Lex and parse a single expression from a string interpolation fragment.
/// `base_offset` is the byte offset in the original source where the
/// expression text begins, used to shift spans so error messages and
/// LSP features point to the correct source location.
fn parse_interp_expression(source: &str, base_offset: usize) -> Option<Spanned<Expression>> {
    let (tokens, lex_errors) = lexer::lex(source);

    if !lex_errors.is_empty() || tokens.is_empty() {
        return None;
    }

    // Offset token spans to their position in the original source.
    let tokens: Vec<_> = tokens
        .into_iter()
        .map(|(tok, span)| (tok, (span.start + base_offset)..(span.end + base_offset)))
        .collect();

    let (stmts, parse_errors) = parse(tokens);

    if !parse_errors.is_empty() {
        return None;
    }

    // We expect exactly one statement: an expression statement.
    if stmts.len() == 1
        && let Statement::Expression(expr) = stmts.into_iter().next()?.node
    {
        return Some(expr);
    }

    None
}
