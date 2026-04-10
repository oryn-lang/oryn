mod common;
use common::run;

// --- Objects ---

#[test]
fn object_definition_and_instantiation() {
    assert_eq!(
        run("obj Vec2 {\nx: int\ny: int\n}\nlet v = Vec2 { x: 1, y: 2 }\nprint(v.x)"),
        "1\n",
    );
}

#[test]
fn object_field_read_second_field() {
    assert_eq!(
        run("obj Vec2 {\nx: int\ny: int\n}\nlet v = Vec2 { x: 1, y: 2 }\nprint(v.y)"),
        "2\n",
    );
}

#[test]
fn object_field_mutation() {
    assert_eq!(
        run("obj Vec2 {\nx: int\ny: int\n}\nlet v = Vec2 { x: 1, y: 2 }\nv.x = 99\nprint(v.x)"),
        "99\n",
    );
}

#[test]
fn object_reference_aliasing() {
    assert_eq!(
        run(
            "obj Vec2 {\nx: int\ny: int\n}\nlet v = Vec2 { x: 1, y: 2 }\nlet w = v\nw.y = 50\nprint(v.y)"
        ),
        "50\n",
    );
}

#[test]
fn object_fields_out_of_order() {
    assert_eq!(
        run("obj Vec2 {\nx: int\ny: int\n}\nlet v = Vec2 { y: 20, x: 10 }\nprint(v.x)\nprint(v.y)"),
        "10\n20\n",
    );
}

#[test]
fn object_print_shows_instance() {
    assert_eq!(
        run("obj Foo {\nx: int\n}\nlet f = Foo { x: 1 }\nprint(f)"),
        "<Foo instance>\n",
    );
}

#[test]
fn val_prevents_field_mutation() {
    let result = oryn::Chunk::compile("obj Foo {\nx: int\n}\nval f = Foo { x: 1 }\nf.x = 2");

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
    let result = oryn::Chunk::compile("obj Foo {\nx: int\n}\nlet f = Foo { x: 1, z: 2 }");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("unknown field"))
    }));
}

#[test]
fn missing_field_is_compile_error() {
    let result = oryn::Chunk::compile("obj Foo {\nx: int\ny: int\n}\nlet f = Foo { x: 1 }");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("missing field"))
    }));
}

#[test]
fn object_inline_definition() {
    assert_eq!(
        run("obj Vec2 { x: int, y: int }\nlet v = Vec2 { x: 1, y: 2 }\nprint(v.x)"),
        "1\n",
    );
}

#[test]
fn object_with_float_fields() {
    assert_eq!(
        run("obj Point {\nx: float\ny: float\n}\nlet p = Point { x: 3.14, y: 2.71 }\nprint(p.x)"),
        "3.14\n",
    );
}

#[test]
fn object_in_function() {
    assert_eq!(
        run(
            "obj Vec2 {\nx: int\ny: int\n}\nfn get_x(v: Vec2) -> int {\nrn v.x\n}\nlet v = Vec2 { x: 42, y: 0 }\nprint(get_x(v))"
        ),
        "42\n",
    );
}

#[test]
fn obj_field_unknown_type_is_compile_error() {
    let result = oryn::Chunk::compile("obj Foo { x: huge }");

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("undefined type"))
    }));
}

// --- Methods ---

#[test]
fn method_no_params() {
    assert_eq!(
        run(
            "obj Vec2 {\nx: int\ny: int\nfn sum(self) {\nrn self.x + self.y\n}\n}\nlet v = Vec2 { x: 3, y: 4 }\nprint(v.sum())"
        ),
        "7\n",
    );
}

#[test]
fn method_with_params() {
    assert_eq!(
        run(
            "obj Counter {\ncount: int\nfn add(self, n: int) {\nrn self.count + n\n}\n}\nlet c = Counter { count: 10 }\nprint(c.add(5))"
        ),
        "15\n",
    );
}

#[test]
fn method_mutates_field() {
    assert_eq!(
        run(
            "obj Counter {\ncount: int\nfn inc(self) {\nself.count = self.count + 1\n}\n}\nlet c = Counter { count: 0 }\nc.inc()\nprint(c.count)"
        ),
        "1\n",
    );
}

#[test]
fn method_on_val_binding() {
    // Methods should still work on val bindings (calling doesn't reassign).
    assert_eq!(
        run(
            "obj Vec2 {\nx: int\ny: int\nfn sum(self) {\nrn self.x + self.y\n}\n}\nval v = Vec2 { x: 1, y: 2 }\nprint(v.sum())"
        ),
        "3\n",
    );
}

