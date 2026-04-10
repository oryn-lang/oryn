use super::*;

use crate::errors::OrynError;
use crate::lexer::lex;

/// Helper: lex + parse source, assert no errors, return statements.
fn parse_ok(source: &str) -> Vec<Spanned<Statement>> {
    let (tokens, lex_errors) = lex(source);
    assert!(lex_errors.is_empty());
    let (stmts, parse_errors) = parse(tokens);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");

    stmts
}

/// Helper: lex + parse source, return collected parse errors (ignores
/// lex errors). Used for negative parser tests.
fn parse_errors(source: &str) -> Vec<OrynError> {
    let (tokens, _lex_errors) = lex(source);
    let (_stmts, errors) = parse(tokens);
    errors
}

#[test]
fn builds_ast_from_tokens() {
    let stmts = parse_ok("let x = 5");

    assert_eq!(stmts.len(), 1);
    assert!(matches!(&stmts[0].node, Statement::Let { name, .. } if name == "x"));
}

#[test]
fn reports_parse_errors() {
    let (tokens, _) = lex("let = 5");
    let (_, errors) = parse(tokens);

    assert!(!errors.is_empty());
}

#[test]
fn expressions_carry_spans() {
    let stmts = parse_ok("5 + 10");

    assert_eq!(stmts.len(), 1);
    // The whole expression "5 + 10" should span from 0..6
    assert_eq!(stmts[0].span.start, 0);
}

#[test]
fn parses_range_expression() {
    let stmts = parse_ok("0..10");

    assert!(matches!(
        &stmts[0].node,
        Statement::Expression(Spanned {
            node: Expression::Range {
                inclusive: false,
                ..
            },
            ..
        })
    ));
}

#[test]
fn parses_inclusive_range_expression() {
    let stmts = parse_ok("0..=10");

    assert!(matches!(
        &stmts[0].node,
        Statement::Expression(Spanned {
            node: Expression::Range {
                inclusive: true,
                ..
            },
            ..
        })
    ));
}

#[test]
fn parses_for_statement() {
    let stmts = parse_ok("for i in 0..3 { print(i) }");

    assert!(matches!(
        &stmts[0].node,
        Statement::For { name, .. } if name == "i"
    ));
}

// ---------------------------------------------------------------------------
// Phase 1: Nil and error handling — surface syntax and type shapes
// ---------------------------------------------------------------------------

/// Helper: lex + parse source, assert there ARE errors.
fn parse_err(source: &str) -> Vec<OrynError> {
    let (tokens, lex_errors) = lex(source);
    assert!(
        lex_errors.is_empty(),
        "unexpected lex errors: {lex_errors:?}"
    );
    let (_, parse_errors) = parse(tokens);
    assert!(
        !parse_errors.is_empty(),
        "expected parse errors for: {source}"
    );
    parse_errors
}

// -- Type annotation parsing --

#[test]
fn parses_nillable_type_annotation() {
    let stmts = parse_ok("let x: int? = 5");
    assert!(matches!(
        &stmts[0].node,
        Statement::Let { type_ann: Some(TypeAnnotation::Nillable(inner)), .. }
            if matches!(inner.as_ref(), TypeAnnotation::Named(segments) if segments == &["int"])
    ));
}

#[test]
fn parses_error_union_type_annotation() {
    let stmts = parse_ok("let x: !int = 5");
    assert!(matches!(
        &stmts[0].node,
        Statement::Let { type_ann: Some(TypeAnnotation::ErrorUnion(inner)), .. }
            if matches!(inner.as_ref(), TypeAnnotation::Named(segments) if segments == &["int"])
    ));
}

#[test]
fn parses_bare_named_type_annotation() {
    let stmts = parse_ok("let x: int = 5");
    assert!(matches!(
        &stmts[0].node,
        Statement::Let { type_ann: Some(TypeAnnotation::Named(segments)), .. }
            if segments == &["int"]
    ));
}

