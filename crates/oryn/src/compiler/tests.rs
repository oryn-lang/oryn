use std::sync::Arc;

use super::types::ModuleTable;
use super::*;

use crate::native::NativeRegistry;
use crate::parser::{BinOp, Expression, Spanned, Statement};

fn spanned<T>(node: T) -> Spanned<T> {
    Spanned { node, span: 0..0 }
}

fn registry() -> Arc<NativeRegistry> {
    Arc::new(NativeRegistry::build())
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

    let output = compile(stmts, ModuleTable::default(), 0, 0, vec![], registry());

    assert_eq!(
        output.instructions,
        vec![Instruction::PushInt(3), Instruction::Pop,]
    );
    assert_eq!(output.instructions.len(), output.spans.len());
}

#[test]
fn expression_statements_are_popped() {
    let stmts = vec![spanned(Statement::Expression(spanned(Expression::Int(1))))];
    let output = compile(stmts, ModuleTable::default(), 0, 0, vec![], registry());

    assert_eq!(output.instructions.last(), Some(&Instruction::Pop));
}

#[test]
fn builtin_calls_are_lowered_to_call_native() {
    let stmts = vec![spanned(Statement::Expression(spanned(Expression::Call {
        target: Box::new(spanned(Expression::Ident("print".to_string()))),
        args: vec![spanned(Expression::Int(1))],
    })))];

    let reg = registry();
    let print_idx = reg.lookup_global("print").unwrap().0;
    let output = compile(stmts, ModuleTable::default(), 0, 0, vec![], reg);

    assert_eq!(
        output.instructions,
        vec![
            Instruction::PushInt(1),
            Instruction::CallNative(print_idx, 1),
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
fn list_len_method_emits_call_native() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2, 3]\nlet n = xs.len()").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::CallNative(_, 0)))
    );
}

#[test]
fn list_push_method_emits_call_native() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1]\nxs.push(2)").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::CallNative(_, 1)))
    );
}

#[test]
fn list_pop_method_emits_call_native() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2]\nlet last = xs.pop()").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::CallNative(_, 0)))
    );
}

#[test]
fn unknown_list_method_is_compile_error() {
    let errors = crate::Chunk::compile("let xs: [int] = [1]\nxs.frobnicate()").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("unknown method `frobnicate`")),
        "expected unknown-method error, got {errors:?}"
    );
}

#[test]
fn list_method_wrong_arity_is_error() {
    let errors = crate::Chunk::compile("let xs: [int] = [1]\nxs.len(99)").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("method `len` takes 0 argument(s)")),
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
            .any(|e| format!("{e}").contains("method `push` argument 1 type mismatch")),
        "expected push type mismatch, got {errors:?}"
    );
}

#[test]
fn map_literal_emits_make_map() {
    let chunk = crate::Chunk::compile(r#"let stats: {string: int} = {"hp": 10}"#).unwrap();
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
        crate::Chunk::compile("let stats: {string: int} = {\"hp\": 10}\nlet hp = stats[\"hp\"]")
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
    let chunk = crate::Chunk::compile("let stats: {string: int} = {}\nstats[\"hp\"] = 10").unwrap();
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
        crate::Chunk::compile("let stats: {string: int} = {\"hp\": 10}\nlet hp = stats[1]")
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
    let errors = crate::Chunk::compile(r#"let stats: {string: int} = {"hp": "full"}"#).unwrap_err();
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
        "fn head(xs: [int]) -> int { return xs[0] }\nlet y: [string] = head([1, 2])",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("[int]") || format!("{e}").contains("[string]")),
        "expected list type in error message, got {errors:?}"
    );
}

// ---------------------------------------------------------------------------
// Composition via `use` — test matrix
//
// Exercises `obj Foo { use Bar; ... }`. Every test here is named
// `composition_<category><num>_<what>` so the matrix can be filtered with
// `cargo test --package oryn composition_`.
// ---------------------------------------------------------------------------

// ----- A. Sanity: single `use`, no conflicts -----

