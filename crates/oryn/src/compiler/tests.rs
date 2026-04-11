use super::types::{BuiltinFunction, ListMethod, ModuleTable};
use super::*;

use crate::parser::{BinOp, Expression, Spanned, Statement};

fn spanned<T>(node: T) -> Spanned<T> {
    Spanned { node, span: 0..0 }
}

#[test]
fn flattens_ast_to_instructions() {
    let stmts = vec![spanned(Statement::Expression(spanned(
        Expression::BinaryOp {
            op: BinOp::Add,
            left: Box::new(spanned(Expression::Int(1))),
            right: Box::new(spanned(Expression::Int(2))),
        },
    )))];

    let output = compile(stmts, ModuleTable::default(), 0, 0, vec![]);

    assert_eq!(
        output.instructions,
        vec![Instruction::PushInt(3), Instruction::Pop,]
    );
    assert_eq!(output.instructions.len(), output.spans.len());
}

#[test]
fn expression_statements_are_popped() {
    let stmts = vec![spanned(Statement::Expression(spanned(Expression::Int(1))))];
    let output = compile(stmts, ModuleTable::default(), 0, 0, vec![]);

    assert_eq!(output.instructions.last(), Some(&Instruction::Pop));
}

#[test]
fn builtin_calls_are_lowered_to_typed_builtins() {
    let stmts = vec![spanned(Statement::Expression(spanned(Expression::Call {
        name: "print".to_string(),
        args: vec![spanned(Expression::Int(1))],
    })))];

    let output = compile(stmts, ModuleTable::default(), 0, 0, vec![]);

    assert_eq!(
        output.instructions,
        vec![
            Instruction::PushInt(1),
            Instruction::CallBuiltin(BuiltinFunction::Print, 1),
            Instruction::Pop,
        ]
    );
}

#[test]
fn assert_lowers_to_assert_instruction() {
    let chunk = crate::Chunk::compile("assert(true)").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Assert))
    );
}

#[test]
fn assert_rejects_non_bool_condition() {
    let errors = crate::Chunk::compile("assert(5)").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("assert") || format!("{e}").contains("type")),
        "expected a type-related compile error for non-bool assert, got {errors:?}"
    );
}

#[test]
fn test_blocks_are_collected_into_tests_vec() {
    let chunk =
        crate::Chunk::compile("test \"one\" { assert(true) }\ntest \"two\" { assert(1 == 1) }")
            .unwrap();

    assert_eq!(chunk.tests().len(), 2);
    assert_eq!(chunk.tests()[0].display_name, "one");
    assert_eq!(chunk.tests()[1].display_name, "two");
}

#[test]
fn test_body_compiles_as_function() {
    // Each test should produce a synthetic function with a body
    // containing at least one Assert instruction.
    let chunk = crate::Chunk::compile("test \"ok\" { assert(true) }").unwrap();

    assert_eq!(chunk.tests().len(), 1);
    let idx = chunk.tests()[0].function_idx;
    let func = &chunk.functions[idx];
    assert!(
        func.instructions
            .iter()
            .any(|i| matches!(i, Instruction::Assert)),
        "test body should contain an Assert instruction, got {:?}",
        func.instructions
    );
}

// ---------------------------------------------------------------------------
// List type checking and opcode emission
// ---------------------------------------------------------------------------

#[test]
fn list_literal_emits_make_list() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2, 3]").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::MakeList(3))),
        "expected MakeList(3), got {:?}",
        chunk.instructions
    );
}

#[test]
fn list_index_emits_list_get() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2, 3]\nlet y = xs[0]").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::ListGet)),
        "expected ListGet, got {:?}",
        chunk.instructions
    );
}

#[test]
fn list_index_assignment_emits_list_set() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2, 3]\nxs[0] = 42").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::ListSet)),
        "expected ListSet, got {:?}",
        chunk.instructions
    );
}

#[test]
fn list_len_method_emits_call_list_method() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2, 3]\nlet n = xs.len()").unwrap();
    let expected = ListMethod::Len as u8;
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::CallListMethod(id, 0) if *id == expected))
    );
}

#[test]
fn list_push_method_emits_call_list_method() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1]\nxs.push(2)").unwrap();
    let expected = ListMethod::Push as u8;
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::CallListMethod(id, 1) if *id == expected))
    );
}