#[test]
fn parses_dotted_named_type_annotation() {
    let stmts = parse_ok("let x: math.Vec2 = 5");
    assert!(matches!(
        &stmts[0].node,
        Statement::Let { type_ann: Some(TypeAnnotation::Named(segments)), .. }
            if segments == &["math", "Vec2"]
    ));
}

#[test]
fn parses_parenthesized_error_union_of_nillable() {
    // !(T?) — error union wrapping a nillable
    let stmts = parse_ok("let x: !(int?) = 5");
    match &stmts[0].node {
        Statement::Let {
            type_ann: Some(TypeAnnotation::ErrorUnion(inner)),
            ..
        } => {
            assert!(matches!(inner.as_ref(), TypeAnnotation::Nillable(inner2)
                if matches!(inner2.as_ref(), TypeAnnotation::Named(s) if s == &["int"])));
        }
        other => panic!("expected ErrorUnion(Nillable(Named)), got {other:?}"),
    }
}

#[test]
fn parses_parenthesized_nillable_of_error_union() {
    // (!T)? — nillable wrapping an error union
    let stmts = parse_ok("let x: (!int)? = 5");
    match &stmts[0].node {
        Statement::Let {
            type_ann: Some(TypeAnnotation::Nillable(inner)),
            ..
        } => {
            assert!(matches!(inner.as_ref(), TypeAnnotation::ErrorUnion(inner2)
                if matches!(inner2.as_ref(), TypeAnnotation::Named(s) if s == &["int"])));
        }
        other => panic!("expected Nillable(ErrorUnion(Named)), got {other:?}"),
    }
}

#[test]
fn rejects_ambiguous_bang_t_question() {
    // !T? without parentheses must be rejected
    parse_err("let x: !int? = 5");
}

#[test]
fn parses_nillable_return_type() {
    let stmts = parse_ok("fn foo() -> int? { 5 }");
    assert!(matches!(
        &stmts[0].node,
        Statement::Function {
            return_type: Some(TypeAnnotation::Nillable(_)),
            ..
        }
    ));
}

#[test]
fn parses_error_union_return_type() {
    let stmts = parse_ok("fn foo() -> !int { 5 }");
    assert!(matches!(
        &stmts[0].node,
        Statement::Function {
            return_type: Some(TypeAnnotation::ErrorUnion(_)),
            ..
        }
    ));
}

#[test]
fn parses_nillable_param_type() {
    let stmts = parse_ok("fn foo(x: int?) { x }");
    match &stmts[0].node {
        Statement::Function { params, .. } => {
            assert!(matches!(&params[0].1, Some(TypeAnnotation::Nillable(_))));
        }
        other => panic!("expected Function, got {other:?}"),
    }
}

// -- Nil literal --

#[test]
fn parses_nil_literal() {
    let stmts = parse_ok("nil");
    assert!(matches!(
        &stmts[0].node,
        Statement::Expression(Spanned {
            node: Expression::Nil,
            ..
        })
    ));
}

#[test]
fn parses_nil_in_let_binding() {
    let stmts = parse_ok("let x: int? = nil");
    match &stmts[0].node {
        Statement::Let { value, .. } => {
            assert!(matches!(value.node, Expression::Nil));
        }
        other => panic!("expected Let, got {other:?}"),
    }
}

// -- Try expression --

#[test]
fn parses_try_expression() {
    let stmts = parse_ok("try foo()");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::Try(inner),
            ..
        }) => {
            assert!(matches!(inner.node, Expression::Call { .. }));
        }
        other => panic!("expected Try(Call), got {other:?}"),
    }
}

#[test]
fn parses_nested_try() {
    let stmts = parse_ok("try try foo()");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::Try(inner),
            ..
        }) => {
            assert!(matches!(inner.node, Expression::Try(_)));
        }
        other => panic!("expected Try(Try(_)), got {other:?}"),
    }
}

// -- Unwrap error expression (!expr) --

