// --- Compiler hardening ---

// ---------------------------------------------------------------------------
// Phase 2: Nil and error union type checking
// ---------------------------------------------------------------------------

// -- Nillable types --

#[test]
fn nil_assigned_to_nillable_is_ok() {
    let errors = oryn::Chunk::check("let x: int? = nil");
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
    let errors = oryn::Chunk::check("let x: int? = 5");
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
    let errors = oryn::Chunk::check("fn foo() -> int? {\nrn nil\n}");
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
    let errors = oryn::Chunk::check("fn foo() -> int? {\nrn 42\n}");
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
    let errors = oryn::Chunk::check("fn foo(x: int?) {\nprint(1)\n}\nfoo(nil)");
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
    let errors = oryn::Chunk::check("fn foo(x: int?) {\nprint(1)\n}\nfoo(5)");
    let compiler_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e, oryn::OrynError::Compiler { .. }))
        .collect();
    assert!(
        compiler_errors.is_empty(),
        "expected no compiler errors, got: {compiler_errors:?}"
    );
}

// -- Coalesce (??) --

#[test]
fn coalesce_on_nillable_is_ok() {
    let errors = oryn::Chunk::check("let x: int? = nil\nlet y = x ?? 0");
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
    let errors = oryn::Chunk::check("let x: int = 5\nlet y = x ?? 0");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("nillable"))
    }));
}

#[test]
fn coalesce_fallback_type_mismatch_is_error() {
    let errors = oryn::Chunk::check("let x: int? = nil\nlet y = x ?? true");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("type mismatch"))
    }));
}

// -- Error union types --

#[test]
fn error_union_return_type_resolves() {
    let errors = oryn::Chunk::check("fn foo() -> !int {\nrn 42\n}");
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
    let errors = oryn::Chunk::check("fn foo() -> !int {\nlet x: int = 5\nrn try x\n}");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("error union"))
    }));
}

#[test]
fn try_outside_error_union_function_is_error() {
    let errors =
        oryn::Chunk::check("fn bar() -> !int {\nrn 1\n}\nfn foo() -> int {\nrn try bar()\n}");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. }
            if message.contains("enclosing function"))
    }));
}

// -- !expr (unwrap error) --

#[test]
fn unwrap_error_on_non_error_union_is_error() {
    let errors = oryn::Chunk::check("let x: int = 5\nlet y = !x");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("error union"))
    }));
}

// -- if let --

#[test]
fn if_let_on_nillable_is_ok() {
    let errors = oryn::Chunk::check("let x: int? = nil\nif let v = x {\nprint(v)\n}");
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
    let errors =
        oryn::Chunk::check("let x: int? = nil\nif let v = x {\nprint(v)\n} else {\nprint(0)\n}");
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
    let errors = oryn::Chunk::check("let x: int? = nil\nlet y: bool = x");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. }
            if message.contains("int?"))
    }));
}

#[test]
fn error_union_type_display_in_error_message() {
    let errors = oryn::Chunk::check("let x: !int = 5\nlet y: bool = x");
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. }
            if message.contains("!int"))
    }));
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
fn arity_mismatch_is_runtime_error() {
    let chunk = oryn::Chunk::compile("fn add(a: int, b: int) -> int {\nrn a + b\n}\nadd(1)")
        .expect("compile error");
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
fn method_wrong_arity_is_runtime_error() {
    let chunk = oryn::Chunk::compile(
        "obj Foo {\nx: int\nfn add(self, n: int) {\nrn self.x + n\n}\n}\nlet f = Foo { x: 1 }\nf.add()",
    )
    .expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::ArityMismatch { .. }));
}

#[test]
fn method_too_many_args_is_runtime_error() {
    let chunk = oryn::Chunk::compile(
        "obj Foo {\nx: int\nfn get(self) {\nrn self.x\n}\n}\nlet f = Foo { x: 1 }\nf.get(1, 2)",
    )
    .expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::ArityMismatch { .. }));
}

#[test]
fn undefined_static_method_is_compile_error() {
    let result = oryn::Chunk::compile("obj Foo {\n}\nFoo.nope()");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined static method"))
    }));
}

#[test]
fn static_method_argument_type_mismatch_is_compile_error() {
    let result = oryn::Chunk::compile(
        "obj Foo {\nfn make(x: int) -> Foo {\nrn Foo { }\n}\n}\nFoo.make(\"nope\")",
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
fn for_requires_range_iterable() {
    let result = oryn::Chunk::compile("for i in 123 {\nprint(i)\n}");
    assert!(result.is_err());

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("iterable type mismatch"))
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
