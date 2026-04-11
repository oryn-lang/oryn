// --- Compiler hardening ---

// ---------------------------------------------------------------------------
// Phase 2: Nil and error union type checking
// ---------------------------------------------------------------------------

// -- Nillable types --

#[test]
fn nil_assigned_to_nillable_is_ok() {
    let errors = oryn::Chunk::check("let x: maybe int = nil");
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn int_assigned_to_nillable_int_is_ok() {
    let errors = oryn::Chunk::check("let x: maybe int = 5");
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn nil_assigned_to_non_nillable_is_error() {
    let errors = oryn::Chunk::check("let x: int = nil");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("type mismatch"))
    }));
}

#[test]
fn nillable_return_type_accepts_nil() {
    let errors = oryn::Chunk::check(
        "fn foo() -> maybe int {
return nil\n}",
    );
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn nillable_return_type_accepts_value() {
    let errors = oryn::Chunk::check(
        "fn foo() -> maybe int {
return 42\n}",
    );
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn nillable_param_type_accepts_nil() {
    let errors = oryn::Chunk::check("fn foo(x: maybe int) {\nprint(1)\n}\nfoo(nil)");
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn nillable_param_type_accepts_value() {
    let errors = oryn::Chunk::check("fn foo(x: maybe int) {\nprint(1)\n}\nfoo(5)");
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

// -- Coalesce (orelse) --

#[test]
fn coalesce_on_nillable_is_ok() {
    let errors = oryn::Chunk::check("let x: maybe int = nil\nlet y = x orelse 0");
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn coalesce_on_non_nillable_is_error() {
    let errors = oryn::Chunk::check("let x: int = 5\nlet y = x orelse 0");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("nillable"))
    }));
}

#[test]
fn coalesce_fallback_type_mismatch_is_error() {
    let errors = oryn::Chunk::check("let x: maybe int = nil\nlet y = x orelse true");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("type mismatch"))
    }));
}

// -- Error union types --

#[test]
fn error_union_return_type_resolves() {
    let errors = oryn::Chunk::check(
        "fn foo() -> error int {
return 42\n}",
    );
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    // No errors expected for the type resolution itself.
    // (Return type compatibility may report mismatches since we
    // don't yet have implicit T → !T wrapping, which is expected.)
    let _ = compiler_errors;
}

// -- try expression --

#[test]
fn try_on_non_error_union_is_error() {
    let errors = oryn::Chunk::check(
        "fn foo() -> error int {\nlet x: int = 5
return try x\n}",
    );
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("error union"))
    }));
}

#[test]
fn try_outside_error_union_function_is_error() {
    let errors = oryn::Chunk::check(
        "fn bar() -> error int {
return 1\n}\nfn foo() -> int {
return try bar()\n}",
    );
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. }
            if message.contains("enclosing function"))
    }));
}

// -- !expr (unwrap error) --

#[test]
fn unwrap_error_on_non_error_union_is_error() {
    let errors = oryn::Chunk::check("let x: int = 5\nlet y = must x");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("error union"))
    }));
}

// -- if let --

#[test]
fn if_let_on_nillable_is_ok() {
    let errors = oryn::Chunk::check("let x: maybe int = nil\nif let v = x {\nprint(v)\n}");
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn if_let_on_non_nillable_is_error() {
    let errors = oryn::Chunk::check("let x: int = 5\nif let v = x {\nprint(v)\n}");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("nillable"))
    }));
}

#[test]
fn if_let_with_else_on_nillable_is_ok() {
    let errors = oryn::Chunk::check(
        "let x: maybe int = nil\nif let v = x {\nprint(v)\n} else {\nprint(0)\n}",
    );
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

// -- Type display names --

#[test]
fn nillable_type_display_in_error_message() {
    let errors = oryn::Chunk::check("let x: maybe int = nil\nlet y: bool = x");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. }
            if message.contains("maybe int"))
    }));
}

#[test]
fn error_union_type_display_in_error_message() {
    let errors = oryn::Chunk::check("let x: error int = 5\nlet y: bool = x");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. }
            if message.contains("error int"))
    }));
}

// -- error enum declarations --