#[test]
fn parses_unwrap_error_expression() {
    let stmts = parse_ok("!foo()");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::UnwrapError(inner),
            ..
        }) => {
            assert!(matches!(inner.node, Expression::Call { .. }));
        }
        other => panic!("expected UnwrapError(Call), got {other:?}"),
    }
}

#[test]
fn parses_unwrap_error_on_ident() {
    let stmts = parse_ok("!x");
    assert!(matches!(
        &stmts[0].node,
        Statement::Expression(Spanned {
            node: Expression::UnwrapError(_),
            ..
        })
    ));
}

// -- Coalesce expression (orelse) --

#[test]
fn parses_coalesce_expression() {
    let stmts = parse_ok("a orelse b");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::Coalesce { left, right },
            ..
        }) => {
            assert!(matches!(left.node, Expression::Ident(_)));
            assert!(matches!(right.node, Expression::Ident(_)));
        }
        other => panic!("expected Coalesce, got {other:?}"),
    }
}

#[test]
fn coalesce_is_left_associative() {
    // a orelse b orelse c → Coalesce(Coalesce(a, b), c)
    let stmts = parse_ok("a orelse b orelse c");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::Coalesce { left, right },
            ..
        }) => {
            assert!(matches!(left.node, Expression::Coalesce { .. }));
            assert!(matches!(right.node, Expression::Ident(_)));
        }
        other => panic!("expected Coalesce(Coalesce, Ident), got {other:?}"),
    }
}

#[test]
fn coalesce_is_looser_than_or() {
    // a or b orelse c → Coalesce(BinaryOp(a or b), c)
    let stmts = parse_ok("a or b orelse c");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::Coalesce { left, right },
            ..
        }) => {
            assert!(matches!(
                left.node,
                Expression::BinaryOp { op: BinOp::Or, .. }
            ));
            assert!(matches!(right.node, Expression::Ident(_)));
        }
        other => panic!("expected Coalesce(Or(..), Ident), got {other:?}"),
    }
}

#[test]
fn coalesce_is_looser_than_comparison() {
    // a == b orelse c → Coalesce(Equals(a, b), c)
    let stmts = parse_ok("a == b orelse c");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::Coalesce { left, .. },
            ..
        }) => {
            assert!(matches!(
                left.node,
                Expression::BinaryOp {
                    op: BinOp::Equals,
                    ..
                }
            ));
        }
        other => panic!("expected Coalesce with Equals on left, got {other:?}"),
    }
}

#[test]
fn coalesce_with_arithmetic() {
    // a + 1 orelse b * 2 → Coalesce(Add(a, 1), Mul(b, 2))
    let stmts = parse_ok("a + 1 orelse b * 2");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::Coalesce { left, right },
            ..
        }) => {
            assert!(matches!(
                left.node,
                Expression::BinaryOp { op: BinOp::Add, .. }
            ));
            assert!(matches!(
                right.node,
                Expression::BinaryOp { op: BinOp::Mul, .. }
            ));
        }
        other => panic!("expected Coalesce(Add, Mul), got {other:?}"),
    }
}

// -- if let --

#[test]
fn parses_if_let_statement() {
    let stmts = parse_ok("if let x = maybe { print(x) }");
    assert!(matches!(
        &stmts[0].node,
        Statement::IfLet { name, else_body: None, .. } if name == "x"
    ));
}

#[test]
fn parses_if_let_with_else() {
    let stmts = parse_ok("if let x = maybe { print(x) } else { print(0) }");
    match &stmts[0].node {
        Statement::IfLet {
            name,
            else_body: Some(_),
            ..
        } => {
            assert_eq!(name, "x");
        }
        other => panic!("expected IfLet with else, got {other:?}"),
    }
}

#[test]
fn if_let_does_not_break_regular_if() {
    // Regular `if` should still work as before
    let stmts = parse_ok("if x { print(1) }");
    assert!(matches!(&stmts[0].node, Statement::If { .. }));
}

