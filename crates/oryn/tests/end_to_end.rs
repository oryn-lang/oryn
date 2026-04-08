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

// Note: with slot-based locals, `foo = 3` without a prior `let` is
// silently allowed (the compiler assigns a slot). This will become a
// compile-time error when we add proper variable resolution.
// TODO: add compile-time "undefined variable" error.

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

// --- Functions ---

#[test]
fn simple_function() {
    assert_eq!(run("fn greet() {\nprint(42)\n}\ngreet()"), "42\n",);
}

#[test]
fn function_with_params() {
    assert_eq!(run("fn add(a, b) {\nrn a + b\n}\nprint(add(3, 4))"), "7\n",);
}

#[test]
fn function_return_value() {
    assert_eq!(
        run("fn double(x) {\nrn x * 2\n}\nlet y = double(5)\nprint(y)"),
        "10\n",
    );
}

#[test]
fn function_implicit_return() {
    // A function without rn returns 0 by default.
    assert_eq!(run("fn noop() {\nlet x = 1\n}\nprint(noop())"), "0\n",);
}

#[test]
fn function_with_locals() {
    // Variables inside a function are local to that frame.
    assert_eq!(
        run("let x = 1\nfn bump() {\nlet x = 99\nrn x\n}\nprint(bump())\nprint(x)"),
        "99\n1\n",
    );
}

#[test]
fn recursive_function() {
    // Classic: factorial.
    assert_eq!(
        run("fn fact(n) {\nif n <= 1 { rn 1 }\nrn n * fact(n - 1)\n}\nprint(fact(5))"),
        "120\n",
    );
}

#[test]
fn fibonacci() {
    assert_eq!(
        run("fn fib(n) {\nif n <= 1 { rn n }\nrn fib(n - 1) + fib(n - 2)\n}\nprint(fib(10))"),
        "55\n",
    );
}

// --- Strings ---

#[test]
fn string_literal() {
    assert_eq!(run("print(\"hello\")"), "hello\n");
}

#[test]
fn string_in_variable() {
    assert_eq!(run("let name = \"world\"\nprint(name)"), "world\n");
}

#[test]
fn string_with_spaces_and_punctuation() {
    assert_eq!(run("print(\"hello, world!\")"), "hello, world!\n",);
}

#[test]
fn string_equality() {
    assert_eq!(run("print(\"abc\" == \"abc\")"), "true\n");
    assert_eq!(run("print(\"abc\" == \"def\")"), "false\n");
}

#[test]
fn string_as_function_param() {
    assert_eq!(
        run("fn greet(name) {\nprint(name)\n}\ngreet(\"alice\")"),
        "alice\n",
    );
}

// --- Errors ---

#[test]
fn arity_mismatch_is_runtime_error() {
    let chunk = oryn::Chunk::compile("fn add(a, b) {\nrn a + b\n}\nadd(1)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();

    assert!(matches!(
        err,
        oryn::RuntimeError::ArityMismatch {
            expected: 2,
            actual: 1,
            ..
        }
    ));
}

// --- Floats ---

#[test]
fn float_literal() {
    assert_eq!(run("print(3.14)"), "3.14\n");
}

#[test]
fn float_binding() {
    assert_eq!(run("let x = 1.5\nprint(x)"), "1.5\n");
}

#[test]
fn float_addition() {
    assert_eq!(run("print(1.5 + 2.5)"), "4.0\n");
}

#[test]
fn float_subtraction() {
    assert_eq!(run("print(10.5 - 3.5)"), "7.0\n");
}

#[test]
fn float_multiplication() {
    assert_eq!(run("print(2.5 * 4.0)"), "10.0\n");
}

#[test]
fn float_division() {
    assert_eq!(run("print(7.5 / 2.5)"), "3.0\n");
}

#[test]
fn float_comparison() {
    assert_eq!(run("print(3.14 > 2.71)"), "true\n");
    assert_eq!(run("print(1.0 == 1.0)"), "true\n");
    assert_eq!(run("print(1.0 != 2.0)"), "true\n");
    assert_eq!(run("print(1.5 < 2.5)"), "true\n");
    assert_eq!(run("print(3.0 <= 3.0)"), "true\n");
    assert_eq!(run("print(2.0 >= 3.0)"), "false\n");
}

#[test]
fn float_in_function() {
    assert_eq!(
        run("fn half(x) {\nrn x / 2.0\n}\nprint(half(5.0))"),
        "2.5\n",
    );
}

#[test]
fn float_precedence() {
    assert_eq!(run("print(1.0 + 2.0 * 3.0)"), "7.0\n");
}

#[test]
fn mixed_int_float_is_runtime_error() {
    let chunk = oryn::Chunk::compile("print(1 + 1.5)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    assert!(vm.run_with_writer(&chunk, &mut output).is_err());
}

// --- Val bindings ---

#[test]
fn val_binding() {
    assert_eq!(run("val x = 42\nprint(x)"), "42\n");
}

#[test]
fn val_binding_with_expression() {
    assert_eq!(run("val x = 1 + 2\nprint(x)"), "3\n");
}

#[test]
fn val_reassignment_is_compile_error() {
    let result = oryn::Chunk::compile("val x = 1\nx = 2");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, oryn::OrynError::Compiler { .. }))
    );
}

#[test]
fn let_reassignment_still_works() {
    assert_eq!(run("let x = 1\nx = 2\nprint(x)"), "2\n");
}

#[test]
fn val_in_function() {
    assert_eq!(
        run("fn double(n) {\nval result = n * 2\nrn result\n}\nprint(double(5))"),
        "10\n",
    );
}

#[test]
fn val_reassignment_in_function_is_compile_error() {
    let result = oryn::Chunk::compile("fn bad() {\nval x = 1\nx = 2\n}");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, oryn::OrynError::Compiler { .. }))
    );
}