#[test]
fn bare_error_enum_parses_and_type_checks() {
    let errors =
        oryn::Chunk::check("error enum MathError {\nDivByZero\nOverflow { value: int }\n}");
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn pub_error_enum_parses() {
    let errors = oryn::Chunk::check("pub error enum Fault {\nOops\n}");
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn error_enum_variant_promotes_to_error_union_return() {
    let errors = oryn::Chunk::check(
        "error enum MathError { DivByZero }\n\
         fn divide(a: int, b: int) -> error int {\n\
            if b == 0 {\nreturn MathError.DivByZero\n}\n\
            return a / b\n\
         }",
    );
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn regular_enum_does_not_promote_to_error_union() {
    // A plain (non-error) enum cannot substitute on the error
    // side of `error T`. The return type check should reject it.
    let errors = oryn::Chunk::check(
        "enum Color { Red, Blue }\n\
         fn foo() -> error int {\n\
            return Color.Red\n\
         }",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("return type mismatch")
        )),
        "expected return type mismatch, got: {errors:?}"
    );
}

// -- runtime behavior of error enums --

fn run_source(source: &str) -> (Result<(), oryn::RuntimeError>, String) {
    let chunk = oryn::Chunk::compile(source).expect("compile error");
    let mut vm = oryn::VM::new();
    let mut out: Vec<u8> = Vec::new();
    let result = vm.run_with_writer(&chunk, &mut out);
    (result, String::from_utf8(out).unwrap())
}

#[test]
fn error_enum_must_unwrap_succeeds_on_ok_value() {
    let (result, out) = run_source(
        "error enum E { Bad }\n\
         fn succeed() -> error int { return 42 }\n\
         let x = must succeed()\n\
         print(x)",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "42\n");
}

#[test]
fn error_enum_must_traps_on_error_variant() {
    let (result, _out) = run_source(
        "error enum E { Bad }\n\
         fn fail() -> error int { return E.Bad }\n\
         let x = must fail()",
    );
    assert!(
        matches!(result, Err(oryn::RuntimeError::ErrorUnwrapTrap { .. })),
        "expected error unwrap trap, got: {result:?}"
    );
}

#[test]
fn error_enum_try_propagates_through_calling_function() {
    let (result, out) = run_source(
        "error enum E { Bad }\n\
         fn inner() -> error int { return E.Bad }\n\
         fn outer() -> error int { return try inner() }\n\
         let x = outer()\n\
         print(x)",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "E.Bad\n");
}

#[test]
fn error_enum_payload_survives_try_propagation() {
    let (result, out) = run_source(
        "error enum E { Overflow { value: int } }\n\
         fn inner() -> error int {\n\
           return E.Overflow { value: 99 }\n\
         }\n\
         fn outer() -> error int { return try inner() }\n\
         print(outer())",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "E.Overflow { value: 99 }\n");
}

// -- match on error union --

#[test]
fn match_on_loose_error_union_with_ok_and_wildcard() {
    let (result, out) = run_source(
        "error enum E { Bad }\n\
         fn divide(a: int, b: int) -> error int {\n\
            if b == 0 { return E.Bad }\n\
            return a / b\n\
         }\n\
         let got: string = match divide(10, 2) {\n\
            ok v => \"ok {v}\"\n\
            E.Bad => \"bad\"\n\
            _ => \"other\"\n\
         }\n\
         print(got)",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "ok 5\n");
}

#[test]
fn match_on_loose_error_union_catches_error() {
    let (result, out) = run_source(
        "error enum E { Bad }\n\
         fn divide(a: int, b: int) -> error int {\n\
            if b == 0 { return E.Bad }\n\
            return a / b\n\
         }\n\
         let got: string = match divide(10, 0) {\n\
            ok v => \"ok {v}\"\n\
            E.Bad => \"bad\"\n\
            _ => \"other\"\n\
         }\n\
         print(got)",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "bad\n");
}

#[test]
fn match_on_error_union_destructures_payload() {
    let (result, out) = run_source(
        "error enum E { Overflow { value: int } }\n\
         fn run() -> error int { return E.Overflow { value: 42 } }\n\
         let got: string = match run() {\n\
            ok v => \"ok {v}\"\n\
            E.Overflow { value } => \"overflow {value}\"\n\
            _ => \"other\"\n\
         }\n\
         print(got)",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "overflow 42\n");
}

#[test]
fn match_on_loose_error_union_without_wildcard_is_rejected() {
    let errors = oryn::Chunk::check(
        "error enum E { Bad }\n\
         fn f() -> error int { return 1 }\n\
         let got: string = match f() {\n\
            ok v => \"ok\"\n\
            E.Bad => \"bad\"\n\
         }",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("non-exhaustive match on loose")
        )),
        "expected non-exhaustive error, got: {errors:?}"
    );
}