#[test]
fn if_let_does_not_break_if_else() {
    let stmts = parse_ok("if x { print(1) } else { print(2) }");
    assert!(matches!(
        &stmts[0].node,
        Statement::If {
            else_body: Some(_),
            ..
        }
    ));
}

#[test]
fn if_let_does_not_break_elif() {
    let stmts = parse_ok("if x { 1 } elif y { 2 } else { 3 }");
    assert!(matches!(&stmts[0].node, Statement::If { .. }));
}

#[test]
fn parses_unless_statement() {
    let stmts = parse_ok("unless ready { print(0) }");
    assert!(matches!(
        &stmts[0].node,
        Statement::Unless {
            else_body: None,
            ..
        }
    ));
}

#[test]
fn parses_unless_with_else() {
    let stmts = parse_ok("unless ready { print(0) } else { print(1) }");
    assert!(matches!(
        &stmts[0].node,
        Statement::Unless {
            else_body: Some(_),
            ..
        }
    ));
}

// -- Test and Assert --

#[test]
fn parses_test_statement() {
    let stmts = parse_ok("test \"addition works\" { assert(1 == 1) }");
    match &stmts[0].node {
        Statement::Test { name, .. } => assert_eq!(name, "addition works"),
        other => panic!("expected Statement::Test, got {other:?}"),
    }
}

#[test]
fn parses_assert_statement() {
    let stmts = parse_ok("assert(1 == 1)");
    assert!(matches!(&stmts[0].node, Statement::Assert { .. }));
}

#[test]
fn test_block_contains_assert() {
    // Parse the body and confirm the assert lands inside it.
    let stmts = parse_ok("test \"ok\" { assert(true) }");
    let body = match &stmts[0].node {
        Statement::Test { body, .. } => body,
        other => panic!("expected Statement::Test, got {other:?}"),
    };
    let inner = match &body.node {
        Expression::Block(inner) => inner,
        other => panic!("expected block body, got {other:?}"),
    };
    assert!(matches!(&inner[0].node, Statement::Assert { .. }));
}

#[test]
fn test_requires_string_name() {
    // An identifier name isn't accepted; the rest of the file still
    // parses so we verify at least one parse error is raised.
    let errors = parse_errors("test oops { assert(true) }");
    assert!(!errors.is_empty());
}

#[test]
fn assert_requires_parentheses() {
    let errors = parse_errors("assert true");
    assert!(!errors.is_empty());
}

// -- Precedence edge cases --

#[test]
fn try_binds_tighter_than_coalesce() {
    // try a orelse b → Coalesce(Try(a), b)
    let stmts = parse_ok("try a orelse b");
    assert!(matches!(
        &stmts[0].node,
        Statement::Expression(Spanned {
            node: Expression::Coalesce { .. },
            ..
        })
    ));
}

#[test]
fn bang_binds_tighter_than_coalesce() {
    // !a orelse b → Coalesce(UnwrapError(a), b)
    let stmts = parse_ok("!a orelse b");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::Coalesce { left, .. },
            ..
        }) => {
            assert!(matches!(left.node, Expression::UnwrapError(_)));
        }
        other => panic!("expected Coalesce(UnwrapError, _), got {other:?}"),
    }
}

#[test]
fn not_equals_still_works() {
    // Ensure `!=` is not parsed as `!` followed by `=`
    let stmts = parse_ok("a != b");
    assert!(matches!(
        &stmts[0].node,
        Statement::Expression(Spanned {
            node: Expression::BinaryOp {
                op: BinOp::NotEquals,
                ..
            },
            ..
        })
    ));
}

#[test]
fn try_on_method_call() {
    let stmts = parse_ok("try o.method()");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::Try(inner),
            ..
        }) => {
            assert!(matches!(inner.node, Expression::MethodCall { .. }));
        }
        other => panic!("expected Try(MethodCall), got {other:?}"),
    }
}

