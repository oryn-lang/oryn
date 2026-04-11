//! Integration tests for the unified native method registry —
//! `string`, `range`, `[T]`, and `{K: V}` methods, plus the global
//! free functions (`print`, `len`, `to_string`, `parse_int`, `assert`).
//!
//! Each test compiles and runs a small Oryn snippet that exercises
//! one method, then asserts on the printed output. Failures here
//! point at either the method body in `crates/oryn/src/native/<type>.rs`
//! or the dispatch in `crates/oryn/src/compiler/expr.rs::compile_native_method`.

mod common;
use common::run;

// ---------------------------------------------------------------------------
// String methods
// ---------------------------------------------------------------------------

#[test]
fn string_len_returns_byte_length() {
    assert_eq!(run("print(\"hello\".len())"), "5\n");
}

#[test]
fn string_to_upper() {
    assert_eq!(run("print(\"hello\".to_upper())"), "HELLO\n");
}

#[test]
fn string_to_lower() {
    assert_eq!(run("print(\"HELLO\".to_lower())"), "hello\n");
}

#[test]
fn string_trim_strips_whitespace_both_ends() {
    assert_eq!(run("print(\"  hi  \".trim())"), "hi\n");
}

#[test]
fn string_contains_finds_substring() {
    assert_eq!(run("print(\"hello world\".contains(\"world\"))"), "true\n");
    assert_eq!(run("print(\"hello\".contains(\"xyz\"))"), "false\n");
}

#[test]
fn string_starts_with() {
    assert_eq!(run("print(\"hello\".starts_with(\"he\"))"), "true\n");
    assert_eq!(run("print(\"hello\".starts_with(\"lo\"))"), "false\n");
}

#[test]
fn string_ends_with() {
    assert_eq!(run("print(\"hello\".ends_with(\"lo\"))"), "true\n");
    assert_eq!(run("print(\"hello\".ends_with(\"he\"))"), "false\n");
}

#[test]
fn string_index_of_returns_position_or_nil() {
    assert_eq!(run("print(\"hello\".index_of(\"ll\"))"), "2\n");
    assert_eq!(run("print(\"hello\".index_of(\"zz\"))"), "nil\n");
}

#[test]
fn string_replace_replaces_all_occurrences() {
    assert_eq!(run("print(\"a-b-c\".replace(\"-\", \"+\"))"), "a+b+c\n");
}

#[test]
fn string_split_returns_list_of_strings() {
    assert_eq!(
        run("let parts = \"a,b,c\".split(\",\")\nprint(parts.len())"),
        "3\n"
    );
}

#[test]
fn string_repeat_concatenates_n_copies() {
    assert_eq!(run("print(\"ab\".repeat(3))"), "ababab\n");
    assert_eq!(run("print(\"x\".repeat(0))"), "\n");
}

#[test]
fn string_chars_returns_one_per_char() {
    // Pure ASCII to keep the assertion simple.
    assert_eq!(run("let cs = \"abc\".chars()\nprint(cs.len())"), "3\n");
}

#[test]
fn string_parse_int_succeeds_or_returns_nil() {
    assert_eq!(run("print(\"42\".parse_int())"), "42\n");
    assert_eq!(run("print(\"abc\".parse_int())"), "nil\n");
}

#[test]
fn string_parse_float_succeeds_or_returns_nil() {
    assert_eq!(run("print(\"3.14\".parse_float())"), "3.14\n");
    assert_eq!(run("print(\"abc\".parse_float())"), "nil\n");
}

// ---------------------------------------------------------------------------
// Range methods
// ---------------------------------------------------------------------------

#[test]
fn range_start_and_end() {
    assert_eq!(run("let r = 3..7\nprint(r.start())"), "3\n");
    assert_eq!(run("let r = 3..7\nprint(r.end())"), "7\n");
}

#[test]
fn range_len_exclusive() {
    assert_eq!(run("let r = 3..7\nprint(r.len())"), "4\n");
}

#[test]
fn range_len_inclusive() {
    assert_eq!(run("let r = 3..=7\nprint(r.len())"), "5\n");
}

#[test]
fn range_contains() {
    assert_eq!(run("print((1..5).contains(3))"), "true\n");
    assert_eq!(run("print((1..5).contains(5))"), "false\n");
    assert_eq!(run("print((1..=5).contains(5))"), "true\n");
}

#[test]
fn range_to_list_materializes() {
    assert_eq!(run("let xs = (0..3).to_list()\nprint(xs.len())"), "3\n");
}

// ---------------------------------------------------------------------------
// List methods (non-higher-order)
// ---------------------------------------------------------------------------

#[test]
fn list_len_and_is_empty() {
    assert_eq!(run("let xs: [int] = [1, 2, 3]\nprint(xs.len())"), "3\n");
    assert_eq!(run("let xs: [int] = []\nprint(xs.is_empty())"), "true\n");
    assert_eq!(run("let xs: [int] = [1]\nprint(xs.is_empty())"), "false\n");
}

#[test]
fn list_push_pop_round_trip() {
    let source = r#"
let xs: [int] = [1, 2]
xs.push(3)
print(xs.len())
let last = xs.pop()
print(last)
"#;
    // After push(3), xs is [1,2,3]; pop() returns the most recently
    // pushed element (3) and shrinks the list back to [1,2].
    assert_eq!(run(source), "3\n3\n");
}

#[test]
fn list_pop_on_empty_returns_nil() {
    assert_eq!(run("let xs: [int] = []\nprint(xs.pop())"), "nil\n");
}