#[test]
fn method_with_float_fields() {
    assert_eq!(
        run(
            "obj Circle {\nradius: float\nfn area(self) {\nrn self.radius * self.radius * 3.14\n}\n}\nlet c = Circle { radius: 2.0 }\nprint(c.area())"
        ),
        "12.56\n",
    );
}

#[test]
fn multiple_methods() {
    assert_eq!(
        run(
            "obj Vec2 {\nx: int\ny: int\nfn get_x(self) {\nrn self.x\n}\nfn get_y(self) {\nrn self.y\n}\n}\nlet v = Vec2 { x: 10, y: 20 }\nprint(v.get_x())\nprint(v.get_y())"
        ),
        "10\n20\n",
    );
}

#[test]
fn undefined_method_is_runtime_error() {
    let chunk = oryn::Chunk::compile("obj Foo {\nx: int\n}\nlet f = Foo { x: 1 }\nf.nope()")
        .expect("compile error");
    let mut vm = oryn::VM::new();
    let mut output = Vec::new();

    let err = vm.run_with_writer(&chunk, &mut output).unwrap_err();
    assert!(matches!(err, oryn::RuntimeError::UndefinedFunction { .. }));
}

#[test]
fn static_method_no_params() {
    assert_eq!(
        run(
            "obj Vec2 {\nx: int\ny: int\nfn zero() -> Vec2 {\nrn Vec2 { x: 0, y: 0 }\n}\n}\nlet v = Vec2.zero()\nprint(v.x)\nprint(v.y)"
        ),
        "0\n0\n",
    );
}

#[test]
fn static_method_with_params() {
    assert_eq!(
        run(
            "obj Counter {\ncount: int\nfn make(n: int) -> Counter {\nrn Counter { count: n }\n}\n}\nlet c = Counter.make(10)\nprint(c.count)"
        ),
        "10\n",
    );
}

#[test]
fn static_method_can_call_other_static_method() {
    assert_eq!(
        run(
            "obj Vec2 {\nx: int\ny: int\nfn zero() -> Vec2 {\nrn Vec2 { x: 0, y: 0 }\n}\nfn unit_x() -> Vec2 {\nrn Vec2.zero()\n}\n}\nlet v = Vec2.unit_x()\nprint(v.x)\nprint(v.y)"
        ),
        "0\n0\n",
    );
}

#[test]
fn use_inherits_static_methods() {
    assert_eq!(
        run(
            "obj Factory {\nfn answer() -> int {\nrn 42\n}\n}\nobj Wrapper {\nuse Factory\n}\nprint(Wrapper.answer())"
        ),
        "42\n",
    );
}

// --- Use composition ---

#[test]
fn use_inherits_fields() {
    assert_eq!(
        run(
            "obj Health { hp: int }\nobj Player {\nuse Health\nname: String\n}\nlet p = Player { hp: 100, name: \"Alice\" }\nprint(p.hp)"
        ),
        "100\n",
    );
}

#[test]
fn use_inherits_methods() {
    assert_eq!(
        run(
            "obj Health {\nhp: int\nfn heal(self, amount: int) {\nself.hp = self.hp + amount\n}\n}\nobj Player {\nuse Health\nname: String\n}\nlet p = Player { hp: 50, name: \"Bob\" }\np.heal(20)\nprint(p.hp)"
        ),
        "70\n",
    );
}

#[test]
fn use_multiple_types() {
    assert_eq!(
        run(
            "obj Health { hp: int }\nobj Named { name: String }\nobj Player {\nuse Health\nuse Named\n}\nlet p = Player { hp: 100, name: \"Alice\" }\nprint(p.hp)\nprint(p.name)"
        ),
        "100\nAlice\n",
    );
}

#[test]
fn use_field_conflict_is_compile_error() {
    let result =
        oryn::Chunk::compile("obj A { x: int }\nobj B { x: int }\nobj C {\nuse A\nuse B\n}");

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
            "obj Position { x: int, y: int }\nobj Entity {\nuse Position\nname: String\n}\nlet e = Entity { x: 5, y: 10, name: \"thing\" }\nprint(e.x)\nprint(e.name)"
        ),
        "5\nthing\n",
    );
}

// --- Signatures (required methods) ---

#[test]
fn signature_satisfied_by_own_method() {
    assert_eq!(
        run(
            "obj Printable {\nfn to_string(self) -> String\n}\nobj Foo {\nuse Printable\nname: String\nfn to_string(self) -> String {\nrn self.name\n}\n}\nlet f = Foo { name: \"hello\" }\nprint(f.to_string())"
        ),
        "hello\n",
    );
}