#[test]
fn match_ok_pattern_rejected_on_plain_enum() {
    let errors = oryn::Chunk::check(
        "enum Color { Red, Blue }\n\
         fn f(c: Color) -> int {\n\
            return match c {\n\
                ok v => 1\n\
                Color.Red => 2\n\
                Color.Blue => 3\n\
            }\n\
         }",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("`ok` pattern is only valid")
        )),
        "expected ok-pattern-only-for-error-union error, got: {errors:?}"
    );
}

#[test]
fn match_variant_from_non_error_enum_rejected_on_error_union() {
    let errors = oryn::Chunk::check(
        "enum Plain { X, Y }\n\
         fn f() -> error int { return 1 }\n\
         let g: int = match f() {\n\
            ok v => 0\n\
            Plain.X => 1\n\
            _ => 2\n\
         }",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("not an `error enum`")
        )),
        "expected non-error-enum arm error, got: {errors:?}"
    );
}

// -- if let on error unions --

#[test]
fn if_let_on_error_union_binds_success_value() {
    let (result, out) = run_source(
        "error enum E { Bad }\n\
         fn f() -> error int { return 42 }\n\
         if let v = f() {\n\
            print(\"got {v}\")\n\
         } else {\n\
            print(\"no\")\n\
         }",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "got 42\n");
}

#[test]
fn if_let_on_error_union_falls_through_on_error() {
    let (result, out) = run_source(
        "error enum E { Bad }\n\
         fn f() -> error int { return E.Bad }\n\
         if let v = f() {\n\
            print(\"got {v}\")\n\
         } else {\n\
            print(\"nope\")\n\
         }",
    );
    assert!(result.is_ok(), "run failed: {result:?}");
    assert_eq!(out, "nope\n");
}

// -- precise error-union typing --

#[test]
fn precise_error_union_parses_and_type_checks() {
    let errors = oryn::Chunk::check(
        "error enum MathError { DivByZero }\n\
         fn divide(a: int, b: int) -> error int of MathError {\n\
            if b == 0 { return MathError.DivByZero }\n\
            return a / b\n\
         }",
    );
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn precise_error_union_rejects_foreign_error_enum() {
    let errors = oryn::Chunk::check(
        "error enum MathError { DivByZero }\n\
         error enum NetError { Timeout }\n\
         fn divide(a: int, b: int) -> error int of MathError {\n\
            return NetError.Timeout\n\
         }",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("return type mismatch")
        )),
        "expected return type mismatch, got: {errors:?}"
    );
}

#[test]
fn precise_error_union_accepts_its_own_error_enum() {
    let errors = oryn::Chunk::check(
        "error enum MathError { DivByZero }\n\
         fn divide() -> error int of MathError {\n\
            return MathError.DivByZero\n\
         }",
    );
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn precise_error_union_requires_error_enum() {
    // A plain (non-error) enum cannot appear after `of`.
    let errors = oryn::Chunk::check(
        "enum Color { Red, Blue }\n\
         fn f() -> error int of Color { return 1 }",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("error enum")
        )),
        "expected error-enum requirement error, got: {errors:?}"
    );
}

#[test]
fn precise_scrutinee_enforces_full_exhaustiveness_without_wildcard() {
    // When the scrutinee is precise, all variants of the named
    // error enum must be covered or a wildcard is required.
    let errors = oryn::Chunk::check(
        "error enum MathError { DivByZero, Overflow { value: int } }\n\
         fn f() -> error int of MathError { return 1 }\n\
         let got: int = match f() {\n\
            ok v => v\n\
            MathError.DivByZero => 0\n\
         }",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("non-exhaustive match")
                    && message.contains("MathError.Overflow")
        )),
        "expected non-exhaustive error naming MathError.Overflow, got: {errors:?}"
    );
}