#[test]
fn list_pop_method_emits_call_list_method() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2]\nlet last = xs.pop()").unwrap();
    let expected = ListMethod::Pop as u8;
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::CallListMethod(id, 0) if *id == expected))
    );
}

#[test]
fn unknown_list_method_is_compile_error() {
    let errors = crate::Chunk::compile("let xs: [int] = [1]\nxs.frobnicate()").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("unknown list method `frobnicate`")),
        "expected unknown-method error, got {errors:?}"
    );
}

#[test]
fn list_method_wrong_arity_is_error() {
    let errors = crate::Chunk::compile("let xs: [int] = [1]\nxs.len(99)").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("list `len` takes 0 argument(s)")),
        "expected arity error, got {errors:?}"
    );
}

#[test]
fn heterogeneous_list_literal_is_type_error() {
    let errors = crate::Chunk::compile(r#"let xs = [1, "hello"]"#).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("list element type mismatch")),
        "expected a list element type mismatch, got {errors:?}"
    );
}

#[test]
fn empty_list_without_annotation_is_error() {
    let errors = crate::Chunk::compile("let xs = []").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("empty list literal")),
        "expected empty-list error, got {errors:?}"
    );
}

#[test]
fn wrong_element_type_rejected_against_annotation() {
    let errors = crate::Chunk::compile(r#"let xs: [int] = ["a"]"#).unwrap_err();
    assert!(
        errors.iter().any(|e| {
            let s = format!("{e}");
            s.contains("type mismatch") || s.contains("element type")
        }),
        "expected a type mismatch, got {errors:?}"
    );
}

#[test]
fn indexing_non_list_is_error() {
    let errors = crate::Chunk::compile("let x = 5\nlet y = x[0]").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("cannot index into non-list/map type")),
        "expected non-list index error, got {errors:?}"
    );
}

#[test]
fn string_index_is_error() {
    let errors = crate::Chunk::compile("let xs: [int] = [1]\nlet y = xs[\"a\"]").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("list index must be `int`")),
        "expected int-index error, got {errors:?}"
    );
}

#[test]
fn push_argument_type_checked_against_element_type() {
    let errors = crate::Chunk::compile("let xs: [int] = [1]\nxs.push(\"a\")").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("list `push` argument 1 type mismatch")),
        "expected push type mismatch, got {errors:?}"
    );
}

#[test]
fn map_literal_emits_make_map() {
    let chunk = crate::Chunk::compile(r#"let stats: {String: int} = {"hp": 10}"#).unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::MakeMap(1))),
        "expected MakeMap(1), got {:?}",
        chunk.instructions
    );
}

#[test]
fn map_index_emits_map_get() {
    let chunk =
        crate::Chunk::compile("let stats: {String: int} = {\"hp\": 10}\nlet hp = stats[\"hp\"]")
            .unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::MapGet)),
        "expected MapGet, got {:?}",
        chunk.instructions
    );
}

#[test]
fn map_index_assignment_emits_map_set() {
    let chunk = crate::Chunk::compile("let stats: {String: int} = {}\nstats[\"hp\"] = 10").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::MapSet)),
        "expected MapSet, got {:?}",
        chunk.instructions
    );
}

#[test]
fn empty_map_without_annotation_is_error() {
    let errors = crate::Chunk::compile("let stats = {}").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("empty map literal")),
        "expected empty-map error, got {errors:?}"
    );
}

#[test]
fn map_key_type_is_checked() {
    let errors =
        crate::Chunk::compile("let stats: {String: int} = {\"hp\": 10}\nlet hp = stats[1]")
            .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("map key type mismatch")),
        "expected map key type error, got {errors:?}"
    );
}

#[test]
fn map_value_type_is_checked() {
    let errors = crate::Chunk::compile(r#"let stats: {String: int} = {"hp": "full"}"#).unwrap_err();
    assert!(
        errors.iter().any(|e| {
            let s = format!("{e}");
            s.contains("type mismatch") || s.contains("map value type mismatch")
        }),
        "expected map value type error, got {errors:?}"
    );
}

#[test]
fn list_type_round_trips_through_display_name() {
    // Compile a function taking [int] and returning [int] — verify
    // the error rendering for a type mismatch shows `[int]` properly.
    let errors = crate::Chunk::compile(
        "fn head(xs: [int]) -> int { rn xs[0] }\nlet y: [String] = head([1, 2])",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("[int]") || format!("{e}").contains("[String]")),
        "expected list type in error message, got {errors:?}"
    );
}

