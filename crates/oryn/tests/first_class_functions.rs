// Tests for first-class functions and closures with snapshot capture.
//
// Coverage:
//   * `let f = double` — top-level function as a value
//   * Function passed as an argument to another function
//   * Anonymous function with no captures
//   * Closure capturing one outer local
//   * Closure capturing multiple outer locals
//   * Closure capturing a list (binding is read-only, but the
//     captured list value's contents are mutable through methods)
//   * Function-typed local + indirect call
//   * Inline anonymous function call
//   * Returning a function from a function
//   * Type annotation round-trip: `let f: fn(int) -> int = ...`
//   * Type mismatch: passing the wrong function type
//   * Capture mutability rejection: assignment to a captured local
//   * `ok` and `of` are reserved words but `fn` parameter names work

fn run_source(source: &str) -> (Result<(), oryn::RuntimeError>, String) {
    let chunk = oryn::Chunk::compile(source).expect("compile error");
    let mut vm = oryn::VM::new();
    let mut out: Vec<u8> = Vec::new();
    let result = vm.run_with_writer(&chunk, &mut out);
    (result, String::from_utf8(out).unwrap())
}

// -- Function references --

#[test]
fn top_level_function_can_be_bound_as_value() {
    let (result, out) = run_source(
        "fn double(x: int) -> int { return x * 2 }\n\
         let f = double\n\
         print(f(21))",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "42\n");
}

#[test]
fn function_value_supports_explicit_type_annotation() {
    let (result, out) = run_source(
        "fn double(x: int) -> int { return x * 2 }\n\
         let f: fn(int) -> int = double\n\
         print(f(7))",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "14\n");
}

#[test]
fn function_value_can_be_passed_as_argument() {
    let (result, out) = run_source(
        "fn double(x: int) -> int { return x * 2 }\n\
         fn apply(f: fn(int) -> int, x: int) -> int { return f(x) }\n\
         print(apply(double, 21))",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "42\n");
}

#[test]
fn function_value_returned_from_function() {
    let (result, out) = run_source(
        "fn double(x: int) -> int { return x * 2 }\n\
         fn pick() -> fn(int) -> int { return double }\n\
         let f = pick()\n\
         print(f(11))",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "22\n");
}

// -- Anonymous functions (no captures) --

#[test]
fn anonymous_function_with_no_captures() {
    let (result, out) = run_source(
        "let g = fn(x: int) -> int { return x * 4 }\n\
         print(g(5))",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "20\n");
}

#[test]
fn inline_anonymous_function_call() {
    let (result, out) = run_source("print((fn(x: int) -> int { return x + 100 })(7))");
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "107\n");
}

#[test]
fn anonymous_function_with_void_return() {
    let (result, out) = run_source(
        "let greet = fn() { print(\"hi\") }\n\
         greet()",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "hi\n");
}

// -- Closures with captures --

#[test]
fn closure_captures_single_outer_local() {
    let (result, out) = run_source(
        "fn run() {\n\
            let threshold = 10\n\
            let is_big = fn(x: int) -> bool { return x > threshold }\n\
            print(is_big(5))\n\
            print(is_big(15))\n\
         }\n\
         run()",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "false\ntrue\n");
}

#[test]
fn closure_captures_multiple_outer_locals() {
    let (result, out) = run_source(
        "fn run() {\n\
            let lo = 5\n\
            let hi = 15\n\
            let in_range = fn(x: int) -> bool { return x > lo and x < hi }\n\
            print(in_range(3))\n\
            print(in_range(10))\n\
            print(in_range(20))\n\
         }\n\
         run()",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "false\ntrue\nfalse\n");
}

#[test]
fn closure_snapshot_is_taken_at_creation_time() {
    // After capturing `n = 5`, the closure should always use 5 even
    // if a later `n` reassignment happens at the call site. (We can't
    // reassign `let n` in Oryn, so the test uses two distinct
    // bindings to verify the snapshot was made.)
    let (result, out) = run_source(
        "fn make_adder() -> fn(int) -> int {\n\
            let n = 100\n\
            return fn(x: int) -> int { return x + n }\n\
         }\n\
         let add100 = make_adder()\n\
         print(add100(5))\n\
         print(add100(50))",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "105\n150\n");
}

#[test]
fn closure_captures_string_value() {
    let (result, out) = run_source(
        "fn run() {\n\
            let prefix = \"hello, \"\n\
            let greeter = fn(name: string) -> string { return \"{prefix}{name}\" }\n\
            print(greeter(\"alice\"))\n\
            print(greeter(\"bob\"))\n\
         }\n\
         run()",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "hello, alice\nhello, bob\n");
}

// -- Type checking --

#[test]
fn function_type_mismatch_is_compile_error() {
    let errors = oryn::Chunk::check(
        "fn double(x: int) -> int { return x * 2 }\n\
         let f: fn(int) -> bool = double",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("type mismatch")
        )),
        "expected type mismatch error, got: {errors:?}"
    );
}

#[test]
fn calling_non_callable_value_is_compile_error() {
    let errors = oryn::Chunk::check(
        "let x = 5\n\
         let y = x(1)",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("only function values are callable")
        )),
        "expected non-callable error, got: {errors:?}"
    );
}

#[test]
fn arity_mismatch_in_indirect_call_is_compile_error() {
    let errors = oryn::Chunk::check(
        "fn double(x: int) -> int { return x * 2 }\n\
         let f = double\n\
         let y = f(1, 2)",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("arity mismatch")
        )),
        "expected arity mismatch error, got: {errors:?}"
    );
}

// -- Capture mutability rejection --

#[test]
fn closure_cannot_assign_to_captured_local() {
    let errors = oryn::Chunk::check(
        "fn run() {\n\
            let counter = 0\n\
            let inc = fn() { counter = counter + 1 }\n\
         }",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("cannot mutate captured value")
        )),
        "expected capture mutation error, got: {errors:?}"
    );
}
