/// Compiles and runs an Oryn source string, capturing printed output.
fn run(source: &str) -> String {
    let chunk = oryn::Chunk::compile(source).expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    vm.run_with_writer(&chunk, &mut output)
        .expect("runtime error");

    String::from_utf8(output).expect("invalid utf-8")
}

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

#[test]
fn assignment_to_undefined_variable_is_runtime_error() {
    let chunk = oryn::Chunk::compile("foo = 3").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();

    assert!(matches!(
        err,
        oryn::RuntimeError::UndefinedVariable { ref name, .. } if name == "foo"
    ));
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
    // not binds tighter than and/or: `not true or true` → `(not true) or true` → true
    assert_eq!(run("print(not true or true)"), "true\n");
    // and binds tighter than or: `false or true and true` → `false or (true and true)` → true
    assert_eq!(run("print(false or true and true)"), "true\n");
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
