use chumsky::IterParser as _;
use chumsky::Parser;
use chumsky::input::{Input as _, MappedInput};
use chumsky::prelude::{Rich, SimpleSpan, choice, extra, just, recursive, select};

use crate::errors::OrynError;
use crate::lexer::{self, Token};

use super::ast::*;

// Chumsky needs tokens paired with their source spans.
type TokenSpanned = (Token, SimpleSpan);
// The input type chumsky operates on. A slice of `TokenSpanned` tokens.
type TokenInput<'src> = MappedInput<'src, Token, SimpleSpan, &'src [TokenSpanned]>;

/// What kind of suffix follows a postfix `.field` access. Built once
/// per `.ident` step in the postfix parser; the foldl callback decides
/// whether to construct a FieldAccess, MethodCall, or ObjLiteral.
enum PostfixSuffix {
    Field,
    Call(Vec<Spanned<Expression>>),
    ObjLit(Vec<(String, Spanned<Expression>)>),
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
fn atom<'src>(
    expr: impl Parser<
        'src,
        TokenInput<'src>,
        Spanned<Expression>,
        extra::Err<Rich<'src, Token, SimpleSpan>>,
    > + Clone,
) -> impl Parser<'src, TokenInput<'src>, Spanned<Expression>, extra::Err<Rich<'src, Token, SimpleSpan>>>
+ Clone {
    let bool_lit = select! { Token::True => Expression::True, Token::False => Expression::False };
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
        .delimited_by(just(Token::LeftParen), just(Token::RightParen))
        .map(|spanned| spanned.node);

    bool_lit
        .or(float)
        .or(int)
        .or(string)
        .or(ident_or_call)
        .or(paren)
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
    let expr = recursive(|expr| {
        let atom = atom(expr.clone());

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

        // Each postfix step yields one of three suffix variants.
        let postfix_step = just(Token::Dot)
            .ignore_then(select! { Token::Ident(name) => name })
            .then(choice((
                postfix_call_args.map(PostfixSuffix::Call),
                postfix_obj_fields.map(PostfixSuffix::ObjLit),
                chumsky::primitive::empty().map(|_| PostfixSuffix::Field),
            )));

        let postfix = atom
            .clone()
            .foldl(postfix_step.repeated(), |object, (name, suffix)| {
                let span = object.span.clone();
                match suffix {
                    PostfixSuffix::Call(args) => Spanned {
                        node: Expression::MethodCall {
                            object: Box::new(object),
                            method: name,
                            args,
                        },
                        span,
                    },
                    PostfixSuffix::Field => Spanned {
                        node: Expression::FieldAccess {
                            object: Box::new(object),
                            field: name,
                        },
                        span,
                    },
                    PostfixSuffix::ObjLit(fields) => {
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

        // not (prefix, right-associative)
        let not = just(Token::Not)
            .repeated()
            .foldr(comparison.boxed(), |_op, expr| {
                let span = expr.span.clone();
                Spanned {
                    node: Expression::UnaryOp {
                        op: UnaryOp::Not,
                        expr: Box::new(expr),
                    },
                    span,
                }
            })
            .boxed();

        // and
        let and = not.clone().foldl(
            just(Token::And).to(BinOp::And).then(not).repeated(),
            binop_fold,
        );

        // or (loosest)
        let or = and.clone().foldl(
            just(Token::Or).to(BinOp::Or).then(and).repeated(),
            binop_fold,
        );

        or.labelled("expression").boxed()
    });

    // -- Statement parsers --

    let newlines = just(Token::Newline).repeated();

    let stmt = recursive(|stmt| {
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

        // A dotted type path: `i32`, `Vec2`, or `math.Vec2`,
        // `std.collections.List`. Stored as a `Vec<String>` so the
        // compiler can look it up against `obj_table` (single segment)
        // or `ModuleTable` (multi-segment).
        let dotted_type = select! { Token::Ident(name) => name }
            .separated_by(just(Token::Dot))
            .at_least(1)
            .collect::<Vec<String>>()
            .map(TypeAnnotation::Named);

        let type_annotation = just(Token::Colon).ignore_then(dotted_type.clone());

        // { stmt \n stmt \n ... }
        let block = stmt
            .clone()
            .separated_by(newlines.clone())
            .allow_trailing()
            .collect::<Vec<Spanned<Statement>>>()
            .delimited_by(
                just(Token::LeftCurly).then(newlines.clone()),
                newlines.clone().or_not().then(just(Token::RightCurly)),
            )
            .map_with(|stmts, extra| Spanned::new(Expression::Block(stmts), extra.span()));

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

        // v.x = expr (must be tried before plain assignment)
        let field_assign_stmt = select! { Token::Ident(name) => name }
            .then_ignore(just(Token::Dot))
            .then(select! { Token::Ident(field) => field })
            .then_ignore(just(Token::Equals))
            .then(expr.clone())
            .map_with(|((name, field), value), extra| {
                let name_span = extra.span();
                Spanned::new(
                    Statement::FieldAssignment {
                        object: Spanned::new(Expression::Ident(name), name_span),
                        field,
                        value,
                    },
                    name_span,
                )
            })
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
            .then(dotted_type.clone())
            .map_with(|((is_pub, name), ty), extra| {
                let s: SimpleSpan = extra.span();
                ObjField {
                    name,
                    type_ann: ty,
                    span: s.start..s.end,
                    is_pub,
                }
            });

        let param_list = select! { Token::Ident(name) => name }
            .then(type_annotation.clone().or_not())
            .separated_by(just(Token::Comma))
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LeftParen), just(Token::RightParen));

        let return_type_ann = just(Token::Arrow).ignore_then(dotted_type.clone()).or_not();

        // Shared header: `fn <name> (<params>) -> <return_type>`
        // Used by both obj_method (optional body) and fn_stmt (required body).
        let fn_header = just(Token::Fn)
            .ignore_then(select! { Token::Ident(name) => name })
            .then(param_list.clone())
            .then(return_type_ann.clone());

        let obj_method = pub_prefix
            .clone()
            .then(fn_header.clone())
            .then(block.clone().or_not())
            .map(
                |((is_pub, ((name, params), return_type)), body)| ObjMethod {
                    name,
                    params,
                    body,
                    return_type,
                    is_pub,
                },
            );

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
            .or(obj_field.map(ObjItem::Field));

        // obj <name> { <field>, <field> } or { <field> \n <field> }
        let field_sep = just(Token::Comma)
            .then(newlines.clone().or_not())
            .or(newlines.clone().map(|n| (Token::Newline, Some(n))))
            .ignored();

        let obj_stmt = just(Token::Pub)
            .or_not()
            .map(|t| t.is_some())
            .then_ignore(just(Token::Obj))
            .then(select! { Token::Ident(name) => name })
            .then(
                obj_item
                    .separated_by(field_sep)
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

        let return_stmt = just(Token::Rn)
            .ignore_then(expr.clone().or_not())
            .map_with(|value, extra| Spanned::new(Statement::Return(value), extra.span()))
            .labelled("return");

        // -- Control flow --

        let if_stmt = just(Token::If)
            .ignore_then(recursive(|if_body| {
                let else_branch =
                    just(Token::Else)
                        .ignore_then(block.clone())
                        .or(just(Token::Elif).ignore_then(if_body).map_with(
                            |elif_stmt: Spanned<Statement>, extra| {
                                // Desugar `elif` into an else-block containing a single if statement.
                                Spanned::new(Expression::Block(vec![elif_stmt]), extra.span())
                            },
                        ));

                expr.clone()
                    .then(block.clone())
                    .then(else_branch.or_not())
                    .map_with(|((condition, body), else_body), extra| {
                        Spanned::new(
                            Statement::If {
                                condition,
                                body,
                                else_body,
                            },
                            extra.span(),
                        )
                    })
            }))
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
            .or(assign_stmt)
            .or(val_stmt)
            .or(obj_stmt)
            .or(fn_stmt)
            .or(return_stmt)
            .or(if_stmt)
            .or(while_stmt)
            .or(for_stmt)
            .or(break_stmt)
            .or(continue_stmt)
            .or(block_stmt)
            .or(expr_stmt)
    })
    .labelled("statement");

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
