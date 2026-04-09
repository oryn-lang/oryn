mod common;
use common::run;

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
        run("fn greet(name: String) {\nprint(name)\n}\ngreet(\"alice\")"),
        "alice\n",
    );
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
        run("fn half(x: f32) -> f32 {\nrn x / 2.0\n}\nprint(half(5.0))"),
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
        run("fn double(n: i32) -> i32 {\nval result = n * 2\nrn result\n}\nprint(double(5))"),
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
        run("fn add(a: i32, b: i32) -> i32 {\nrn a + b\n}\nprint(add(2, 3))"),
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
fn function_missing_param_type_is_compile_error() {
    let result = oryn::Chunk::compile("fn add(a: i32, b) -> i32 {\nrn a + b\n}");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("requires a type"))
    }));
}

// --- Type checking ---

#[test]
fn unknown_type_annotation_is_compile_error() {
    let result = oryn::Chunk::compile("let x: banana = 5");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined type"))
    }));
}

#[test]
fn let_type_mismatch_is_compile_error() {
    let result = oryn::Chunk::compile("let x: i32 = \"hello\"");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("type mismatch"))
    }));
}

#[test]
fn val_type_mismatch_is_compile_error() {
    let result = oryn::Chunk::compile("val x: f32 = 5");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("type mismatch"))
    }));
}

#[test]
fn assignment_type_mismatch_is_compile_error() {
    let result = oryn::Chunk::compile("let x = 5\nx = \"hello\"");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("type mismatch"))
    }));
}

#[test]
fn void_function_no_return_type_needed() {
    // Functions without -> ReturnType are void.
    assert_eq!(
        run("fn greet(name: String) {\nprint(name)\n}\ngreet(\"world\")"),
        "world\n",
    );
}

#[test]
fn correct_type_annotations_pass() {
    assert_eq!(run("let x: i32 = 5\nprint(x)"), "5\n");
    assert_eq!(run("let x: f32 = 3.14\nprint(x)"), "3.14\n");
    assert_eq!(run("let x: bool = true\nprint(x)"), "true\n");
    assert_eq!(run("let x: String = \"hi\"\nprint(x)"), "hi\n");
}

#[test]
fn inferred_types_work_without_annotations() {
    assert_eq!(run("let x = 5\nlet y = 10\nprint(x + y)"), "15\n");
    assert_eq!(run("let x = 3.14\nprint(x)"), "3.14\n");
    assert_eq!(run("let x = true\nprint(x)"), "true\n");
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