// ---------------------------------------------------------------------------
// Composition via `use` — test matrix
//
// Exercises `obj Foo { use Bar; ... }`. Every test here is named
// `composition_<category><num>_<what>` so the matrix can be filtered with
// `cargo test --package oryn composition_`.
//
// The categories correspond to the test plan at
// ~/.claude/plans/shiny-hopping-dragon.md:
//   A = sanity, B = conflicts, C = multi-use, D = diamond,
//   E = transitivity, Ep = inherited method bodies, F = self typing,
//   G = override attempts, H = edge cases, I = runtime-observable.
// ---------------------------------------------------------------------------

// ----- A. Sanity: single `use`, no conflicts -----

#[test]
fn composition_a1_single_use_inlines_fields() {
    let src = r#"
obj H {
    hp: int
}
obj G {
    use H
    name: String
}
let g = G { hp: 1, name: "x" }
assert(g.hp == 1)
assert(g.name == "x")
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_a2_single_use_inlines_methods() {
    let src = r#"
obj H {
    hp: int
    fn is_alive(self) -> bool {
        rn self.hp > 0
    }
}
obj G {
    use H
    name: String
}
let g = G { hp: 5, name: "x" }
assert(g.is_alive())
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_a3_own_field_appended_after_inherited() {
    // Invariant check: Guard's flattened `fields` vector must start with
    // Health's fields before Guard's own. This is the layout trick that
    // makes inherited methods see `self.hp` at the same offset.
    let chunk = crate::Chunk::compile("obj H { hp: int }\nobj G {\n    use H\n    name: String\n}")
        .unwrap();
    let g = chunk
        .obj_defs
        .iter()
        .find(|d| d.name == "G")
        .expect("Guard obj def must exist");
    assert_eq!(
        g.fields,
        vec!["hp".to_string(), "name".to_string()],
        "inherited fields must come before own fields, got {:?}",
        g.fields
    );
}

#[test]
fn composition_a4_inherited_method_reads_inherited_field_correctly() {
    let src = r#"
obj H {
    hp: int
    fn read(self) -> int {
        rn self.hp
    }
}
obj G {
    use H
    name: String
}
let g = G { hp: 42, name: "x" }
assert(g.read() == 42)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_a5_inherited_method_writes_inherited_field() {
    let src = r#"
obj H {
    hp: int
    fn heal(self, n: int) {
        self.hp = self.hp + n
    }
}
obj G {
    use H
    name: String
}
let g = G { hp: 10, name: "x" }
g.heal(5)
assert(g.hp == 15)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

// ----- B. Conflict detection -----

#[test]
fn composition_b1_field_conflict_errors() {
    let errors =
        crate::Chunk::compile("obj H { hp: int }\nobj G {\n    use H\n    hp: int\n}").unwrap_err();
    assert!(
        errors.iter().any(|e| format!("{e}").contains("conflict")),
        "expected 'conflict' in error, got {errors:?}"
    );
}

#[test]
fn composition_b2_instance_method_override_succeeds_with_matching_signature() {
    // Overriding an inherited instance method is allowed as long as the
    // signature matches. G's `tick` shadows H's, and `g.tick()` dispatches
    // to G's version.
    let src = r#"
obj H {
    hp: int
    fn tick(self) -> int { rn 1 }
}
obj G {
    use H
    fn tick(self) -> int { rn 2 }
}
let g = G { hp: 0 }
assert(g.tick() == 2)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_b3_static_method_override_with_self_return_covariance() {
    // Static methods can be overridden with the using type as return
    // type even though the inherited signature names the source type.
    let src = r#"
obj H {
    hp: int
    fn make() -> H { rn H { hp: 0 } }
}
obj G {
    use H
    name: String
    fn make() -> G { rn G { hp: 0, name: "x" } }
}
let g = G.make()
assert(g.hp == 0)
assert(g.name == "x")
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_b4_override_with_mismatched_signature_errors() {
    // Override is allowed, but the signature must match. Changing the
    // parameter list or the return type (without Self covariance) errors.
    let errors = crate::Chunk::compile(
        "obj H {\n    hp: int\n    fn tick(self) -> int { rn 1 }\n}\nobj G {\n    use H\n    fn tick(self) -> bool { rn true }\n}",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("override") && format!("{e}").contains("return")),
        "expected override return-type mismatch error, got {errors:?}"
    );
}

#[test]
fn composition_b4b_override_with_mismatched_arity_errors() {
    let errors = crate::Chunk::compile(
        "obj H {\n    hp: int\n    fn tick(self) -> int { rn 1 }\n}\nobj G {\n    use H\n    fn tick(self, n: int) -> int { rn n }\n}",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("override") && format!("{e}").contains("parameter")),
        "expected override arity error, got {errors:?}"
    );
}

#[test]
fn composition_b4c_override_inherited_call_uses_inherited_version() {
    // Non-virtual dispatch: an inherited method that calls `self.other()`
    // is statically bound at the inherited type's compile time. Even if
    // the using type overrides `other`, the inherited body still calls
    // the inherited version. This is the no-virtual-dispatch rule.
    let src = r#"
obj H {
    hp: int
    fn label(self) -> int { rn 1 }
    fn outer(self) -> int { rn self.label() }
}
obj G {
    use H
    fn label(self) -> int { rn 99 }
}
let g = G { hp: 0 }
assert(g.label() == 99)
assert(g.outer() == 1)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_b5_conflict_error_mentions_use_clause() {
    let errors = crate::Chunk::compile(
        "obj Health { hp: int }\nobj Guard {\n    use Health\n    hp: int\n}",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("Health") || format!("{e}").contains("use")),
        "error should name the `use` clause or source type, got {errors:?}"
    );
}

// ----- C. Multi-use -----

#[test]
fn composition_c1_multi_use_parses_disjoint() {
    let src = r#"
obj A { x: int }
obj B { y: int }
obj G {
    use A
    use B
    name: String
}
let g = G { x: 1, y: 2, name: "z" }
assert(g.x == 1)
assert(g.y == 2)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_c2_multi_use_order_preserved() {
    let chunk = crate::Chunk::compile(
        "obj A { x: int }\nobj B { y: int }\nobj G {\n    use A\n    use B\n    name: String\n}",
    )
    .unwrap();
    let g = chunk
        .obj_defs
        .iter()
        .find(|d| d.name == "G")
        .expect("G obj def must exist");
    assert_eq!(
        g.fields,
        vec!["x".to_string(), "y".to_string(), "name".to_string()],
        "expected A's fields, then B's, then own; got {:?}",
        g.fields
    );
}

#[test]
fn composition_c3_multi_use_conflict_across_clauses() {
    let errors = crate::Chunk::compile(
        "obj A { x: int }\nobj B { x: int }\nobj G {\n    use A\n    use B\n}",
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|e| format!("{e}").contains("conflict")),
        "expected cross-use conflict, got {errors:?}"
    );
}

#[test]
fn composition_c5_multi_use_method_collision_resolved_by_own_decl() {
    // Two `use` clauses define `tick`. Without an own declaration this
    // is a hard error (c3-style). With an own declaration the using
    // type's version replaces both, no sig check (the user is picking
    // neither inherited version).
    let src = r#"
obj A {
    x: int
    fn tick(self) -> int { rn 1 }
}
obj B {
    y: int
    fn tick(self) -> bool { rn true }
}
obj G {
    use A
    use B
    fn tick(self) -> String { rn "g" }
}
let g = G { x: 1, y: 2 }
assert(g.tick() == "g")
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_c4_multi_use_each_contributes_methods() {
    let src = r#"
obj A {
    x: int
    fn get_x(self) -> int { rn self.x }
}
obj B {
    y: int
    fn get_y(self) -> int { rn self.y }
}
obj G {
    use A
    use B
}
let g = G { x: 7, y: 9 }
assert(g.get_x() == 7)
assert(g.get_y() == 9)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

// ----- D. Diamond -----

#[test]
fn composition_d1_diamond_same_ancestor_errors() {
    // T is the shared ancestor. M and R both use T. E uses both M and R,
    // which tries to add T's `pos` twice.
    let errors = crate::Chunk::compile(
        "obj T { pos: int }\nobj M {\n    use T\n    vel: int\n}\nobj R {\n    use T\n    sprite: int\n}\nobj E {\n    use M\n    use R\n}",
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|e| format!("{e}").contains("conflict")),
        "diamond should error via conflict, got {errors:?}"
    );
}

#[test]
fn composition_d2_diamond_methods_also_conflict() {
    let errors = crate::Chunk::compile(
        "obj T {\n    pos: int\n    fn origin(self) -> int { rn self.pos }\n}\nobj M {\n    use T\n    vel: int\n}\nobj R {\n    use T\n    sprite: int\n}\nobj E {\n    use M\n    use R\n}",
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|e| format!("{e}").contains("conflict")),
        "method diamond should error, got {errors:?}"
    );
}

// ----- E. Transitivity -----

#[test]
fn composition_e1_chain_a_b_c_fields() {
    let src = r#"
obj A { x: int }
obj B {
    use A
    y: int
}
obj C {
    use B
    z: int
}
let c = C { x: 1, y: 2, z: 3 }
assert(c.x == 1)
assert(c.y == 2)
assert(c.z == 3)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_e2_chain_method_from_grandparent() {
    let src = r#"
obj A {
    x: int
    fn foo(self) -> int { rn self.x }
}
obj B {
    use A
    y: int
}
obj C {
    use B
    z: int
}
let c = C { x: 10, y: 20, z: 30 }
assert(c.foo() == 10)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_e3_chain_depends_on_definition_order() {
    // Using a type before it is defined should error.
    let errors = crate::Chunk::compile(
        "obj C {\n    use B\n    z: int\n}\nobj B {\n    use A\n    y: int\n}\nobj A { x: int }",
    )
    .unwrap_err();
    assert!(
        !errors.is_empty(),
        "expected forward-reference to error, got {errors:?}"
    );
}

// ----- E'. Transitivity — method bodies -----

#[test]
fn composition_ep1_inherited_method_calls_other_inherited_method() {
    let src = r#"
obj H {
    hp: int
    fn a(self) -> int {
        rn self.b() + 1
    }
    fn b(self) -> int {
        rn self.hp
    }
}
obj G {
    use H
    name: String
}
let g = G { hp: 5, name: "x" }
assert(g.a() == 6)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_ep2_inherited_method_cannot_reference_using_types_method() {
    // H's method body references `self.guard_only()`. At H's compile time
    // this method doesn't exist, so type-check must reject it.
    let errors = crate::Chunk::compile(
        "obj H {\n    hp: int\n    fn a(self) -> int { rn self.guard_only() }\n}\nobj G {\n    use H\n    fn guard_only(self) -> int { rn 1 }\n}",
    )
    .unwrap_err();
    assert!(
        !errors.is_empty(),
        "H should not be able to reference a G-only method, got no errors"
    );
}

// ----- F. `self` typing in inherited methods -----

#[test]
fn composition_f1_inherited_method_self_sees_defining_type() {
    // Inside H::is_alive, `self` is typed as H. We observe this indirectly:
    // the method compiles and runs on a Guard instance because the offsets
    // happen to match, but the type annotation inside was H.
    let src = r#"
obj H {
    hp: int
    fn is_alive(self) -> bool { rn self.hp > 0 }
}
obj G {
    use H
}
let g = G { hp: 1 }
assert(g.is_alive())
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_f2_inherited_method_cannot_access_using_types_new_field() {
    // H's method references `self.guard_only`. At H's compile time only
    // H's fields exist — this must error.
    let errors = crate::Chunk::compile(
        "obj H {\n    hp: int\n    fn peek(self) -> int { rn self.guard_only }\n}\nobj G {\n    use H\n    guard_only: int\n}",
    )
    .unwrap_err();
    assert!(
        !errors.is_empty(),
        "H should not see G's new field, got no errors"
    );
}

// ----- G. Override attempts -----

#[test]
fn composition_g2_shadow_field_with_different_type_errors() {
    let errors = crate::Chunk::compile("obj H { x: int }\nobj G {\n    use H\n    x: String\n}")
        .unwrap_err();
    assert!(
        errors.iter().any(|e| format!("{e}").contains("conflict")),
        "retype-shadow should error, got {errors:?}"
    );
}

#[test]
fn composition_g3_guard_adds_non_conflicting_method() {
    let src = r#"
obj H {
    hp: int
    fn a(self) -> int { rn self.hp }
}
obj G {
    use H
    fn b(self) -> int { rn self.hp + 1 }
}
let g = G { hp: 10 }
assert(g.a() == 10)
assert(g.b() == 11)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

// ----- H. Edge cases -----

#[test]
fn composition_h1_use_unknown_type_errors() {
    let errors = crate::Chunk::compile("obj G {\n    use DoesNotExist\n    x: int\n}").unwrap_err();
    assert!(
        !errors.is_empty(),
        "use of unknown type should error, got no errors"
    );
}

#[test]
fn composition_h2_use_self_errors() {
    let errors = crate::Chunk::compile("obj G {\n    use G\n    x: int\n}").unwrap_err();
    assert!(!errors.is_empty(), "self-use should error, got no errors");
}

#[test]
fn composition_h3_use_after_own_field_in_body() {
    // Does the parser accept `use` after a field? If so, does the
    // compiler still flatten inherited-first?
    let result =
        crate::Chunk::compile("obj H { hp: int }\nobj G {\n    name: String\n    use H\n}");
    match result {
        Ok(chunk) => {
            let g = chunk
                .obj_defs
                .iter()
                .find(|d| d.name == "G")
                .expect("G must exist");
            // Even if parseable, inherited fields should still come first
            // in the flattened layout — otherwise inherited methods break.
            assert_eq!(
                g.fields.first().map(|s| s.as_str()),
                Some("hp"),
                "inherited fields must be first regardless of source order; got {:?}",
                g.fields
            );
        }
        Err(errors) => {
            // Also acceptable: parser or compiler rejects the form.
            assert!(
                !errors.is_empty(),
                "expected either success or errors, got empty error list"
            );
        }
    }
}

#[test]
fn composition_h4_empty_use_target() {
    let src = r#"
obj Empty {
}
obj G {
    use Empty
    x: int
}
let g = G { x: 5 }
assert(g.x == 5)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_h6_static_method_inherited_then_called() {
    // H defines a static `make` returning H. Guard uses H. Can you call
    // Guard.make()? And what type does it return — H or G?
    let src = r#"
obj H {
    hp: int
    fn make() -> H { rn H { hp: 99 } }
}
obj G {
    use H
    name: String
}
let h = G.make()
assert(h.hp == 99)
"#;
    let result = crate::Chunk::compile(src);
    match result {
        Ok(chunk) => {
            // If it compiles, run it and see what actually happens.
            let mut vm = crate::VM::new();
            vm.run(&chunk).unwrap();
        }
        Err(errors) => {
            // Also informative — the compiler rejecting this is an honest
            // answer. We just want to know which.
            assert!(!errors.is_empty());
        }
    }
}

// ----- I. Runtime-observable behavior -----

#[test]
fn composition_i1_inherited_damage_mutates_guard_hp() {
    let src = r#"
obj H {
    hp: int
    fn damage(self, n: int) {
        self.hp = self.hp - n
    }
}
obj G {
    use H
    name: String
}
let g = G { hp: 100, name: "x" }
g.damage(30)
assert(g.hp == 70)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_i2_guard_method_calls_inherited_method_on_self() {
    // Mirrors examples/05_composition.on — Guard's own method calls an
    // inherited method via self.
    let src = r#"
obj H {
    hp: int
    fn damage(self, n: int) {
        self.hp = self.hp - n
    }
    fn is_alive(self) -> bool {
        rn self.hp > 0
    }
}
obj G {
    use H
    name: String
    fn take_hit(self, n: int) -> bool {
        self.damage(n)
        rn self.is_alive()
    }
}
let g = G { hp: 50, name: "x" }
assert(g.take_hit(30))
assert(g.hp == 20)
assert(not g.take_hit(100))
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn composition_i3_two_guards_do_not_share_state() {
    let src = r#"
obj H {
    hp: int
    fn damage(self, n: int) { self.hp = self.hp - n }
}
obj G {
    use H
    name: String
}
let a = G { hp: 100, name: "a" }
let b = G { hp: 100, name: "b" }
a.damage(40)
assert(a.hp == 60)
assert(b.hp == 100)
"#;
    let chunk = crate::Chunk::compile(src).unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}
