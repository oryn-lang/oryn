//! Integration tests for higher-order list methods. These are
//! emitted by the compiler as bytecode loops over `CallValue` —
//! they don't go through the native registry at runtime.
//!
//! Failures here point at
//! `crates/oryn/src/compiler/expr.rs::compile_list_intrinsic` or
//! the dispatch table in `crates/oryn/src/native/intrinsics.rs`.

mod common;
use common::run;

#[test]
fn map_doubles_each_element() {
    let source = r#"
fn double(x: int) -> int { return x * 2 }
let xs: [int] = [1, 2, 3]
let ys = xs.map(double)
print(ys.len())
print(ys.first())
print(ys.last())
"#;
    assert_eq!(run(source), "3\n2\n6\n");
}

#[test]
fn map_with_anonymous_function() {
    let source = r#"
let xs: [int] = [1, 2, 3]
let ys = xs.map(fn(x: int) -> int { return x * 10 })
print(ys.first())
print(ys.last())
"#;
    assert_eq!(run(source), "10\n30\n");
}

#[test]
fn filter_keeps_matching_elements() {
    let source = r#"
let xs: [int] = [1, 2, 3, 4, 5]
let evens = xs.filter(fn(x: int) -> bool { return x == 2 or x == 4 })
print(evens.len())
print(evens.first())
print(evens.last())
"#;
    assert_eq!(run(source), "2\n2\n4\n");
}

#[test]
fn fold_sums_elements() {
    let source = r#"
let xs: [int] = [1, 2, 3, 4]
let sum = xs.fold(0, fn(acc: int, x: int) -> int { return acc + x })
print(sum)
"#;
    assert_eq!(run(source), "10\n");
}

#[test]
fn each_runs_callback_per_element() {
    let source = r#"
let xs: [int] = [10, 20, 30]
xs.each(fn(x: int) { print(x) })
"#;
    assert_eq!(run(source), "10\n20\n30\n");
}

#[test]
fn find_returns_first_match_or_nil() {
    let source = r#"
let xs: [int] = [1, 5, 10, 15]
let found = xs.find(fn(x: int) -> bool { return x > 7 })
print(found)
"#;
    assert_eq!(run(source), "10\n");
}

#[test]
fn find_returns_nil_when_no_match() {
    let source = r#"
let xs: [int] = [1, 2, 3]
let found = xs.find(fn(x: int) -> bool { return x > 99 })
print(found)
"#;
    assert_eq!(run(source), "nil\n");
}

#[test]
fn any_short_circuits_on_first_match() {
    let source = r#"
let xs: [int] = [1, 2, 3, 4]
print(xs.any(fn(x: int) -> bool { return x == 3 }))
print(xs.any(fn(x: int) -> bool { return x == 99 }))
"#;
    assert_eq!(run(source), "true\nfalse\n");
}

#[test]
fn all_returns_true_when_predicate_holds_for_each() {
    let source = r#"
let xs: [int] = [2, 4, 6]
print(xs.all(fn(x: int) -> bool { return x > 0 }))
print(xs.all(fn(x: int) -> bool { return x > 4 }))
"#;
    assert_eq!(run(source), "true\nfalse\n");
}

#[test]
fn map_filter_chain_via_intermediate_let() {
    // The chain works element-by-element via two intermediates;
    // this exercises both the result-list construction in `map`
    // and the conditional path in `filter`.
    let source = r#"
let xs: [int] = [1, 2, 3, 4, 5]
let doubled = xs.map(fn(x: int) -> int { return x * 2 })
let big = doubled.filter(fn(x: int) -> bool { return x > 4 })
print(big.len())
print(big.first())
"#;
    assert_eq!(run(source), "3\n6\n");
}

#[test]
fn closure_capture_inside_map_callback() {
    // Confirm the FCF capture path is exercised through the
    // intrinsic emitter — `factor` is captured by value at
    // closure construction.
    let source = r#"
fn run() {
    let factor = 10
    let xs: [int] = [1, 2, 3]
    let ys = xs.map(fn(x: int) -> int { return x * factor })
    print(ys.first())
    print(ys.last())
}
run()
"#;
    assert_eq!(run(source), "10\n30\n");
}
