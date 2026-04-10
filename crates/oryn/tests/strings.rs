mod common;
use common::run;

// --- Plain strings (no interpolation) ---

#[test]
fn plain_string() {
    assert_eq!(run(r#"print("hello")"#), "hello\n");
}

#[test]
fn plain_string_with_spaces() {
    assert_eq!(run(r#"print("hello world")"#), "hello world\n");
}

// --- String interpolation ---

#[test]
fn interpolation_variable() {
    assert_eq!(
        run(r#"let name = "world"
print("hello {name}")"#),
        "hello world\n",
    );
}

#[test]
fn interpolation_expression() {
    assert_eq!(run(r#"print("{1 + 2}")"#), "3\n");
}

#[test]
fn interpolation_multiple() {
    assert_eq!(
        run(r#"let a = "foo"
let b = "bar"
print("{a} and {b}")"#),
        "foo and bar\n",
    );
}

#[test]
fn interpolation_int_to_string() {
    assert_eq!(
        run(r#"let x = 42
print("value: {x}")"#),
        "value: 42\n"
    );
}

#[test]
fn interpolation_float_to_string() {
    assert_eq!(
        run(r#"let x = 3.14
print("pi: {x}")"#),
        "pi: 3.14\n"
    );
}

#[test]
fn interpolation_bool_to_string() {
    assert_eq!(run(r#"print("alive: {true}")"#), "alive: true\n");
}

#[test]
fn interpolation_complex_expression() {
    assert_eq!(
        run(r#"let x = 10
print("{x} + {x} = {x + x}")"#),
        "10 + 10 = 20\n",
    );
}

// --- Escape sequences ---

#[test]
fn escaped_braces() {
    assert_eq!(
        run(r#"print("literal \{ braces \}")"#),
        "literal { braces }\n"
    );
}

#[test]
fn escaped_backslash() {
    assert_eq!(run(r#"print("back\\slash")"#), "back\\slash\n");
}

#[test]
fn mixed_escape_and_interpolation() {
    assert_eq!(
        run(r#"let x = 42
print("{x} and \{not this\}")"#),
        "42 and {not this}\n",
    );
}

// --- Edge cases ---

#[test]
fn interpolation_at_start() {
    assert_eq!(
        run(r#"let x = "hi"
print("{x}!")"#),
        "hi!\n"
    );
}

#[test]
fn interpolation_at_end() {
    assert_eq!(
        run(r#"let x = "end"
print("the {x}")"#),
        "the end\n"
    );
}

#[test]
fn empty_string() {
    assert_eq!(run(r#"print("")"#), "\n");
}

#[test]
fn constant_string_interpolation_is_folded() {
    let chunk = oryn::Chunk::compile(r#"print("sum: {1 + 2}")"#).expect("compile error");
    let disassembly = chunk.disassemble();

    assert!(disassembly.contains("PushString sum: 3"));
    assert!(!disassembly.contains("ToString"));
    assert!(!disassembly.contains("Concat"));
}