#[test]
fn unwrap_error_on_method_call() {
    let stmts = parse_ok("!o.method()");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::UnwrapError(inner),
            ..
        }) => {
            assert!(matches!(inner.node, Expression::MethodCall { .. }));
        }
        other => panic!("expected UnwrapError(MethodCall), got {other:?}"),
    }
}

#[test]
fn if_let_with_call_expression() {
    let stmts = parse_ok("if let v = get_thing() { print(v) }");
    match &stmts[0].node {
        Statement::IfLet { name, value, .. } => {
            assert_eq!(name, "v");
            assert!(matches!(value.node, Expression::Call { .. }));
        }
        other => panic!("expected IfLet with call value, got {other:?}"),
    }
}

#[test]
fn if_let_with_coalesce_in_value() {
    let stmts = parse_ok("if let x = a orelse b { print(x) }");
    match &stmts[0].node {
        Statement::IfLet { value, .. } => {
            assert!(matches!(value.node, Expression::Coalesce { .. }));
        }
        other => panic!("expected IfLet with Coalesce value, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// List syntax: [T] types, [a, b, c] literals, xs[i] indexing, xs[i] = y
// ---------------------------------------------------------------------------

#[test]
fn parses_list_type_annotation() {
    let stmts = parse_ok("let xs: [int] = [1, 2, 3]");
    match &stmts[0].node {
        Statement::Let {
            type_ann: Some(TypeAnnotation::List(inner)),
            ..
        } => {
            assert!(matches!(inner.as_ref(), TypeAnnotation::Named(s) if s == &["int"]));
        }
        other => panic!("expected Let with List type, got {other:?}"),
    }
}

#[test]
fn parses_nested_list_type_annotation() {
    let stmts = parse_ok("let xs: [[int]] = [[1], [2, 3]]");
    match &stmts[0].node {
        Statement::Let {
            type_ann: Some(TypeAnnotation::List(inner)),
            ..
        } => {
            assert!(matches!(inner.as_ref(), TypeAnnotation::List(_)));
        }
        other => panic!("expected Let with nested List type, got {other:?}"),
    }
}

#[test]
fn parses_nillable_list_type() {
    let stmts = parse_ok("let xs: [int]? = nil");
    match &stmts[0].node {
        Statement::Let {
            type_ann: Some(TypeAnnotation::Nillable(inner)),
            ..
        } => {
            assert!(matches!(inner.as_ref(), TypeAnnotation::List(_)));
        }
        other => panic!("expected Let with Nillable(List), got {other:?}"),
    }
}

#[test]
fn parses_list_literal_expression() {
    let stmts = parse_ok("[1, 2, 3]");
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::ListLiteral(elements),
            ..
        }) => {
            assert_eq!(elements.len(), 3);
        }
        other => panic!("expected ListLiteral expression, got {other:?}"),
    }
}

#[test]
fn parses_empty_list_literal() {
    let stmts = parse_ok("let xs: [int] = []");
    match &stmts[0].node {
        Statement::Let { value, .. } => {
            assert!(matches!(value.node, Expression::ListLiteral(ref v) if v.is_empty()));
        }
        other => panic!("expected Let with empty ListLiteral, got {other:?}"),
    }
}

#[test]
fn parses_list_literal_with_trailing_comma() {
    let stmts = parse_ok("[1, 2, 3,]");
    assert!(matches!(
        &stmts[0].node,
        Statement::Expression(Spanned {
            node: Expression::ListLiteral(v),
            ..
        }) if v.len() == 3
    ));
}

#[test]
fn parses_multiline_list_literal() {
    let stmts = parse_ok("let xs: [int] = [\n1,\n2,\n3,\n]");
    match &stmts[0].node {
        Statement::Let { value, .. } => {
            assert!(matches!(value.node, Expression::ListLiteral(ref v) if v.len() == 3));
        }
        other => panic!("expected Let with multiline ListLiteral, got {other:?}"),
    }
}

