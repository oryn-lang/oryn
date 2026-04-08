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

// --- Compiler hardening ---

#[test]
fn undefined_variable_is_compile_error() {
    let result = oryn::Chunk::compile("print(typo)");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined variable"))
    }));
}

#[test]
fn assignment_to_undefined_is_compile_error() {
    let result = oryn::Chunk::compile("x = 5");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined variable"))
    }));
}

#[test]
fn and_with_non_bool_is_type_error() {
    let chunk = oryn::Chunk::compile("print(5 and true)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::TypeError { .. }));
}

#[test]
fn or_with_non_bool_is_type_error() {
    let chunk = oryn::Chunk::compile("print(true or 5)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::TypeError { .. }));
}

#[test]
fn mixed_type_arithmetic_reports_real_types() {
    let chunk = oryn::Chunk::compile("print(true + 1)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(
        err,
        oryn::RuntimeError::TypeMismatch { op: "+", .. }
    ));
}

#[test]
fn integer_division_by_zero_is_error() {
    let chunk = oryn::Chunk::compile("print(1 / 0)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::DivisionByZero { .. }));
}

#[test]
fn float_division_by_zero_is_error() {
    let chunk = oryn::Chunk::compile("print(1.0 / 0.0)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::DivisionByZero { .. }));
}

// --- Type annotations (parsed, not enforced) ---

#[test]
fn let_with_type_annotation() {
    assert_eq!(run("let x: i32 = 5\nprint(x)"), "5\n");
}

#[test]
fn val_with_type_annotation() {
    assert_eq!(run("val x: f32 = 3.14\nprint(x)"), "3.14\n");
}

#[test]
fn function_with_param_types() {
    assert_eq!(
        run("fn add(a: i32, b: i32) {\nrn a + b\n}\nprint(add(2, 3))"),
        "5\n",
    );
}

#[test]
fn function_with_return_type() {
    assert_eq!(
        run("fn double(x: i32) -> i32 {\nrn x * 2\n}\nprint(double(5))"),
        "10\n",
    );
}

#[test]
fn mixed_annotated_and_unannotated() {
    assert_eq!(run("let x: i32 = 10\nlet y = 20\nprint(x + y)"), "30\n",);
}

#[test]
fn function_with_some_typed_params() {
    assert_eq!(
        run("fn add(a: i32, b) {\nrn a + b\n}\nprint(add(2, 3))"),
        "5\n",
    );
}

// --- Objects ---

#[test]
fn object_definition_and_instantiation() {
    assert_eq!(
        run("obj Vec2 {\nx: i32\ny: i32\n}\nlet v = Vec2 { x: 1, y: 2 }\nprint(v.x)"),
        "1\n",
    );
}

#[test]
fn object_field_read_second_field() {
    assert_eq!(
        run("obj Vec2 {\nx: i32\ny: i32\n}\nlet v = Vec2 { x: 1, y: 2 }\nprint(v.y)"),
        "2\n",
    );
}

#[test]
fn object_field_mutation() {
    assert_eq!(
        run("obj Vec2 {\nx: i32\ny: i32\n}\nlet v = Vec2 { x: 1, y: 2 }\nv.x = 99\nprint(v.x)"),
        "99\n",
    );
}

#[test]
fn object_reference_aliasing() {
    assert_eq!(
        run(
            "obj Vec2 {\nx: i32\ny: i32\n}\nlet v = Vec2 { x: 1, y: 2 }\nlet w = v\nw.y = 50\nprint(v.y)"
        ),
        "50\n",
    );
}

#[test]
fn object_fields_out_of_order() {
    assert_eq!(
        run("obj Vec2 {\nx: i32\ny: i32\n}\nlet v = Vec2 { y: 20, x: 10 }\nprint(v.x)\nprint(v.y)"),
        "10\n20\n",
    );
}

#[test]
fn object_print_shows_instance() {
    assert_eq!(
        run("obj Foo {\nx: i32\n}\nlet f = Foo { x: 1 }\nprint(f)"),
        "<Foo instance>\n",
    );
}

#[test]
fn val_prevents_field_mutation() {
    let result = oryn::Chunk::compile("obj Foo {\nx: i32\n}\nval f = Foo { x: 1 }\nf.x = 2");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("val"))
    }));
}

#[test]
fn undefined_type_is_compile_error() {
    let result = oryn::Chunk::compile("let f = Unknown { x: 1 }");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined type"))
    }));
}

#[test]
fn unknown_field_is_compile_error() {
    let result = oryn::Chunk::compile("obj Foo {\nx: i32\n}\nlet f = Foo { x: 1, z: 2 }");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("unknown field"))
    }));
}

#[test]
fn missing_field_is_compile_error() {
    let result = oryn::Chunk::compile("obj Foo {\nx: i32\ny: i32\n}\nlet f = Foo { x: 1 }");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("missing field"))
    }));
}

#[test]
fn object_inline_definition() {
    assert_eq!(
        run("obj Vec2 { x: i32, y: i32 }\nlet v = Vec2 { x: 1, y: 2 }\nprint(v.x)"),
        "1\n",
    );
}

#[test]
fn object_with_float_fields() {
    assert_eq!(
        run("obj Point {\nx: f32\ny: f32\n}\nlet p = Point { x: 3.14, y: 2.71 }\nprint(p.x)"),
        "3.14\n",
    );
}