#[test]
fn composition_a1_single_use_inlines_fields() {
    let src = r#"
struct H {
    hp: int
}
struct G {
    use H
    name: string
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
struct H {
    hp: int
    fn is_alive(self) -> bool {
        return self.hp > 0
    }
}
struct G {
    use H
    name: string
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
    let chunk = crate::Chunk::compile(
        "struct H { hp: int }
struct G {\n    use H\n    name: string\n}",
    )
    .unwrap();
    let g = chunk
        .obj_defs
        .iter()
        .find(|d| d.name == "G")
        .expect("Guard struct def must exist");
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
struct H {
    hp: int
    fn read(self) -> int {
        return self.hp
    }
}
struct G {
    use H
    name: string
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
struct H {
    hp: int
    fn heal(mut self, n: int) {
        self.hp = self.hp + n
    }
}
struct G {
    use H
    name: string
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
    let errors = crate::Chunk::compile(
        "struct H { hp: int }
struct G {\n    use H\n    hp: int\n}",
    )
    .unwrap_err();
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
struct H {
    hp: int
    fn tick(self) -> int { return 1 }
}
struct G {
    use H
    fn tick(self) -> int { return 2 }
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
struct H {
    hp: int
    fn make() -> H { return H { hp: 0 } }
}
struct G {
    use H
    name: string
    fn make() -> G { return G { hp: 0, name: "x" } }
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
        "struct H {\n    hp: int\n    fn tick(self) -> int { return 1 }\n}
struct G {\n    use H\n    fn tick(self) -> bool { return true }\n}",
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
        "struct H {\n    hp: int\n    fn tick(self) -> int { return 1 }\n}
struct G {\n    use H\n    fn tick(self, n: int) -> int { return n }\n}",
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
struct H {
    hp: int
    fn label(self) -> int { return 1 }
    fn outer(self) -> int { return self.label() }
}
struct G {
    use H
    fn label(self) -> int { return 99 }
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
        "struct Health { hp: int }
struct Guard {\n    use Health\n    hp: int\n}",
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
struct A { x: int }
struct B { y: int }
struct G {
    use A
    use B
    name: string
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
        "struct A { x: int }
struct B { y: int }
struct G {\n    use A\n    use B\n    name: string\n}",
    )
    .unwrap();
    let g = chunk
        .obj_defs
        .iter()
        .find(|d| d.name == "G")
        .expect("G struct def must exist");
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
        "struct A { x: int }
struct B { x: int }
struct G {\n    use A\n    use B\n}",
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
struct A {
    x: int
    fn tick(self) -> int { return 1 }
}
struct B {
    y: int
    fn tick(self) -> bool { return true }
}
struct G {
    use A
    use B
    fn tick(self) -> string { return "g" }
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
struct A {
    x: int
    fn get_x(self) -> int { return self.x }
}
struct B {
    y: int
    fn get_y(self) -> int { return self.y }
}
struct G {
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
        "struct T { pos: int }
struct M {\n    use T\n    vel: int\n}
struct R {\n    use T\n    sprite: int\n}
struct E {\n    use M\n    use R\n}",
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
        "struct T {\n    pos: int\n    fn origin(self) -> int { return self.pos }\n}
struct M {\n    use T\n    vel: int\n}
struct R {\n    use T\n    sprite: int\n}
struct E {\n    use M\n    use R\n}",
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
struct A { x: int }
struct B {
    use A
    y: int
}
struct C {
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
struct A {
    x: int
    fn foo(self) -> int { return self.x }
}
struct B {
    use A
    y: int
}
struct C {
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
        "struct C {\n    use B\n    z: int\n}
struct B {\n    use A\n    y: int\n}
struct A { x: int }",
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
struct H {
    hp: int
    fn a(self) -> int {
        return self.b() + 1
    }
    fn b(self) -> int {
        return self.hp
    }
}
struct G {
    use H
    name: string
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
        "struct H {\n    hp: int\n    fn a(self) -> int { return self.guard_only() }\n}
struct G {\n    use H\n    fn guard_only(self) -> int { return 1 }\n}",
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
struct H {
    hp: int
    fn is_alive(self) -> bool { return self.hp > 0 }
}
struct G {
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
        "struct H {\n    hp: int\n    fn peek(self) -> int { return self.guard_only }\n}
struct G {\n    use H\n    guard_only: int\n}",
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
    let errors = crate::Chunk::compile(
        "struct H { x: int }
struct G {\n    use H\n    x: string\n}",
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|e| format!("{e}").contains("conflict")),
        "retype-shadow should error, got {errors:?}"
    );
}

#[test]
fn composition_g3_guard_adds_non_conflicting_method() {
    let src = r#"
struct H {
    hp: int
    fn a(self) -> int { return self.hp }
}
struct G {
    use H
    fn b(self) -> int { return self.hp + 1 }
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
    let errors =
        crate::Chunk::compile("struct G {\n    use DoesNotExist\n    x: int\n}").unwrap_err();
    assert!(
        !errors.is_empty(),
        "use of unknown type should error, got no errors"
    );
}

#[test]
fn composition_h2_use_self_errors() {
    let errors = crate::Chunk::compile("struct G {\n    use G\n    x: int\n}").unwrap_err();
    assert!(!errors.is_empty(), "self-use should error, got no errors");
}

#[test]
fn composition_h3_use_after_own_field_in_body() {
    // Does the parser accept `use` after a field? If so, does the
    // compiler still flatten inherited-first?
    let result = crate::Chunk::compile(
        "struct H { hp: int }
struct G {\n    name: string\n    use H\n}",
    );
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
struct Empty {
}
struct G {
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
struct H {
    hp: int
    fn make() -> H { return H { hp: 99 } }
}
struct G {
    use H
    name: string
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
struct H {
    hp: int
    fn damage(mut self, n: int) {
        self.hp = self.hp - n
    }
}
struct G {
    use H
    name: string
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
    // inherited method via self. Both `damage` and `take_hit` mutate
    // (or call something that mutates) self, so they're declared
    // `mut self`. `is_alive` only reads, so it stays plain `fn`.
    let src = r#"
struct H {
    hp: int
    fn damage(mut self, n: int) {
        self.hp = self.hp - n
    }
    fn is_alive(self) -> bool {
        return self.hp > 0
    }
}
struct G {
    use H
    name: string
    fn take_hit(mut self, n: int) -> bool {
        self.damage(n)
        return self.is_alive()
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
struct H {
    hp: int
    fn damage(mut self, n: int) { self.hp = self.hp - n }
}
struct G {
    use H
    name: string
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

// ---------------------------------------------------------------------------
// Mutability — pin the current behaviour of `let`/`val`, parameters, and
// `self` so the design discussion (WARTS W1, W11, W12, W24) has accurate
// ground truth. Cross-checked against `BUGS.md` items 2 and 6.
//
// Each test is named `mut_<category><num>_<what>` so the matrix can be
// filtered with `cargo test --package oryn mut_`.
//
// Category map:
//   A = val sanity (binding-level reassignment)
//   B = val + lists (index assignment, push/pop, len)
//   C = val + maps (gap candidate)
//   D = val rooted through nested fields and indexes
//   E = parameter immutability + W12/W24 error message
//   F = self mutability inside methods
// ---------------------------------------------------------------------------

// ----- A. val sanity -----

#[test]
fn mut_a1_val_binding_cannot_be_reassigned() {
    let errors = crate::Chunk::compile("val x: int = 1\nx = 2").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("cannot reassign val binding")),
        "expected reassign error, got {errors:?}"
    );
}

#[test]
fn mut_a2_let_binding_can_be_reassigned() {
    let chunk = crate::Chunk::compile("let x: int = 1\nx = 2").unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn mut_a3_val_field_assignment_rejected() {
    let errors =
        crate::Chunk::compile("struct C { count: int }\nval c: C = C { count: 1 }\nc.count = 2")
            .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("val binding")),
        "expected val-field error, got {errors:?}"
    );
}

#[test]
fn mut_a4_let_field_assignment_allowed() {
    let chunk =
        crate::Chunk::compile("struct C { count: int }\nlet c: C = C { count: 1 }\nc.count = 2")
            .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

// ----- B. val + lists -----

#[test]
fn mut_b1_val_list_index_assignment_rejected() {
    let errors = crate::Chunk::compile("val xs: [int] = [1, 2, 3]\nxs[0] = 99").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("val binding")),
        "expected val list index error, got {errors:?}"
    );
}

#[test]
fn mut_b2_val_list_push_rejected() {
    let errors = crate::Chunk::compile("val xs: [int] = [1]\nxs.push(2)").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("val binding") && format!("{e}").contains("push")),
        "expected val list push error, got {errors:?}"
    );
}

#[test]
fn mut_b3_val_list_pop_rejected() {
    let errors = crate::Chunk::compile("val xs: [int] = [1, 2]\nlet y = xs.pop()").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("val binding") && format!("{e}").contains("pop")),
        "expected val list pop error, got {errors:?}"
    );
}

#[test]
fn mut_b4_val_list_len_allowed() {
    // `len` is read-only, so val should not block it.
    let chunk = crate::Chunk::compile("val xs: [int] = [1, 2, 3]\nlet n = xs.len()").unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn mut_b5_val_list_read_via_index_allowed() {
    let chunk = crate::Chunk::compile("val xs: [int] = [1, 2, 3]\nlet y = xs[0]").unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

// ----- C. val + maps (gap candidate) -----

#[test]
fn mut_c1_val_map_index_assignment_rejected() {
    // The IndexAssignment check at compiler/stmt.rs:242 fires regardless
    // of the receiver type, so this should error today. Pin it to be sure.
    let errors =
        crate::Chunk::compile("val m: {string: int} = {\"a\": 1}\nm[\"a\"] = 2").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("val binding")),
        "expected val map index error, got {errors:?}"
    );
}

#[test]
fn mut_c2_val_map_read_via_index_allowed() {
    let chunk =
        crate::Chunk::compile("val m: {string: int} = {\"a\": 1}\nlet v = m[\"a\"]").unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

// ----- D. val rooted through nested fields and indexes -----

#[test]
fn mut_d1_val_nested_field_assignment_rejected() {
    let errors = crate::Chunk::compile(
        "struct Inner { x: int }
struct Outer { inner: Inner }\nval o: Outer = Outer { inner: Inner { x: 1 } }\no.inner.x = 5",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("val binding")),
        "expected val nested field error, got {errors:?}"
    );
}

#[test]
fn mut_d2_val_field_then_list_push_rejected() {
    // BUGS.md item 6: bag.xs.push(2) on a val bag.
    let errors = crate::Chunk::compile(
        "struct Bag { xs: [int] }\nval bag: Bag = Bag { xs: [1] }\nbag.xs.push(2)",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("val binding") && format!("{e}").contains("push")),
        "expected val nested list push error, got {errors:?}"
    );
}

#[test]
fn mut_d3_val_list_index_field_assignment_rejected() {
    // val xs[0].field = ... — index then field on a val root.
    let errors = crate::Chunk::compile(
        "struct C { count: int }\nval xs: [C] = [C { count: 1 }]\nxs[0].count = 2",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("val binding")),
        "expected val list-then-field error, got {errors:?}"
    );
}

#[test]
fn mut_d4_val_field_then_index_assignment_rejected() {
    // val o.xs[0] = ... — field then index on a val root.
    let errors = crate::Chunk::compile(
        "struct Bag { xs: [int] }\nval bag: Bag = Bag { xs: [1] }\nbag.xs[0] = 9",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("val binding")),
        "expected val field-then-index error, got {errors:?}"
    );
}

#[test]
fn mut_d5_val_user_method_mutation_rejected() {
    // The mutability cluster closed this gap: a `mut self` method
    // cannot be called on a val-rooted receiver. The val-root walker
    // at the user-method call site rejects it before any code is
    // emitted. (When the method is plain `fn`, writing self.count
    // is rejected at the method's own definition site, not the call
    // site — different error path, same end result.)
    let errors = crate::Chunk::compile(
        "struct C { count: int\n    fn bump(mut self) { self.count = self.count + 1 } }\nval c: C = C { count: 1 }\nc.bump()",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("call mutating method `bump`")
                && format!("{e}").contains("val binding")),
        "expected val-rooted mut self rejection, got {errors:?}"
    );
}

// ----- E. parameter immutability (W12) and error message (W24) -----

#[test]
fn mut_e1_function_param_field_assignment_rejected() {
    let errors = crate::Chunk::compile(
        "struct C { count: int }\nfn bump(c: C) { c.count = c.count + 1 }\nlet x: C = C { count: 1 }\nbump(x)",
    )
    .unwrap_err();
    // After W24 fix: the error names the binding kind (parameter)
    // accurately, instead of lying about "val binding".
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("parameter `c`")
                && format!("{e}").contains("immutable")),
        "expected source-accurate parameter error, got {errors:?}"
    );
}

#[test]
fn mut_e2_function_param_list_push_rejected() {
    let errors =
        crate::Chunk::compile("fn add(xs: [int]) { xs.push(1) }\nlet ys: [int] = [0]\nadd(ys)")
            .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("parameter `xs`") && format!("{e}").contains("push")),
        "expected source-accurate parameter-list-push error, got {errors:?}"
    );
}

#[test]
fn mut_e3_method_param_field_assignment_rejected() {
    // Same rule applies inside methods to non-self params.
    let errors = crate::Chunk::compile(
        "struct C { count: int }
struct Hub {\n    fn bump(self, c: C) { c.count = c.count + 1 }\n}",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("parameter `c`")
                && format!("{e}").contains("immutable")),
        "expected source-accurate method-parameter error, got {errors:?}"
    );
}

#[test]
fn mut_e4_function_param_reassignment_rejected() {
    // What about reassigning the parameter binding itself?
    let errors = crate::Chunk::compile("fn f(n: int) { n = 5 }").unwrap_err();
    assert!(
        !errors.is_empty(),
        "expected error reassigning a parameter, got no errors"
    );
}

// ----- F. self mutability inside methods -----

#[test]
fn mut_f1_self_field_assignment_allowed_in_mut_fn() {
    // `mut self` is the opt-in for self mutation. Plain `fn` rejects.
    let chunk = crate::Chunk::compile(
        "struct C { count: int\n    fn bump(mut self) { self.count = self.count + 1 } }\nlet c: C = C { count: 1 }\nc.bump()",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn mut_f1b_self_field_assignment_rejected_in_plain_fn() {
    let errors = crate::Chunk::compile(
        "struct C { count: int\n    fn bump(self) { self.count = self.count + 1 } }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("non-mutating method")
                && format!("{e}").contains("`mut self`")),
        "expected non-mut self-mutation rejection, got {errors:?}"
    );
}

#[test]
fn mut_f2_self_field_read_allowed() {
    let chunk = crate::Chunk::compile(
        "struct C { count: int\n    fn get(self) -> int { return self.count } }\nlet c: C = C { count: 5 }\nlet n = c.get()",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn mut_f3_self_list_field_push_allowed_in_mut_fn() {
    // mut self permits mutating methods on list fields of self.
    let chunk = crate::Chunk::compile(
        "struct Bag { xs: [int]\n    fn add(mut self, n: int) { self.xs.push(n) } }\nlet b: Bag = Bag { xs: [1] }\nb.add(2)",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn mut_f3b_self_list_field_push_rejected_in_plain_fn() {
    let errors = crate::Chunk::compile(
        "struct Bag { xs: [int]\n    fn add(self, n: int) { self.xs.push(n) } }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("`push`") && format!("{e}").contains("`mut self`")),
        "expected non-mut list-push rejection, got {errors:?}"
    );
}

#[test]
fn mut_f4_self_reassignment_rejected() {
    // The mutability cluster Step 1 closed this gap: `self = ...` is
    // now always a compile error, even inside what will eventually be
    // a `mut self` method. The local for self is conceptually a
    // receiver borrow; rebinding it would silently no-op for the
    // caller.
    let errors = crate::Chunk::compile(
        "struct C { count: int\n    fn replace(self) { self = C { count: 99 } } }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("cannot reassign `self`")),
        "expected self-reassignment error, got {errors:?}"
    );
}

#[test]
fn mut_f5_plain_fn_cannot_call_mut_fn_on_self() {
    // The fn-from-mut-fn rule: a plain `fn` method cannot call a
    // `mut self` method on `self`. The plain method's contract says
    // "I don't mutate"; calling out to a mutating sibling would
    // break it for any val caller relying on the contract.
    let errors = crate::Chunk::compile(
        "struct C {\n    count: int\n    fn bump(mut self) { self.count = self.count + 1 }\n    fn outer(self) { self.bump() }\n}",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("call mutating method `bump`")
                && format!("{e}").contains("non-mutating method")),
        "expected fn-from-mut-fn rejection, got {errors:?}"
    );
}

#[test]
fn mut_f6_mut_fn_can_call_mut_fn_on_self() {
    // The positive case: a `mut self` method can call other `mut self`
    // methods on `self`. This is what makes the override-extends-
    // inherited-behavior pattern work in examples/05_composition.on.
    let chunk = crate::Chunk::compile(
        "struct C {\n    count: int\n    fn bump(mut self) { self.count = self.count + 1 }\n    fn outer(mut self) { self.bump() }\n}\nlet c: C = C { count: 1 }\nc.outer()\nassert(c.count == 2)",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

// ----- G. mut parameters and override asymmetry -----

#[test]
fn mut_g1_mut_param_allows_field_assignment() {
    // A function declared with `mut x: T` can mutate the parameter's
    // fields. The caller may pass a `let` binding (rejected for `val`
    // bindings, see g3).
    let chunk = crate::Chunk::compile(
        "struct C { count: int }\nfn bump(mut c: C) { c.count = c.count + 1 }\nlet x: C = C { count: 1 }\nbump(x)\nassert(x.count == 2)",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn mut_g2_mut_param_allows_list_push() {
    let chunk = crate::Chunk::compile(
        "fn add(mut xs: [int], n: int) { xs.push(n) }\nlet ys: [int] = [1]\nadd(ys, 2)\nassert(ys.len() == 2)",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn mut_g3_val_arg_to_mut_param_rejected() {
    let errors = crate::Chunk::compile(
        "struct C { count: int }\nfn bump(mut c: C) { c.count = c.count + 1 }\nval x: C = C { count: 1 }\nbump(x)",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("pass to mut parameter")
                && format!("{e}").contains("val binding")),
        "expected val-into-mut-param rejection, got {errors:?}"
    );
}

#[test]
fn mut_g4_let_arg_to_mut_param_allowed() {
    // The positive complement of g3.
    let chunk = crate::Chunk::compile(
        "struct C { count: int }\nfn bump(mut c: C) { c.count = c.count + 1 }\nlet x: C = C { count: 1 }\nbump(x)",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

// ----- H. mut self override asymmetry -----

#[test]
fn mut_h1_override_mut_fn_with_plain_fn_allowed() {
    // Stricter override: the inherited method allows mutation; the
    // override doesn't. Val callers benefit — they couldn't call the
    // inherited mut self, but they CAN call the plain override.
    let chunk = crate::Chunk::compile(
        "struct Parent {\n    n: int\n    fn touch(mut self) { self.n = self.n + 1 }\n}
struct Child {\n    use Parent\n    fn touch(self) {}\n}\nval c: Child = Child { n: 0 }\nc.touch()",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn mut_h2_override_plain_fn_with_mut_fn_rejected() {
    // Looser override: the inherited contract said "no mutation", the
    // override would let it happen. Val callers relying on the parent
    // contract would be silently lied to.
    let errors = crate::Chunk::compile(
        "struct Parent {\n    n: int\n    fn touch(self) {}\n}
struct Child {\n    use Parent\n    fn touch(mut self) { self.n = self.n + 1 }\n}",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("cannot widen mutation contract")),
        "expected mut-widening rejection, got {errors:?}"
    );
}

#[test]
fn mut_h3_signature_mut_must_match_implementation() {
    // A `mut self` signature requires a `mut self` implementation.
    // A plain `self` impl that satisfies the shape of a `mut self`
    // signature is rejected — the mutability contract is part of
    // the type.
    let errors = crate::Chunk::compile(
        "struct Healable {\n    fn heal(mut self, n: int)\n}
struct Health {\n    hp: int\n    fn heal(self, n: int) {}\n}
struct Player {\n    use Healable\n    use Health\n}",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("mutability mismatch")
                && format!("{e}").contains("`mut self`")),
        "expected sig mutability mismatch, got {errors:?}"
    );
}

// ---------------------------------------------------------------------------
// W9 — `orelse` is right-associative so chains compose naturally.
//
// Before this fix, `a orelse b orelse c` parsed left-associatively as
// `(a orelse b) orelse c`, which type-checked the inner expression as
// non-nillable T and then rejected the outer `orelse` for having a
// non-nillable left operand. The user had to write
// `a orelse (b orelse c)` explicitly. The parser is now
// right-associative, so the natural form works.
// ---------------------------------------------------------------------------

#[test]
fn orelse_chain_three_elements_compiles_and_runs() {
    // All three sources are nil except the last; the chain should
    // produce the last value (3). Under the old left-associative
    // rule this would not even compile.
    let chunk = crate::Chunk::compile(
        "fn first() -> maybe int { return nil }\n\
         fn second() -> maybe int { return nil }\n\
         fn third() -> int { return 3 }\n\
         let x: int = first() orelse second() orelse third()\n\
         assert(x == 3)",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn orelse_chain_short_circuits_at_first_non_nil() {
    // The chain stops at the first non-nil source. With `first`
    // returning a real value, `second` and `third` should not be
    // observable in the result.
    let chunk = crate::Chunk::compile(
        "fn first() -> maybe int { return 1 }\n\
         fn second() -> maybe int { return 2 }\n\
         fn third() -> int { return 3 }\n\
         let x: int = first() orelse second() orelse third()\n\
         assert(x == 1)",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

#[test]
fn orelse_chain_four_elements() {
    // Pin associativity beyond the trivial 3-case. Four nillable
    // sources, only the third has a value. Right-associative
    // grouping `a orelse (b orelse (c orelse d))` short-circuits at
    // the first non-nil reachable from the right.
    let chunk = crate::Chunk::compile(
        "fn a() -> maybe int { return nil }\n\
         fn b() -> maybe int { return nil }\n\
         fn c() -> maybe int { return 7 }\n\
         fn d() -> int { return 99 }\n\
         let x: int = a() orelse b() orelse c() orelse d()\n\
         assert(x == 7)",
    )
    .unwrap();
    let mut vm = crate::VM::new();
    vm.run(&chunk).unwrap();
}

// ---------------------------------------------------------------------------
// Enums (Slice 1+2): declarations, constructors, value representation,
// structural equality, and discriminant-based match dispatch. Payload
// bindings (Slice 3) and full exhaustiveness over payload shapes
// (Slice 4) are deliberately not exercised here — only what the
// initial slice claims to support.
//
// Naming convention: `enum_<area>_<scenario>`.
//   - `enum_decl_*`        — enum declaration shape and validation.
//   - `enum_ctor_*`        — constructor expressions and field handling.
//   - `enum_eq_*`          — structural equality semantics.
//   - `enum_match_*`       — match expression dispatch and type rules.
//   - `enum_type_*`        — enum types in annotations / function sigs.
//   - `enum_print_*`       — Display output for nullary and payload values.
// ---------------------------------------------------------------------------

// ----- enum_decl: declaration shape and validation -----

#[test]
fn enum_decl_nullary_only_compiles() {
    // Bare enum with three nullary variants is valid.
    crate::Chunk::compile("enum Color { Red\n Green\n Blue }").unwrap();
}

#[test]
fn enum_decl_payload_variants_compile() {
    // Variants may carry named-field payloads with arbitrary types.
    crate::Chunk::compile(
        "enum Shape {\n\
            Circle { radius: float }\n\
            Rect { w: int, h: int }\n\
            Point\n\
         }",
    )
    .unwrap();
}

#[test]
fn enum_decl_empty_body_rejected() {
    // The parser rejects an empty `enum Foo { }` — there's no point
    // declaring a sum type with no inhabitants.
    let errors = crate::Chunk::compile("enum Empty { }").unwrap_err();
    assert!(
        !errors.is_empty(),
        "expected an error for empty enum, got none"
    );
}

#[test]
fn enum_decl_duplicate_variant_rejected() {
    // Two variants with the same name in the same enum is a hard error.
    let errors = crate::Chunk::compile("enum Foo { A\n A }").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("duplicate variant")),
        "expected duplicate variant error, got {errors:?}"
    );
}

#[test]
fn enum_decl_duplicate_field_in_variant_rejected() {
    // Duplicate field names within a single variant payload — same
    // rule as struct fields.
    let errors = crate::Chunk::compile("enum Foo { Bar { x: int, x: int } }").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("duplicate field")),
        "expected duplicate field error, got {errors:?}"
    );
}

#[test]
fn enum_decl_unknown_field_type_rejected() {
    // A payload field referencing an undefined type produces a
    // resolved-type error attached to the field.
    let errors = crate::Chunk::compile("enum Foo { Bar { x: NotAType } }").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("undefined type")),
        "expected undefined-type error, got {errors:?}"
    );
}

// ----- enum_ctor: constructor expressions -----

#[test]
fn enum_ctor_nullary_compiles_and_runs() {
    let chunk = crate::Chunk::compile(
        "enum Color { Red\n Green\n Blue }\n\
         let c = Color.Red\n\
         assert(c == Color.Red)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_ctor_payload_compiles_and_runs() {
    let chunk = crate::Chunk::compile(
        "enum Shape { Circle { radius: float } }\n\
         let s = Shape.Circle { radius: 1.5 }\n\
         assert(s == Shape.Circle { radius: 1.5 })",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_ctor_field_order_does_not_matter() {
    // Payload fields are reordered to declaration order before MakeEnum,
    // so writing them out of order is fine and produces equal values.
    let chunk = crate::Chunk::compile(
        "enum P { Pair { x: int, y: int } }\n\
         let a = P.Pair { x: 1, y: 2 }\n\
         let b = P.Pair { y: 2, x: 1 }\n\
         assert(a == b)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_ctor_missing_field_rejected() {
    let errors =
        crate::Chunk::compile("enum P { Pair { x: int, y: int } }\nlet p = P.Pair { x: 1 }")
            .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("missing field `y`")),
        "expected missing field error, got {errors:?}"
    );
}

#[test]
fn enum_ctor_unknown_field_rejected() {
    let errors = crate::Chunk::compile(
        "enum P { Pair { x: int, y: int } }\nlet p = P.Pair { x: 1, y: 2, z: 3 }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("unknown field `z`")),
        "expected unknown field error, got {errors:?}"
    );
}

#[test]
fn enum_ctor_duplicate_field_rejected() {
    let errors = crate::Chunk::compile(
        "enum P { Pair { x: int, y: int } }\nlet p = P.Pair { x: 1, x: 2, y: 3 }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("duplicate field `x`")),
        "expected duplicate field error, got {errors:?}"
    );
}

#[test]
fn enum_ctor_field_type_mismatch_rejected() {
    let errors = crate::Chunk::compile("enum P { Pair { x: int } }\nlet p = P.Pair { x: \"hi\" }")
        .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("type mismatch")),
        "expected field type mismatch, got {errors:?}"
    );
}

#[test]
fn enum_ctor_nullary_with_field_block_rejected() {
    // Nullary variants must be referenced as bare paths — supplying
    // a `{ }` block is a hard error.
    let errors =
        crate::Chunk::compile("enum Color { Red }\nlet c = Color.Red { x: 1 }").unwrap_err();
    assert!(
        errors.iter().any(|e| format!("{e}").contains("nullary")),
        "expected nullary-with-fields error, got {errors:?}"
    );
}

#[test]
fn enum_ctor_payload_without_field_block_rejected() {
    // The dual of the previous test: a payload variant referenced
    // as a bare path is missing required fields.
    let errors = crate::Chunk::compile("enum P { Pair { x: int } }\nlet p = P.Pair").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("requires fields")
                || format!("{e}").contains("missing field")),
        "expected payload-required error, got {errors:?}"
    );
}

#[test]
fn enum_ctor_unknown_variant_rejected() {
    let errors = crate::Chunk::compile("enum Color { Red }\nlet c = Color.Blue").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("no variant `Blue`")),
        "expected unknown variant error, got {errors:?}"
    );
}

// ----- enum_eq: structural equality -----

#[test]
fn enum_eq_same_nullary_variant_equal() {
    let chunk = crate::Chunk::compile("enum Color { Red\n Green }\nassert(Color.Red == Color.Red)")
        .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_eq_different_variants_unequal() {
    let chunk =
        crate::Chunk::compile("enum Color { Red\n Green }\nassert(Color.Red != Color.Green)")
            .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_eq_same_payload_equal() {
    // Two independently constructed payload values with the same
    // contents compare equal — this is structural, not identity.
    let chunk = crate::Chunk::compile(
        "enum P { Pair { x: int, y: int } }\n\
         let a = P.Pair { x: 1, y: 2 }\n\
         let b = P.Pair { x: 1, y: 2 }\n\
         assert(a == b)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_eq_different_payload_unequal() {
    let chunk = crate::Chunk::compile(
        "enum P { Pair { x: int, y: int } }\n\
         let a = P.Pair { x: 1, y: 2 }\n\
         let b = P.Pair { x: 1, y: 3 }\n\
         assert(a != b)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

// ----- enum_match: dispatch and type rules -----

#[test]
fn enum_match_dispatches_to_correct_arm() {
    let chunk = crate::Chunk::compile(
        "enum Color { Red\n Green\n Blue }\n\
         fn name(c: Color) -> string {\n\
             return match c {\n\
                 Color.Red => \"red\"\n\
                 Color.Green => \"green\"\n\
                 Color.Blue => \"blue\"\n\
             }\n\
         }\n\
         assert(name(Color.Red) == \"red\")\n\
         assert(name(Color.Green) == \"green\")\n\
         assert(name(Color.Blue) == \"blue\")",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_match_with_wildcard_catches_remaining() {
    // Note: Oryn's `!` is the error-unwrap operator, not boolean
    // negation, so the expected-false branches use `== false`.
    let chunk = crate::Chunk::compile(
        "enum Color { Red\n Green\n Blue }\n\
         fn is_red(c: Color) -> bool {\n\
             return match c {\n\
                 Color.Red => true\n\
                 _ => false\n\
             }\n\
         }\n\
         assert(is_red(Color.Red))\n\
         assert(is_red(Color.Green) == false)\n\
         assert(is_red(Color.Blue) == false)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_match_used_as_expression_in_let() {
    // Match-as-expression: the match's value flows directly into a
    // let binding. This is the canonical reason for match being an
    // expression rather than a statement.
    let chunk = crate::Chunk::compile(
        "enum Color { Red\n Green }\n\
         let c = Color.Red\n\
         let n: int = match c {\n\
             Color.Red => 1\n\
             Color.Green => 2\n\
         }\n\
         assert(n == 1)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_match_payload_variant_dispatches_correctly() {
    // Even with payload variants in the enum, Slice 1+2 dispatch only
    // looks at the discriminant — the payload comes along for the ride
    // but isn't bound here. Slice 3 will add bindings.
    let chunk = crate::Chunk::compile(
        "enum FsResult {\n\
             Ok { content: string }\n\
             NotFound\n\
         }\n\
         let r = FsResult.Ok { content: \"hi\" }\n\
         let label: string = match r {\n\
             FsResult.Ok => \"got\"\n\
             FsResult.NotFound => \"missing\"\n\
         }\n\
         assert(label == \"got\")",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_match_non_exhaustive_rejected() {
    // Slice 4 polish: missing variants are listed fully
    // qualified (`Color.Blue` not bare `Blue`).
    let errors = crate::Chunk::compile(
        "enum Color { Red\n Green\n Blue }\n\
         let c = Color.Red\n\
         let n: int = match c {\n\
             Color.Red => 1\n\
             Color.Green => 2\n\
         }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("non-exhaustive")
                && format!("{e}").contains("Color.Blue")),
        "expected non-exhaustive error mentioning Color.Blue, got {errors:?}"
    );
}

#[test]
fn enum_match_wildcard_makes_exhaustive() {
    // The exhaustiveness check accepts a `_` arm as a catch-all even
    // when not every named variant is listed.
    crate::Chunk::compile(
        "enum Color { Red\n Green\n Blue }\n\
         let c = Color.Red\n\
         let n: int = match c {\n\
             Color.Red => 1\n\
             _ => 0\n\
         }",
    )
    .unwrap();
}

#[test]
fn enum_match_duplicate_arm_rejected() {
    let errors = crate::Chunk::compile(
        "enum Color { Red\n Green }\n\
         let c = Color.Red\n\
         let n: int = match c {\n\
             Color.Red => 1\n\
             Color.Red => 2\n\
             Color.Green => 3\n\
         }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("duplicate match arm")),
        "expected duplicate arm error, got {errors:?}"
    );
}

#[test]
fn enum_match_unknown_variant_in_pattern_rejected() {
    let errors = crate::Chunk::compile(
        "enum Color { Red\n Green }\n\
         let c = Color.Red\n\
         let n: int = match c {\n\
             Color.Purple => 1\n\
             _ => 0\n\
         }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("no variant `Purple`")),
        "expected unknown-variant error, got {errors:?}"
    );
}

#[test]
fn enum_match_pattern_type_mismatch_rejected() {
    // A pattern naming a variant of a different enum is rejected
    // even if a variant with that name exists somewhere else.
    let errors = crate::Chunk::compile(
        "enum A { X }\n\
         enum B { X }\n\
         let a = A.X\n\
         let n: int = match a {\n\
             B.X => 1\n\
             _ => 0\n\
         }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("does not match scrutinee enum")),
        "expected scrutinee-enum mismatch, got {errors:?}"
    );
}

#[test]
fn enum_match_non_enum_scrutinee_rejected() {
    let errors =
        crate::Chunk::compile("let n = 1\nlet s: string = match n { _ => \"x\" }").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("must be an enum or error union")),
        "expected non-enum scrutinee error, got {errors:?}"
    );
}

#[test]
fn enum_match_arm_result_type_mismatch_rejected() {
    // All arm bodies must produce the same type. The first arm's
    // type is the canonical one; subsequent arms are checked
    // against it.
    let errors = crate::Chunk::compile(
        "enum Color { Red\n Green }\n\
         let c = Color.Red\n\
         let v = match c {\n\
             Color.Red => 1\n\
             Color.Green => \"green\"\n\
         }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("match arm result type mismatch")),
        "expected arm type mismatch, got {errors:?}"
    );
}

#[test]
fn enum_match_unreachable_after_wildcard_rejected() {
    // Anything after a `_` arm is dead code; the compiler reports it.
    let errors = crate::Chunk::compile(
        "enum Color { Red\n Green }\n\
         let c = Color.Red\n\
         let n: int = match c {\n\
             _ => 0\n\
             _ => 1\n\
         }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("unreachable")),
        "expected unreachable arm error, got {errors:?}"
    );
}

// ----- enum_type: type system integration -----

#[test]
fn enum_type_in_function_return_resolves() {
    // Critical regression test for the bug where `-> FsResult` was
    // resolving to Unknown because resolve_type only checked
    // obj_table. The match's body type comes back as String here
    // (not Unknown / 0) only when the function return type
    // resolves correctly.
    let chunk = crate::Chunk::compile(
        "enum R { Ok\n Err }\n\
         fn pick(b: bool) -> R { if b { return R.Ok }\n return R.Err }\n\
         let label: string = match pick(true) {\n\
             R.Ok => \"yes\"\n\
             R.Err => \"no\"\n\
         }\n\
         assert(label == \"yes\")",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_type_in_let_annotation_resolves() {
    let chunk = crate::Chunk::compile(
        "enum Color { Red\n Green }\nlet c: Color = Color.Red\nassert(c == Color.Red)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_type_in_function_parameter_resolves() {
    let chunk = crate::Chunk::compile(
        "enum Color { Red\n Green }\n\
         fn is_red(c: Color) -> bool {\n\
             return match c { Color.Red => true\n _ => false }\n\
         }\n\
         assert(is_red(Color.Red))",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_type_undefined_annotation_rejected() {
    let errors = crate::Chunk::compile("let c: NotAType = 1").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("undefined type `NotAType`")),
        "expected undefined-type error, got {errors:?}"
    );
}

#[test]
fn enum_type_assigned_wrong_enum_rejected() {
    // Cross-enum assignment is a type error: a `Color` slot can't
    // hold a `Mood` value even though both are enums.
    let errors =
        crate::Chunk::compile("enum Color { Red }\nenum Mood { Happy }\nlet c: Color = Mood.Happy")
            .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("expected `Color`")
                && format!("{e}").contains("got `Mood`")),
        "expected cross-enum assignment error, got {errors:?}"
    );
}

// ----- enum_print: Display output via the print() builtin -----

#[test]
fn enum_print_nullary_variant() {
    let chunk = crate::Chunk::compile("enum Color { Red\n Green }\nprint(Color.Red)").unwrap();
    let mut buf: Vec<u8> = Vec::new();
    crate::VM::new().run_with_writer(&chunk, &mut buf).unwrap();
    assert_eq!(String::from_utf8(buf).unwrap(), "Color.Red\n");
}

#[test]
fn enum_print_payload_variant() {
    let chunk =
        crate::Chunk::compile("enum P { Pair { x: int, y: int } }\nprint(P.Pair { x: 1, y: 2 })")
            .unwrap();
    let mut buf: Vec<u8> = Vec::new();
    crate::VM::new().run_with_writer(&chunk, &mut buf).unwrap();
    assert_eq!(String::from_utf8(buf).unwrap(), "P.Pair { x: 1, y: 2 }\n");
}

#[test]
fn enum_print_payload_string_field_quoted() {
    // Strings inside enum payloads print quoted, mirroring Debug
    // output for debuggability.
    let chunk = crate::Chunk::compile(
        "enum FsResult { Ok { content: string } }\nprint(FsResult.Ok { content: \"hi\" })",
    )
    .unwrap();
    let mut buf: Vec<u8> = Vec::new();
    crate::VM::new().run_with_writer(&chunk, &mut buf).unwrap();
    assert_eq!(
        String::from_utf8(buf).unwrap(),
        "FsResult.Ok { content: \"hi\" }\n"
    );
}

// ---------------------------------------------------------------------------
// Slice 3 + 4: payload bindings in match patterns and full
// exhaustiveness checking. Patterns now allow `Variant { field, ... }`
// brace blocks where each binding is either shorthand (`field`) or
// explicit (`field: name`). Slice 4 fully qualifies missing-variant
// names in non-exhaustive errors and rejects wildcard arms that
// appear after every variant has already been covered.
// ---------------------------------------------------------------------------

// ----- enum_bind: shorthand and explicit payload bindings -----

#[test]
fn enum_bind_shorthand_single_field() {
    // The simplest case: one field, shorthand binds it under its
    // own name. The body uses the binding via string interp.
    let chunk = crate::Chunk::compile(
        "enum P { Pair { x: int } }\n\
         let p = P.Pair { x: 7 }\n\
         let v: int = match p { P.Pair { x } => x }\n\
         assert(v == 7)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_bind_shorthand_multiple_fields() {
    let chunk = crate::Chunk::compile(
        "enum Move { Step { dx: int, dy: int } }\n\
         let m = Move.Step { dx: 3, dy: 4 }\n\
         let sum: int = match m { Move.Step { dx, dy } => dx + dy }\n\
         assert(sum == 7)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_bind_explicit_rename() {
    // `field: name` form binds the payload under a different
    // local name. The original `dx`/`dy` names should NOT be in
    // scope.
    let chunk = crate::Chunk::compile(
        "enum Move { Step { dx: int, dy: int } }\n\
         let m = Move.Step { dx: 3, dy: 4 }\n\
         let sum: int = match m { Move.Step { dx: a, dy: b } => a + b }\n\
         assert(sum == 7)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_bind_mixed_shorthand_and_rename() {
    // Both forms can mix in the same brace block.
    let chunk = crate::Chunk::compile(
        "enum Move { Step { dx: int, dy: int } }\n\
         let m = Move.Step { dx: 3, dy: 4 }\n\
         let sum: int = match m { Move.Step { dx, dy: y } => dx + y }\n\
         assert(sum == 7)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_bind_partial_destructuring_allowed() {
    // Partial destructuring: a 2-field variant can be matched with
    // a single binding; the unlisted field is simply not bound.
    let chunk = crate::Chunk::compile(
        "enum Move { Step { dx: int, dy: int } }\n\
         let m = Move.Step { dx: 3, dy: 4 }\n\
         let just_x: int = match m { Move.Step { dx } => dx }\n\
         assert(just_x == 3)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_bind_tag_only_on_payload_variant_still_works() {
    // Backwards-compatibility: tag-only `Variant` (no braces) on
    // a payload-carrying variant must still be valid. Slice 1+2
    // syntax is preserved.
    let chunk = crate::Chunk::compile(
        "enum Move { Step { dx: int, dy: int } }\n\
         let m = Move.Step { dx: 1, dy: 2 }\n\
         let s: string = match m { Move.Step => \"moved\" }\n\
         assert(s == \"moved\")",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_bind_mixed_arms_some_with_bindings_some_without() {
    let chunk = crate::Chunk::compile(
        "enum FsResult { Ok { content: string } NotFound }\n\
         let r = FsResult.Ok { content: \"hi\" }\n\
         let label: string = match r {\n\
             FsResult.Ok { content } => content\n\
             FsResult.NotFound => \"missing\"\n\
         }\n\
         assert(label == \"hi\")",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_bind_string_payload_typed_correctly() {
    // The binding's static type comes from the variant's declared
    // field type. Verify by using a String binding in a context
    // that requires a String.
    let chunk = crate::Chunk::compile(
        "enum E { Msg { text: string } }\n\
         let e = E.Msg { text: \"hello\" }\n\
         let upper: string = match e { E.Msg { text } => text }\n\
         assert(upper == \"hello\")",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn enum_bind_scope_does_not_leak_across_arms() {
    // Bindings are arm-local. After the arm body, the binding
    // names are no longer in scope, so a later use must produce
    // an undefined-variable error.
    let errors = crate::Chunk::compile(
        "enum P { Pair { x: int } NoPayload }\n\
         let p = P.Pair { x: 1 }\n\
         let r: int = match p { P.Pair { x } => x\n P.NoPayload => 0 }\n\
         print(x)",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("undefined variable `x`")),
        "expected undefined-variable error after match scope, got {errors:?}"
    );
}

#[test]
fn enum_bind_unknown_field_rejected() {
    let errors = crate::Chunk::compile(
        "enum Move { Step { dx: int, dy: int } }\n\
         let m = Move.Step { dx: 1, dy: 2 }\n\
         let v: int = match m { Move.Step { wat } => wat }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("unknown field `wat`")),
        "expected unknown-field error, got {errors:?}"
    );
}

#[test]
fn enum_bind_duplicate_local_name_rejected() {
    // Two bindings with the same local name (after rename
    // resolution) are rejected as a category error.
    let errors = crate::Chunk::compile(
        "enum Move { Step { dx: int, dy: int } }\n\
         let m = Move.Step { dx: 1, dy: 2 }\n\
         let v: int = match m { Move.Step { dx: a, dy: a } => a }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("duplicate binding name `a`")),
        "expected duplicate binding error, got {errors:?}"
    );
}

#[test]
fn enum_bind_nullary_with_braces_rejected() {
    // `Color.Red { }` is a category error: nullary variants have
    // no payload. Same rule as the constructor side.
    let errors = crate::Chunk::compile(
        "enum Color { Red\n Green }\n\
         let c = Color.Red\n\
         let s: string = match c { Color.Red { } => \"r\"\n Color.Green => \"g\" }",
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|e| format!("{e}").contains("nullary")),
        "expected nullary-with-braces error, got {errors:?}"
    );
}

#[test]
fn enum_bind_type_mismatch_in_arm_body_rejected() {
    // Bindings carry the variant's declared field type. Using a
    // String binding where an int is required is a type error.
    let errors = crate::Chunk::compile(
        "enum E { Msg { text: string } }\n\
         let e = E.Msg { text: \"hi\" }\n\
         let n: int = match e { E.Msg { text } => text }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("type mismatch")
                || format!("{e}").contains("expected `int`")),
        "expected binding-type-mismatch error, got {errors:?}"
    );
}

#[test]
fn enum_bind_does_not_pollute_outer_locals() {
    // A binding inside a match arm shadows any outer let with the
    // same name only for the arm's body. After the match, the
    // outer value is unchanged.
    let chunk = crate::Chunk::compile(
        "enum P { Pair { x: int } }\n\
         let x = 100\n\
         let p = P.Pair { x: 7 }\n\
         let inside: int = match p { P.Pair { x } => x }\n\
         assert(inside == 7)\n\
         assert(x == 100)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

// ----- enum_exhaust: Slice 4 exhaustiveness polish -----

#[test]
fn enum_exhaust_missing_vars_fully_qualified() {
    // Slice 4 polish: every missing variant is listed with its
    // full `EnumName.Variant` qualification, not just the bare
    // variant name.
    let errors = crate::Chunk::compile(
        "enum Color { Red\n Green\n Blue }\n\
         let c = Color.Red\n\
         let s: string = match c { Color.Red => \"r\" }",
    )
    .unwrap_err();
    let msg = errors
        .iter()
        .map(|e| format!("{e}"))
        .find(|m| m.contains("non-exhaustive"))
        .expect("expected a non-exhaustive error");
    assert!(
        msg.contains("Color.Green") && msg.contains("Color.Blue"),
        "expected fully-qualified missing variants, got {msg}"
    );
}

#[test]
fn enum_exhaust_dead_wildcard_after_total_coverage_rejected() {
    // Slice 4: a wildcard arm that comes after every variant has
    // been listed explicitly is dead code. Reject it.
    let errors = crate::Chunk::compile(
        "enum Color { Red\n Green\n Blue }\n\
         let c = Color.Red\n\
         let s: string = match c {\n\
             Color.Red => \"r\"\n\
             Color.Green => \"g\"\n\
             Color.Blue => \"b\"\n\
             _ => \"x\"\n\
         }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("wildcard arm is unreachable")
                && format!("{e}").contains("all variants")),
        "expected dead-wildcard error, got {errors:?}"
    );
}

#[test]
fn enum_exhaust_wildcard_first_position_still_unreachable_for_later_arms() {
    // The pre-existing reachability check (a second `_` after the
    // first) should still fire. This pins the older behaviour.
    let errors = crate::Chunk::compile(
        "enum Color { Red\n Green }\n\
         let c = Color.Red\n\
         let s: string = match c { _ => \"a\"\n _ => \"b\" }",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("unreachable")),
        "expected unreachable-after-wildcard error, got {errors:?}"
    );
}

#[test]
fn enum_exhaust_variant_after_wildcard_currently_compiles() {
    // Pin current behaviour: Slice 4 explicitly does NOT detect
    // a variant arm that follows a `_` wildcard as unreachable.
    // The design call was "ship E1 (qualify variants) + E2 (dead
    // wildcard after total coverage) and defer the rest". This
    // test exists so the gap is captured if we ever decide to
    // close it later.
    crate::Chunk::compile(
        "enum Color { Red\n Green\n Blue }\n\
         let c = Color.Red\n\
         let s: string = match c { Color.Red => \"r\"\n _ => \"x\"\n Color.Green => \"g\" }",
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Slice 5 W26: `if` and `if let` lifted to expressions. These tests
// pin the new behaviour: a non-block `if`/`if let` produces a value
// that flows into a let binding or function return, and statement-
// position bare `if` keeps working.
// ---------------------------------------------------------------------------

#[test]
fn if_expression_in_let_binding() {
    let chunk = crate::Chunk::compile(
        "let n = 7\n\
         let label: string = if n > 5 { \"big\" } else { \"small\" }\n\
         assert(label == \"big\")",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn if_expression_in_function_return() {
    let chunk = crate::Chunk::compile(
        "fn classify(x: int) -> string {\n\
             return if x < 0 { \"neg\" } elif x == 0 { \"zero\" } else { \"pos\" }\n\
         }\n\
         assert(classify(-3) == \"neg\")\n\
         assert(classify(0) == \"zero\")\n\
         assert(classify(42) == \"pos\")",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn if_let_expression_in_function_return() {
    let chunk = crate::Chunk::compile(
        "fn describe(m: maybe int) -> string {\n\
             return if let v = m { \"got {v}\" } else { \"nothing\" }\n\
         }\n\
         assert(describe(10) == \"got 10\")\n\
         assert(describe(nil) == \"nothing\")",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn if_branches_must_produce_same_type() {
    let errors = crate::Chunk::compile("let x = if true { 1 } else { \"two\" }").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("must produce the same type")),
        "expected branch type mismatch, got {errors:?}"
    );
}

#[test]
fn bare_if_in_statement_position_still_works() {
    // No else branch — the value is implicitly nil and discarded
    // by the wrapping expression-statement.
    let chunk = crate::Chunk::compile("let n = 5\nif n > 0 { print(\"positive\") }").unwrap();
    crate::VM::new().run(&chunk).unwrap();
}

#[test]
fn if_expression_value_block_takes_last_statement() {
    // The body block has let-bindings followed by an expression;
    // the block's value is the trailing expression.
    let chunk = crate::Chunk::compile(
        "let n = if true {\n\
             let a = 3\n\
             let b = 4\n\
             a + b\n\
         } else {\n\
             0\n\
         }\n\
         assert(n == 7)",
    )
    .unwrap();
    crate::VM::new().run(&chunk).unwrap();
}