#[test]
fn parses_index_expression() {
    let stmts = parse_ok("let xs: [int] = [1, 2, 3]\nxs[0]");
    assert_eq!(stmts.len(), 2);
    match &stmts[1].node {
        Statement::Expression(Spanned {
            node: Expression::Index { .. },
            ..
        }) => {}
        other => panic!("expected Index expression, got {other:?}"),
    }
}

#[test]
fn parses_chained_index_expression() {
    let stmts = parse_ok("let xs: [[int]] = [[1], [2]]\nxs[0][0]");
    // xs[0][0] should parse as Index(Index(Ident, 0), 0)
    match &stmts[1].node {
        Statement::Expression(Spanned {
            node: Expression::Index { object, .. },
            ..
        }) => {
            assert!(matches!(object.node, Expression::Index { .. }));
        }
        other => panic!("expected chained Index expression, got {other:?}"),
    }
}

#[test]
fn parses_index_assignment() {
    let stmts = parse_ok("let xs: [int] = [1, 2, 3]\nxs[0] = 42");
    match &stmts[1].node {
        Statement::IndexAssignment { .. } => {}
        other => panic!("expected IndexAssignment, got {other:?}"),
    }
}

#[test]
fn parses_field_index_assignment() {
    let stmts = parse_ok("box.xs[0] = 42");
    match &stmts[0].node {
        Statement::IndexAssignment { object, .. } => {
            assert!(matches!(object.node, Expression::FieldAccess { .. }));
        }
        other => panic!("expected IndexAssignment with field object, got {other:?}"),
    }
}

#[test]
fn parses_index_field_assignment() {
    let stmts = parse_ok("xs[0].value = 42");
    match &stmts[0].node {
        Statement::FieldAssignment { object, field, .. } => {
            assert_eq!(field, "value");
            assert!(matches!(object.node, Expression::Index { .. }));
        }
        other => panic!("expected FieldAssignment with index object, got {other:?}"),
    }
}

#[test]
fn parses_list_method_calls() {
    // len/push/pop all lower to method calls — the compiler decides
    // later which receiver-specific opcode to emit.
    let stmts = parse_ok("let xs: [int] = [1]\nxs.push(2)\nxs.len()\nxs.pop()");
    assert_eq!(stmts.len(), 4);
}

#[test]
fn parses_map_type_annotation() {
    let stmts = parse_ok(r#"let stats: {String: int} = {"hp": 10}"#);
    match &stmts[0].node {
        Statement::Let {
            type_ann: Some(TypeAnnotation::Map(key, value)),
            ..
        } => {
            assert!(matches!(key.as_ref(), TypeAnnotation::Named(s) if s == &["String"]));
            assert!(matches!(value.as_ref(), TypeAnnotation::Named(s) if s == &["int"]));
        }
        other => panic!("expected Let with Map type, got {other:?}"),
    }
}

#[test]
fn parses_map_literal_expression() {
    let stmts = parse_ok(r#"{"hp": 10, "mp": 4}"#);
    match &stmts[0].node {
        Statement::Expression(Spanned {
            node: Expression::MapLiteral(entries),
            ..
        }) => assert_eq!(entries.len(), 2),
        other => panic!("expected MapLiteral expression, got {other:?}"),
    }
}

#[test]
fn parses_empty_map_literal() {
    let stmts = parse_ok("let stats: {String: int} = {}");
    match &stmts[0].node {
        Statement::Let { value, .. } => {
            assert!(matches!(value.node, Expression::MapLiteral(ref v) if v.is_empty()));
        }
        other => panic!("expected Let with empty MapLiteral, got {other:?}"),
    }
}

#[test]
fn parses_nested_field_assignment() {
    let stmts = parse_ok("holder.counter.count = 5");
    match &stmts[0].node {
        Statement::FieldAssignment { object, field, .. } => {
            assert_eq!(field, "count");
            assert!(matches!(object.node, Expression::FieldAccess { .. }));
        }
        other => panic!("expected nested FieldAssignment, got {other:?}"),
    }
}
