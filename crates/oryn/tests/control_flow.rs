mod common;
use common::run;

// --- Nil and error runtime behavior ---

#[test]
fn nil_binding_prints_nil() {
    assert_eq!(run("let x: maybe int = nil\nprint(x)"), "nil\n");
}

#[test]
fn nillable_binding_with_value_prints_value() {
    assert_eq!(run("let x: maybe int = 5\nprint(x)"), "5\n");
}

#[test]
fn coalesce_returns_value_when_not_nil() {
    assert_eq!(
        run("let x: maybe int = 5\nlet y = x orelse 0\nprint(y)"),
        "5\n"
    );
}

#[test]
fn coalesce_returns_fallback_when_nil() {
    assert_eq!(
        run("let x: maybe int = nil\nlet y = x orelse 42\nprint(y)"),
        "42\n"
    );
}

#[test]
fn coalesce_nested_fallback() {
    // Chaining orelse is left-associative: (a orelse b) returns int, so a second
    // orelse on the result would not type-check. Use if-let or nested
    // coalesce with separate bindings instead.
    assert_eq!(
        run(
            "let a: maybe int = nil\nlet b: maybe int = 7\nlet y = a orelse (b orelse 0)\nprint(y)"
        ),
        "7\n"
    );
}

#[test]
fn coalesce_nested_all_nil() {
    assert_eq!(
        run(
            "let a: maybe int = nil\nlet b: maybe int = nil\nlet y = a orelse (b orelse 99)\nprint(y)"
        ),
        "99\n"
    );
}

#[test]
fn if_let_runs_body_when_not_nil() {
    assert_eq!(
        run("let x: maybe int = 10\nif let v = x {\nprint(v)\n}"),
        "10\n"
    );
}

#[test]
fn if_let_skips_body_when_nil() {
    assert_eq!(
        run("let x: maybe int = nil\nif let v = x {\nprint(v)\n}\nprint(0)"),
        "0\n"
    );
}

#[test]
fn if_let_else_runs_else_when_nil() {
    assert_eq!(
        run("let x: maybe int = nil\nif let v = x {\nprint(v)\n} else {\nprint(99)\n}"),
        "99\n"
    );
}

#[test]
fn if_let_else_runs_body_when_not_nil() {
    assert_eq!(
        run("let x: maybe int = 7\nif let v = x {\nprint(v)\n} else {\nprint(99)\n}"),
        "7\n"
    );
}

#[test]
fn nil_equality() {
    assert_eq!(run("let x: maybe int = nil\nprint(x == nil)"), "true\n");
}

#[test]
fn non_nil_not_equal_to_nil() {
    assert_eq!(run("let x: maybe int = 5\nprint(x == nil)"), "false\n");
}

#[test]
fn nil_not_equals() {
    assert_eq!(run("let x: maybe int = nil\nprint(x != nil)"), "false\n");
    assert_eq!(run("let x: maybe int = 5\nprint(x != nil)"), "true\n");
}

// --- If / else / elif ---

#[test]
fn if_true_runs_body() {
    assert_eq!(run("if true { print(1) }"), "1\n");
}

#[test]
fn if_false_skips_body() {
    assert_eq!(run("if false { print(1) }\nprint(2)"), "2\n");
}

#[test]
fn if_else_takes_else_when_false() {
    assert_eq!(run("if false { print(1) } else { print(2) }"), "2\n");
}

#[test]
fn if_else_takes_if_when_true() {
    assert_eq!(run("if true { print(1) } else { print(2) }"), "1\n");
}

#[test]
fn elif_takes_first_true_branch() {
    assert_eq!(
        run("if false { print(1) } elif true { print(2) } else { print(3) }"),
        "2\n",
    );
}

#[test]
fn elif_falls_through_to_else() {
    assert_eq!(
        run("if false { print(1) } elif false { print(2) } else { print(3) }"),
        "3\n",
    );
}

#[test]
fn if_with_condition_expression() {
    assert_eq!(
        run("let x = 5\nif x > 3 { print(1) } else { print(2) }"),
        "1\n"
    );
}

#[test]
fn if_block_runs_multiple_statements() {
    assert_eq!(
        run("let x = 1\nif true {\nx = x + 1\nx = x + 1\nprint(x)\n}"),
        "3\n",
    );
}

#[test]
fn else_block_runs_multiple_statements() {
    assert_eq!(
        run("let x = 1\nif false {\nprint(0)\n} else {\nx = x + 10\nx = x + 5\nprint(x)\n}"),
        "16\n",
    );
}