#[test]
fn object_in_function() {
    assert_eq!(
        run(
            "obj Vec2 {\nx: i32\ny: i32\n}\nfn get_x(v: Vec2) {\nrn v.x\n}\nlet v = Vec2 { x: 42, y: 0 }\nprint(get_x(v))"
        ),
        "42\n",
    );
}

// --- Unary minus ---

#[test]
fn negate_int() {
    assert_eq!(run("print(-5)"), "-5\n");
}

#[test]
fn negate_float() {
    assert_eq!(run("print(-3.14)"), "-3.14\n");
}

#[test]
fn negate_variable() {
    assert_eq!(run("let x = 10\nprint(-x)"), "-10\n");
}

#[test]
fn negate_in_expression() {
    assert_eq!(run("print(-2 * 3)"), "-6\n");
}

#[test]
fn double_negate() {
    assert_eq!(run("print(--5)"), "5\n");
}

#[test]
fn negate_precedence() {
    assert_eq!(run("print(-2 + 3)"), "1\n");
}

// --- Methods ---

#[test]
fn method_no_params() {
    assert_eq!(
        run(
            "obj Vec2 {\nx: i32\ny: i32\nfn sum(self) {\nrn self.x + self.y\n}\n}\nlet v = Vec2 { x: 3, y: 4 }\nprint(v.sum())"
        ),
        "7\n",
    );
}

#[test]
fn method_with_params() {
    assert_eq!(
        run(
            "obj Counter {\ncount: i32\nfn add(self, n: i32) {\nrn self.count + n\n}\n}\nlet c = Counter { count: 10 }\nprint(c.add(5))"
        ),
        "15\n",
    );
}

#[test]
fn method_mutates_field() {
    assert_eq!(
        run(
            "obj Counter {\ncount: i32\nfn inc(self) {\nself.count = self.count + 1\n}\n}\nlet c = Counter { count: 0 }\nc.inc()\nprint(c.count)"
        ),
        "1\n",
    );
}

#[test]
fn method_on_val_binding() {
    // Methods should still work on val bindings (calling doesn't reassign).
    assert_eq!(
        run(
            "obj Vec2 {\nx: i32\ny: i32\nfn sum(self) {\nrn self.x + self.y\n}\n}\nval v = Vec2 { x: 1, y: 2 }\nprint(v.sum())"
        ),
        "3\n",
    );
}

#[test]
fn method_with_float_fields() {
    assert_eq!(
        run(
            "obj Circle {\nradius: f32\nfn area(self) {\nrn self.radius * self.radius * 3.14\n}\n}\nlet c = Circle { radius: 2.0 }\nprint(c.area())"
        ),
        "12.56\n",
    );
}

#[test]
fn multiple_methods() {
    assert_eq!(
        run(
            "obj Vec2 {\nx: i32\ny: i32\nfn get_x(self) {\nrn self.x\n}\nfn get_y(self) {\nrn self.y\n}\n}\nlet v = Vec2 { x: 10, y: 20 }\nprint(v.get_x())\nprint(v.get_y())"
        ),
        "10\n20\n",
    );
}

#[test]
fn undefined_method_is_runtime_error() {
    let chunk = oryn::Chunk::compile("obj Foo {\nx: i32\n}\nlet f = Foo { x: 1 }\nf.nope()")
        .expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::UndefinedFunction { .. }));
}

// --- Use composition ---

#[test]
fn use_inherits_fields() {
    assert_eq!(
        run(
            "obj Health { hp: i32 }\nobj Player {\nuse Health\nname: String\n}\nlet p = Player { hp: 100, name: \"Alice\" }\nprint(p.hp)"
        ),
        "100\n",
    );
}

#[test]
fn use_inherits_methods() {
    assert_eq!(
        run(
            "obj Health {\nhp: i32\nfn heal(self, amount: i32) {\nself.hp = self.hp + amount\n}\n}\nobj Player {\nuse Health\nname: String\n}\nlet p = Player { hp: 50, name: \"Bob\" }\np.heal(20)\nprint(p.hp)"
        ),
        "70\n",
    );
}

#[test]
fn use_multiple_types() {
    assert_eq!(
        run(
            "obj Health { hp: i32 }\nobj Named { name: String }\nobj Player {\nuse Health\nuse Named\n}\nlet p = Player { hp: 100, name: \"Alice\" }\nprint(p.hp)\nprint(p.name)"
        ),
        "100\nAlice\n",
    );
}

#[test]
fn use_field_conflict_is_compile_error() {
    let result =
        oryn::Chunk::compile("obj A { x: i32 }\nobj B { x: i32 }\nobj C {\nuse A\nuse B\n}");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("conflicts"))
    }));
}

#[test]
fn use_undefined_type_is_compile_error() {
    let result = oryn::Chunk::compile("obj Foo {\nuse Nonexistent\n}");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined type"))
    }));
}

#[test]
fn use_own_fields_after_composed() {
    assert_eq!(
        run(
            "obj Position { x: i32, y: i32 }\nobj Entity {\nuse Position\nname: String\n}\nlet e = Entity { x: 5, y: 10, name: \"thing\" }\nprint(e.x)\nprint(e.name)"
        ),
        "5\nthing\n",
    );
}
