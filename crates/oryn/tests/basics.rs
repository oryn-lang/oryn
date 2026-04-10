mod common;
use common::run;

// --- Variables ---

#[test]
fn let_binding() {
    assert_eq!(run("let x = 42\nprint(x)"), "42\n");
}

// --- Assignment ---

#[test]
fn assignment() {
    assert_eq!(run("let x = 5\nx = 10\nprint(x)"), "10\n");
}

#[test]
fn assignment_with_expression() {
    assert_eq!(run("let x = 5\nx = x + 1\nprint(x)"), "6\n");
}

// --- Arithmetic ---

#[test]
fn addition() {
    assert_eq!(run("print(2 + 3)"), "5\n");
}

#[test]
fn subtraction() {
    assert_eq!(run("print(10 - 4)"), "6\n");
}

#[test]
fn multiplication() {
    assert_eq!(run("print(3 * 7)"), "21\n");
}

#[test]
fn division() {
    assert_eq!(run("print(20 / 5)"), "4\n");
}

#[test]
fn precedence() {
    assert_eq!(run("print(2 + 3 * 4)"), "14\n");
}

#[test]
fn parentheses() {
    assert_eq!(run("print((2 + 3) * 4)"), "20\n");
}

// --- Comparisons ---

#[test]
fn equals() {
    assert_eq!(run("print(5 == 5)"), "true\n");
    assert_eq!(run("print(5 == 3)"), "false\n");
}

#[test]
fn not_equals() {
    assert_eq!(run("print(5 != 3)"), "true\n");
    assert_eq!(run("print(5 != 5)"), "false\n");
}

#[test]
fn less_than() {
    assert_eq!(run("print(3 < 5)"), "true\n");
    assert_eq!(run("print(5 < 3)"), "false\n");
}

#[test]
fn greater_than() {
    assert_eq!(run("print(5 > 3)"), "true\n");
    assert_eq!(run("print(3 > 5)"), "false\n");
}

#[test]
fn less_than_equals() {
    assert_eq!(run("print(3 <= 5)"), "true\n");
    assert_eq!(run("print(5 <= 5)"), "true\n");
    assert_eq!(run("print(6 <= 5)"), "false\n");
}

#[test]
fn greater_than_equals() {
    assert_eq!(run("print(5 >= 3)"), "true\n");
    assert_eq!(run("print(5 >= 5)"), "true\n");
    assert_eq!(run("print(4 >= 5)"), "false\n");
}

// --- Logical operators ---

#[test]
fn and_operator() {
    assert_eq!(run("print(true and true)"), "true\n");
    assert_eq!(run("print(true and false)"), "false\n");
}

#[test]
fn or_operator() {
    assert_eq!(run("print(false or true)"), "true\n");
    assert_eq!(run("print(false or false)"), "false\n");
}

#[test]
fn not_operator() {
    assert_eq!(run("print(not true)"), "false\n");
    assert_eq!(run("print(not false)"), "true\n");
}

#[test]
fn logical_precedence() {
    // not binds tighter than and/or: `not true or true` -> `(not true) or true` -> true
    assert_eq!(run("print(not true or true)"), "true\n");
    // and binds tighter than or: `false or true and true` -> `false or (true and true)` -> true
    assert_eq!(run("print(false or true and true)"), "true\n");
}

#[test]
fn and_short_circuits_on_false() {
    // Without short-circuit this would divide by zero at runtime. The RHS
    // must be skipped when the LHS is false.
    assert_eq!(run("print(false and (1 / 0 == 0))"), "false\n");
}

#[test]
fn or_short_circuits_on_true() {
    // Without short-circuit this would divide by zero at runtime. The RHS
    // must be skipped when the LHS is true.
    assert_eq!(run("print(true or (1 / 0 == 0))"), "true\n");
}

#[test]
fn and_evaluates_rhs_when_lhs_true() {
    assert_eq!(run("print(true and (1 == 1))"), "true\n");
    assert_eq!(run("print(true and (1 == 2))"), "false\n");
}

#[test]
fn or_evaluates_rhs_when_lhs_false() {
    assert_eq!(run("print(false or (1 == 1))"), "true\n");
    assert_eq!(run("print(false or (1 == 2))"), "false\n");
}

// --- Mixed expressions ---

#[test]
fn comparison_with_arithmetic() {
    assert_eq!(run("print(2 + 3 >= 5)"), "true\n");
}

#[test]
fn logical_with_comparison() {
    assert_eq!(run("print(true and 5 > 3)"), "true\n");
}

#[test]
fn multiple_statements() {
    assert_eq!(run("let x = 5\nlet y = 10\nprint(x + y)"), "15\n",);
}

// --- Shadowing ---

#[test]
fn shadowing_same_type() {
    assert_eq!(run("let x = 5\nlet x = 10\nprint(x)"), "10\n");
}

#[test]
fn shadowing_different_type() {
    assert_eq!(run("let x = 5\nlet x = \"hello\"\nprint(x)"), "hello\n");
}

#[test]
fn shadowing_preserves_old_value_before_shadow() {
    assert_eq!(
        run("let x = 5\nprint(x)\nlet x = \"hello\"\nprint(x)"),
        "5\nhello\n",
    );
}

#[test]
fn shadowing_multiple_times() {
    assert_eq!(
        run("let x = 1\nlet x = true\nlet x = \"done\"\nprint(x)"),
        "done\n",
    );
}