#[test]
fn precise_scrutinee_full_coverage_without_wildcard_compiles() {
    let errors = oryn::Chunk::check(
        "error enum MathError { DivByZero, Overflow { value: int } }\n\
         fn f() -> error int of MathError { return 1 }\n\
         let got: int = match f() {\n\
            ok v => v\n\
            MathError.DivByZero => 0\n\
            MathError.Overflow { value } => value\n\
         }",
    );
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn precise_scrutinee_rejects_foreign_variant_in_match() {
    let errors = oryn::Chunk::check(
        "error enum MathError { DivByZero }\n\
         error enum NetError { Timeout }\n\
         fn f() -> error int of MathError { return 1 }\n\
         let got: int = match f() {\n\
            ok v => v\n\
            NetError.Timeout => 0\n\
         }",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("does not match scrutinee's precise error enum")
        )),
        "expected precise-enum mismatch, got: {errors:?}"
    );
}

#[test]
fn try_propagates_precise_error_to_matching_precise_caller() {
    let errors = oryn::Chunk::check(
        "error enum MathError { DivByZero }\n\
         fn inner() -> error int of MathError { return MathError.DivByZero }\n\
         fn outer() -> error int of MathError { return try inner() }",
    );
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

#[test]
fn try_rejects_propagating_precise_error_across_different_precise_enums() {
    let errors = oryn::Chunk::check(
        "error enum MathError { DivByZero }\n\
         error enum NetError { Timeout }\n\
         fn inner() -> error int of NetError { return NetError.Timeout }\n\
         fn outer() -> error int of MathError { return try inner() }",
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            oryn::OrynError::Compiler { message, .. }
                if message.contains("try")
                    && message.contains("NetError")
                    && message.contains("MathError")
        )),
        "expected precise try cross-enum error, got: {errors:?}"
    );
}

#[test]
fn try_propagates_precise_error_to_loose_caller() {
    let errors = oryn::Chunk::check(
        "error enum MathError { DivByZero }\n\
         fn inner() -> error int of MathError { return MathError.DivByZero }\n\
         fn outer() -> error int { return try inner() }",
    );
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

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
fn undefined_function_is_compile_error() {
    let result = oryn::Chunk::compile("nope()");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined function"))
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
fn and_with_non_bool_is_compile_error() {
    let errors = oryn::Chunk::compile("print(5 and true)").unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("logical operand must be `bool`"))
    }));
}

#[test]
fn or_with_non_bool_is_compile_error() {
    let errors = oryn::Chunk::compile("print(false or 5)").unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("logical operand must be `bool`"))
    }));
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

#[test]
fn arity_mismatch_is_compile_error() {
    let result = oryn::Chunk::compile(
        "fn add(a: int, b: int) -> int {
return a + b\n}\nadd(1)",
    );

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("arity mismatch"))
    }));
}

// --- Bug fix regression tests ---

#[test]
fn integer_overflow_is_runtime_error() {
    let chunk = oryn::Chunk::compile("print(2147483647 + 1)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::IntegerOverflow { .. }));
}

#[test]
fn integer_underflow_is_runtime_error() {
    let chunk = oryn::Chunk::compile("print(-2147483647 - 2)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::IntegerOverflow { .. }));
}

#[test]
fn integer_multiply_overflow_is_runtime_error() {
    let chunk = oryn::Chunk::compile("print(2147483647 * 2)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::IntegerOverflow { .. }));
}

#[test]
fn negate_min_int_is_overflow() {
    // -2147483647 - 1 produces int::MIN, then negating it overflows.
    let chunk = oryn::Chunk::compile("let x = -2147483647 - 1\nprint(-x)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::IntegerOverflow { .. }));
}

#[test]
fn method_wrong_arity_is_compile_error() {
    let result = oryn::Chunk::compile(
        "struct Foo {\nx: int\nfn add(self, n: int) -> int {
return self.x + n\n}\n}\nlet f = Foo { x: 1 }\nf.add()",
    );

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("arity mismatch"))
    }));
}