#[test]
fn elif_block_runs_multiple_statements() {
    assert_eq!(
        run(
            "let x = 0\nif false {\nprint(1)\n} elif true {\nx = x + 7\nx = x + 3\nprint(x)\n} else {\nprint(99)\n}"
        ),
        "10\n",
    );
}

#[test]
fn elif_chain() {
    assert_eq!(
        run(
            "let x = 3\nif x == 1 { print(1) } elif x == 2 { print(2) } elif x == 3 { print(3) } else { print(4) }"
        ),
        "3\n",
    );
}

#[test]
fn block_scope_does_not_escape() {
    let result = oryn::Chunk::compile("{\nlet x = 1\n}\nprint(x)");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined variable"))
    }));
}

#[test]
fn block_scope_restores_shadowed_binding() {
    assert_eq!(
        run("let x = 7\n{\nlet x = 9\nprint(x)\n}\nprint(x)"),
        "9\n7\n"
    );
}

#[test]
fn if_scope_does_not_escape() {
    let result = oryn::Chunk::compile("if true {\nlet x = 1\n}\nprint(x)");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined variable"))
    }));
}

#[test]
fn unless_false_runs_body() {
    assert_eq!(run("if not false { print(1) }"), "1\n");
}

#[test]
fn unless_true_skips_body() {
    assert_eq!(run("if not true { print(1) }\nprint(2)"), "2\n");
}

#[test]
fn unless_else_takes_else_when_true() {
    assert_eq!(run("if not true { print(1) } else { print(2) }"), "2\n");
}

#[test]
fn unless_else_takes_body_when_false() {
    assert_eq!(run("if not false { print(1) } else { print(2) }"), "1\n");
}

// --- While loops ---

#[test]
fn while_loop() {
    assert_eq!(
        run("let x = 0\nwhile x < 3 {\nx = x + 1\n}\nprint(x)"),
        "3\n",
    );
}

#[test]
fn while_false_skips_body() {
    assert_eq!(run("while false {\nprint(1)\n}\nprint(2)"), "2\n");
}

#[test]
fn while_with_print_each_iteration() {
    assert_eq!(
        run("let i = 0\nwhile i < 3 {\nprint(i)\ni = i + 1\n}"),
        "0\n1\n2\n",
    );
}

#[test]
fn break_exits_loop() {
    assert_eq!(
        run("let i = 0\nwhile true {\nif i == 3 { break }\ni = i + 1\n}\nprint(i)"),
        "3\n",
    );
}

#[test]
fn continue_skips_rest_of_body() {
    // Print only odd numbers: skip even ones with continue.
    assert_eq!(
        run(
            "let i = 0\nwhile i < 5 {\ni = i + 1\nif i == 2 { continue }\nif i == 4 { continue }\nprint(i)\n}"
        ),
        "1\n3\n5\n",
    );
}

#[test]
fn break_in_nested_if() {
    // break inside an if inside a while should exit the while.
    assert_eq!(
        run("let x = 0\nwhile true {\nx = x + 1\nif x > 5 {\nbreak\n}\n}\nprint(x)"),
        "6\n",
    );
}

#[test]
fn while_scope_does_not_escape() {
    let result = oryn::Chunk::compile("while false {\nlet x = 1\n}\nprint(x)");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined variable"))
    }));
}

#[test]
fn for_loop_iterates_half_open_range() {
    assert_eq!(run("for i in 0..3 {\nprint(i)\n}"), "0\n1\n2\n");
}

#[test]
fn for_loop_iterates_inclusive_range() {
    assert_eq!(run("for i in 2..=5 {\nprint(i)\n}"), "2\n3\n4\n5\n");
}

#[test]
fn for_loop_can_accumulate() {
    assert_eq!(
        run("let total = 0\nfor i in 1..4 {\ntotal = total + i\n}\nprint(total)"),
        "6\n",
    );
}

#[test]
fn for_continue_skips_rest_of_iteration() {
    assert_eq!(
        run("for i in 0..5 {\nif i == 1 { continue }\nif i == 3 { continue }\nprint(i)\n}"),
        "0\n2\n4\n",
    );
}

#[test]
fn for_break_exits_loop() {
    assert_eq!(
        run("for i in 0..5 {\nif i == 3 { break }\nprint(i)\n}"),
        "0\n1\n2\n",
    );
}

#[test]
fn for_loop_variable_does_not_escape() {
    let result = oryn::Chunk::compile("for i in 0..1 {\nprint(i)\n}\nprint(i)");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined variable"))
    }));
}

#[test]
fn for_loop_restores_shadowed_outer_binding() {
    assert_eq!(
        run("let i = 99\nfor i in 0..2 {\nprint(i)\n}\nprint(i)"),
        "0\n1\n99\n"
    );
}