#[test]
fn list_insert_remove() {
    let source = r#"
let xs: [int] = [10, 30]
xs.insert(1, 20)
print(xs.len())
let mid = xs.remove(1)
print(mid)
"#;
    assert_eq!(run(source), "3\n20\n");
}

#[test]
fn list_clear_empties_the_list() {
    let source = r#"
let xs: [int] = [1, 2, 3]
xs.clear()
print(xs.is_empty())
"#;
    assert_eq!(run(source), "true\n");
}

#[test]
fn list_contains_and_index_of() {
    let source = r#"
let xs: [int] = [10, 20, 30]
print(xs.contains(20))
print(xs.contains(99))
print(xs.index_of(30))
print(xs.index_of(99))
"#;
    assert_eq!(run(source), "true\nfalse\n2\nnil\n");
}

#[test]
fn list_first_and_last() {
    let source = r#"
let xs: [int] = [10, 20, 30]
print(xs.first())
print(xs.last())
"#;
    assert_eq!(run(source), "10\n30\n");
}

#[test]
fn list_first_last_on_empty_return_nil() {
    let source = r#"
let xs: [int] = []
print(xs.first())
print(xs.last())
"#;
    assert_eq!(run(source), "nil\nnil\n");
}

#[test]
fn list_reverse_in_place() {
    let source = r#"
let xs: [int] = [1, 2, 3]
xs.reverse()
print(xs.first())
print(xs.last())
"#;
    assert_eq!(run(source), "3\n1\n");
}

#[test]
fn list_sort_ints_in_place() {
    let source = r#"
let xs: [int] = [3, 1, 2]
xs.sort()
print(xs.first())
print(xs.last())
"#;
    assert_eq!(run(source), "1\n3\n");
}

#[test]
fn list_concat_returns_new_list() {
    let source = r#"
let xs: [int] = [1, 2]
let ys: [int] = [3, 4]
let zs = xs.concat(ys)
print(zs.len())
"#;
    assert_eq!(run(source), "4\n");
}

#[test]
fn list_push_wrong_element_type_is_compile_error() {
    let errors = oryn::Chunk::compile("let xs: [int] = [1]\nxs.push(\"hi\")").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("argument 1 type mismatch")),
        "expected element type mismatch, got {errors:?}"
    );
}

// ---------------------------------------------------------------------------
// Map methods
// ---------------------------------------------------------------------------

#[test]
fn map_len_and_is_empty() {
    assert_eq!(
        run(r#"let m: {string: int} = {"a": 1, "b": 2}
print(m.len())"#),
        "2\n"
    );
    assert_eq!(
        run("let m: {string: int} = {}\nprint(m.is_empty())"),
        "true\n"
    );
}

#[test]
fn map_contains_key_and_get() {
    let source = r#"
let m: {string: int} = {"hp": 10}
print(m.contains_key("hp"))
print(m.contains_key("mp"))
print(m.get("hp"))
print(m.get("mp"))
"#;
    assert_eq!(run(source), "true\nfalse\n10\nnil\n");
}

#[test]
fn map_insert_replaces_existing_value() {
    let source = r#"
let m: {string: int} = {"hp": 10}
m.insert("hp", 99)
m.insert("mp", 5)
print(m.len())
print(m.get("hp"))
"#;
    assert_eq!(run(source), "2\n99\n");
}

#[test]
fn map_remove_returns_old_value_or_nil() {
    let source = r#"
let m: {string: int} = {"a": 1, "b": 2}
print(m.remove("a"))
print(m.remove("zzz"))
print(m.len())
"#;
    assert_eq!(run(source), "1\nnil\n1\n");
}

#[test]
fn map_clear_empties_the_map() {
    let source = r#"
let m: {string: int} = {"a": 1}
m.clear()
print(m.is_empty())
"#;
    assert_eq!(run(source), "true\n");
}

#[test]
fn map_keys_returns_insertion_order() {
    let source = r#"
let m: {string: int} = {"a": 1, "b": 2}
let ks = m.keys()
print(ks.len())
print(ks.first())
"#;
    assert_eq!(run(source), "2\na\n");
}

#[test]
fn map_values_returns_insertion_order() {
    let source = r#"
let m: {string: int} = {"a": 1, "b": 2}
let vs = m.values()
print(vs.len())
print(vs.first())
"#;
    assert_eq!(run(source), "2\n1\n");
}

// ---------------------------------------------------------------------------
// Globals
// ---------------------------------------------------------------------------

#[test]
fn global_print_handles_multiple_args() {
    assert_eq!(run("print(1, 2, 3)"), "1, 2, 3\n");
}

#[test]
fn global_len_dispatches_on_runtime_shape() {
    assert_eq!(run("print(len(\"hello\"))"), "5\n");
    assert_eq!(run("let xs: [int] = [1, 2, 3]\nprint(len(xs))"), "3\n");
    assert_eq!(
        run(r#"let m: {string: int} = {"a": 1}
print(len(m))"#),
        "1\n"
    );
}

#[test]
fn global_to_string_renders_primitives() {
    assert_eq!(run("print(to_string(42))"), "42\n");
    assert_eq!(run("print(to_string(true))"), "true\n");
}

#[test]
fn global_parse_int_works() {
    assert_eq!(run("print(parse_int(\"42\"))"), "42\n");
    assert_eq!(run("print(parse_int(\"abc\"))"), "nil\n");
}