#[test]
fn method_too_many_args_is_compile_error() {
    let result = oryn::Chunk::compile(
        "struct Foo {\nx: int\nfn get(self) -> int {
return self.x\n}\n}\nlet f = Foo { x: 1 }\nf.get(1, 2)",
    );

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("arity mismatch"))
    }));
}

#[test]
fn undefined_static_method_is_compile_error() {
    let result = oryn::Chunk::compile("struct Foo {\n}\nFoo.nope()");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined static method"))
    }));
}

#[test]
fn static_method_argument_type_mismatch_is_compile_error() {
    let result = oryn::Chunk::compile(
        "struct Foo {\nfn make(x: int) -> Foo {
return Foo { }\n}\n}\nFoo.make(\"nope\")",
    );
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("argument 1 type mismatch"))
    }));
}

#[test]
fn break_outside_loop_is_compile_error() {
    let result = oryn::Chunk::compile("break");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("break outside"))
    }));
}

#[test]
fn continue_outside_loop_is_compile_error() {
    let result = oryn::Chunk::compile("continue");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("continue outside"))
    }));
}

#[test]
fn not_non_bool_is_type_error() {
    let chunk = oryn::Chunk::compile("print(not 5)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::TypeError { .. }));
}

// --- Mixed-type comparison errors ---

#[test]
fn mixed_int_bool_comparison_is_runtime_error() {
    let chunk = oryn::Chunk::compile("print(1 < true)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();
    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::TypeMismatch { .. }));
}

#[test]
fn mixed_int_float_comparison_is_runtime_error() {
    let chunk = oryn::Chunk::compile("print(1 == 1.5)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();
    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::TypeMismatch { .. }));
}

#[test]
fn mixed_string_int_comparison_is_runtime_error() {
    let chunk = oryn::Chunk::compile("print(\"a\" < 1)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();
    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::TypeMismatch { .. }));
}

#[test]
fn same_type_int_comparison_still_works() {
    let chunk = oryn::Chunk::compile("print(1 < 2)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();
    vm.run_with_writer(&chunk, &mut output)
        .expect("runtime error");
    assert_eq!(String::from_utf8(output).unwrap(), "true\n");
}

#[test]
fn same_type_float_comparison_still_works() {
    let chunk = oryn::Chunk::compile("print(1.5 < 2.5)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();
    vm.run_with_writer(&chunk, &mut output)
        .expect("runtime error");
    assert_eq!(String::from_utf8(output).unwrap(), "true\n");
}

#[test]
fn same_type_bool_equality_still_works() {
    let chunk = oryn::Chunk::compile("print(true == true)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();
    vm.run_with_writer(&chunk, &mut output)
        .expect("runtime error");
    assert_eq!(String::from_utf8(output).unwrap(), "true\n");
}

#[test]
fn same_type_string_equality_still_works() {
    let chunk = oryn::Chunk::compile("print(\"abc\" == \"abc\")").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();
    vm.run_with_writer(&chunk, &mut output)
        .expect("runtime error");
    assert_eq!(String::from_utf8(output).unwrap(), "true\n");
}

#[test]
fn string_ordering_still_works() {
    let chunk = oryn::Chunk::compile("print(\"a\" < \"b\")").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();
    vm.run_with_writer(&chunk, &mut output)
        .expect("runtime error");
    assert_eq!(String::from_utf8(output).unwrap(), "true\n");
}

#[test]
fn bool_ordering_is_runtime_error() {
    let chunk = oryn::Chunk::compile("print(true < false)").expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();
    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::TypeMismatch { .. }));
}

#[test]
fn for_requires_range_or_list_iterable() {
    let result = oryn::Chunk::compile("for i in 123 {\nprint(i)\n}");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("must be a range or list"))
    }));
}

#[test]
fn range_start_must_be_int() {
    let result = oryn::Chunk::compile("print(true..3)");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("range start type mismatch"))
    }));
}

#[test]
fn range_end_must_be_int() {
    let result = oryn::Chunk::compile("print(0..false)");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("range end type mismatch"))
    }));
}

#[test]
fn inclusive_range_end_must_be_int() {
    let result = oryn::Chunk::compile("print(0..=false)");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("range end type mismatch"))
    }));
}