#[test]
fn missing_signature_is_compile_error() {
    let result = oryn::Chunk::compile(
        "obj Printable {\nfn to_string(self) -> String\n}\nobj Foo {\nuse Printable\nname: String\n}",
    );

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("missing required method"))
    }));
}

#[test]
fn signature_satisfied_by_composed_method() {
    // Health provides heal(), Healable requires heal().
    // Player uses both - heal() from Health satisfies Healable's requirement.
    assert_eq!(
        run(
            "obj Healable {\nfn heal(self, amount: int)\n}\nobj Health {\nhp: int\nfn heal(self, amount: int) {\nself.hp = self.hp + amount\n}\n}\nobj Player {\nuse Healable\nuse Health\n}\nlet p = Player { hp: 50 }\np.heal(20)\nprint(p.hp)"
        ),
        "70\n",
    );
}

#[test]
fn signature_on_type_with_no_uses() {
    // An object with only signatures and no fields is valid (a pure interface).
    let result = oryn::Chunk::compile("obj Printable {\nfn to_string(self) -> String\n}");
    assert!(result.is_ok());
}

#[test]
fn multiple_signatures_all_must_be_satisfied() {
    let result = oryn::Chunk::compile(
        "obj Serializable {\nfn to_string(self) -> String\nfn to_bytes(self) -> int\n}\nobj Foo {\nuse Serializable\nfn to_string(self) -> String {\nrn \"foo\"\n}\n}",
    );

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("to_bytes"))
    }));
}

// --- Method return type enforcement ---

#[test]
fn method_return_type_mismatch_is_compile_error() {
    let result = oryn::Chunk::compile(
        "obj Foo {\n  name: String\n  fn get_name(self) -> String {\n    rn 42\n  }\n}",
    );
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("return type mismatch"))
    }));
}

#[test]
fn method_correct_return_type_still_works() {
    assert_eq!(
        run(
            "obj Foo {\n  name: String\n  fn get_name(self) -> String {\n    rn self.name\n  }\n}\nlet f = Foo { name: \"hello\" }\nprint(f.get_name())"
        ),
        "hello\n",
    );
}

// --- Signature shape checking ---

#[test]
fn signature_wrong_return_type_is_compile_error() {
    // Signature requires -> String but impl returns int.
    let result = oryn::Chunk::compile(
        "obj Printable {\nfn to_string(self) -> String\n}\nobj Foo {\nuse Printable\nfn to_string(self) -> int {\nrn 42\n}\n}",
    );
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("return type mismatch"))
    }));
}

#[test]
fn signature_wrong_param_type_is_compile_error() {
    // Signature requires fn process(self, x: int) but impl has (self, x: String).
    let result = oryn::Chunk::compile(
        "obj Processor {\nfn process(self, x: int)\n}\nobj Foo {\nuse Processor\nfn process(self, x: String) {\nprint(x)\n}\n}",
    );
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("parameter 1 type mismatch"))
    }));
}

#[test]
fn signature_wrong_param_count_is_compile_error() {
    // Signature requires fn process(self, x: int) but impl has (self).
    let result = oryn::Chunk::compile(
        "obj Processor {\nfn process(self, x: int)\n}\nobj Foo {\nuse Processor\nfn process(self) {\nprint(1)\n}\n}",
    );
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        matches!(e, oryn::OrynError::Compiler { message, .. } if message.contains("parameter(s) but signature requires"))
    }));
}

#[test]
fn signature_matching_shape_compiles() {
    // Full shape match: same params and return type.
    assert_eq!(
        run(
            "obj Printable {\nfn to_string(self) -> String\n}\nobj Foo {\nuse Printable\nname: String\nfn to_string(self) -> String {\nrn self.name\n}\n}\nlet f = Foo { name: \"hello\" }\nprint(f.to_string())"
        ),
        "hello\n",
    );
}

#[test]
fn signature_with_params_matching_shape_compiles() {
    // Signature with extra params - shape must match.
    assert_eq!(
        run(
            "obj Healable {\nfn heal(self, amount: int)\n}\nobj Health {\nhp: int\nfn heal(self, amount: int) {\nself.hp = self.hp + amount\n}\n}\nobj Player {\nuse Healable\nuse Health\n}\nlet p = Player { hp: 50 }\np.heal(20)\nprint(p.hp)"
        ),
        "70\n",
    );
}
