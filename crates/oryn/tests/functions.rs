mod common;
use common::run;

// --- Functions ---

#[test]
fn simple_function() {
    // No return type = void function.
    assert_eq!(run("fn greet() {\nprint(42)\n}\ngreet()"), "42\n");
}

#[test]
fn function_with_params() {
    assert_eq!(
        run("fn add(a: int, b: int) -> int {\nrn a + b\n}\nprint(add(3, 4))"),
        "7\n",
    );
}

#[test]
fn function_return_value() {
    assert_eq!(
        run("fn double(x: int) -> int {\nrn x * 2\n}\nlet y = double(5)\nprint(y)"),
        "10\n",
    );
}

#[test]
fn function_implicit_return() {
    // A void function without rn returns 0 (placeholder until None exists).
    assert_eq!(run("fn noop() {\nlet x = 1\n}\nprint(noop())"), "0\n");
}

#[test]
fn function_with_locals() {
    assert_eq!(
        run("let x = 1\nfn bump() -> int {\nlet x = 99\nrn x\n}\nprint(bump())\nprint(x)"),
        "99\n1\n",
    );
}

#[test]
fn recursive_function() {
    assert_eq!(
        run("fn fact(n: int) -> int {\nif n <= 1 { rn 1 }\nrn n * fact(n - 1)\n}\nprint(fact(5))"),
        "120\n",
    );
}

#[test]
fn fibonacci() {
    assert_eq!(
        run(
            "fn fib(n: int) -> int {\nif n <= 1 { rn n }\nrn fib(n - 1) + fib(n - 2)\n}\nprint(fib(10))"
        ),
        "55\n",
    );
}

// --- Return type enforcement ---

#[test]
fn function_return_type_mismatch_is_compile_error() {
    let result = oryn::Chunk::compile("fn bad() -> String { rn 123 }");
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("return type mismatch"))
    }));
}

#[test]
fn function_return_type_mismatch_float_for_int() {
    let result = oryn::Chunk::compile("fn bad() -> int { rn 1.5 }");
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("return type mismatch"))
    }));
}

#[test]
fn function_correct_return_type_still_works() {
    assert_eq!(
        run("fn double(x: int) -> int { rn x * 2 }\nprint(double(3))"),
        "6\n",
    );
}
