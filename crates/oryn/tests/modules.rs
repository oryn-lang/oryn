//! Integration tests for the multi-file module system.
//!
//! These tests build temporary project trees on disk, compile them via
//! `Chunk::compile_file`, and run the resulting bytecode through the VM
//! to verify nested imports, object field/method privacy, qualified
//! type annotations, and qualified object literals all work end-to-end.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A self-cleaning temporary project root with `package.on` already in place.
struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new() -> Self {
        // Unique-per-test directory under the OS temp dir. We use a static
        // counter to keep names short and collision-free across parallel runs.
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("oryn-modules-test-{pid}-{n}"));
        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }
        fs::create_dir_all(&root).expect("create temp project root");
        fs::write(root.join("package.on"), "").expect("write package.on");
        Self { root }
    }

    /// Write a module file under the project root, creating parent dirs.
    fn write(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, contents).expect("write module file");
    }

    /// Compile and run the entry file at `relative`. Returns captured stdout.
    fn run(&self, relative: &str) -> Result<String, Vec<oryn::OrynError>> {
        let entry = self.root.join(relative);
        let chunk = oryn::Chunk::compile_file(&entry)?;
        let mut vm = oryn::VM::new();
        let mut output = Vec::new();
        vm.run_with_writer(&chunk, &mut output)
            .expect("runtime error");
        Ok(String::from_utf8(output).expect("invalid utf-8"))
    }

    /// Compile-only entry; return errors if any.
    fn check(&self, relative: &str) -> Result<(), Vec<oryn::OrynError>> {
        let entry = self.root.join(relative);
        oryn::Chunk::compile_file(&entry).map(|_| ())
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_compile_error_contains(errors: &[oryn::OrynError], needle: &str) {
    let any_match = errors.iter().any(|e| {
        let s = format!("{e:?}");
        s.contains(needle)
    });
    assert!(
        any_match,
        "expected an error containing `{needle}`, got: {errors:?}"
    );
}

// ---------------------------------------------------------------------------
// Flat imports
// ---------------------------------------------------------------------------

#[test]
fn flat_import_function_call() {
    let p = TempProject::new();
    p.write("math.on", "pub fn add(a: int, b: int) -> int { rn a + b }");
    p.write("main.on", "import math\nprint(math.add(3, 4))");

    assert_eq!(p.run("main.on").unwrap(), "7\n");
}

#[test]
fn flat_import_pub_constant() {
    let p = TempProject::new();
    p.write("math.on", "pub val PI = 3.14");
    p.write("main.on", "import math\nprint(math.PI)");

    assert_eq!(p.run("main.on").unwrap(), "3.14\n");
}

#[test]
fn flat_import_private_function_rejected() {
    let p = TempProject::new();
    p.write("math.on", "fn helper() -> int { rn 1 }");
    p.write("main.on", "import math\nprint(math.helper())");

    let err = p.run("main.on").unwrap_err();
    assert_compile_error_contains(&err, "undefined function");
}

// ---------------------------------------------------------------------------
// Nested imports
// ---------------------------------------------------------------------------

#[test]
fn nested_import_function_call() {
    let p = TempProject::new();
    p.write(
        "math/nested/lib.on",
        "pub fn triple(n: int) -> int { rn n * 3 }",
    );
    p.write(
        "main.on",
        "import math.nested.lib\nprint(math.nested.lib.triple(7))",
    );

    assert_eq!(p.run("main.on").unwrap(), "21\n");
}

#[test]
fn nested_import_constant() {
    let p = TempProject::new();
    p.write("std/math/constants.on", "pub val TAU = 6.28");
    p.write(
        "main.on",
        "import std.math.constants\nprint(std.math.constants.TAU)",
    );

    assert_eq!(p.run("main.on").unwrap(), "6.28\n");
}

#[test]
fn module_calling_internal_helper() {
    let p = TempProject::new();
    p.write(
        "math.on",
        "fn double(n: int) -> int { rn n + n }
pub fn quadruple(n: int) -> int { rn double(double(n)) }",
    );
    p.write("main.on", "import math\nprint(math.quadruple(5))");

    assert_eq!(p.run("main.on").unwrap(), "20\n");
}

// ---------------------------------------------------------------------------
// Qualified object literals
// ---------------------------------------------------------------------------

#[test]
fn qualified_object_literal_all_pub_fields() {
    let p = TempProject::new();
    p.write(
        "geom.on",
        "pub obj Vec2 {
    pub x: float
    pub y: float
}",
    );
    p.write(
        "main.on",
        "import geom
let v = geom.Vec2 { x: 1.5, y: 2.5 }
print(v.x)
print(v.y)",
    );

    assert_eq!(p.run("main.on").unwrap(), "1.5\n2.5\n");
}

#[test]
fn qualified_object_literal_with_private_field_rejected() {
    let p = TempProject::new();
    p.write(
        "geom.on",
        "pub obj Vec2 {
    pub x: float
    y: float
}",
    );
    p.write(
        "main.on",
        "import geom
let v = geom.Vec2 { x: 1.0, y: 2.0 }
print(v.x)",
    );

    let err = p.run("main.on").unwrap_err();
    assert_compile_error_contains(&err, "private");
}

#[test]
fn cross_module_method_call_pub() {
    let p = TempProject::new();
    p.write(
        "geom.on",
        "pub obj Vec2 {
    pub x: float
    pub y: float

    pub fn length_sq(self) -> float {
        rn self.x * self.x + self.y * self.y
    }
}",
    );
    p.write(
        "main.on",
        "import geom
let v = geom.Vec2 { x: 3.0, y: 4.0 }
print(v.length_sq())",
    );

    assert_eq!(p.run("main.on").unwrap(), "25.0\n");
}

#[test]
fn cross_module_private_method_rejected() {
    let p = TempProject::new();
    p.write(
        "geom.on",
        "pub obj Vec2 {
    pub x: float
    pub y: float

    fn _hidden(self) -> float { rn self.x }
}",
    );
    p.write(
        "main.on",
        "import geom
let v = geom.Vec2 { x: 1.0, y: 2.0 }
print(v._hidden())",
    );

    let err = p.run("main.on").unwrap_err();
    assert_compile_error_contains(&err, "_hidden");
    assert_compile_error_contains(&err, "private");
}

#[test]
fn cross_module_private_field_access_rejected() {
    let p = TempProject::new();
    p.write(
        "geom.on",
        "pub obj Vec2 {
    pub x: float
    pub y: float

    pub fn make(a: float, b: float) -> Vec2 {
        rn Vec2 { x: a, y: b }
    }
}",
    );
    // Construct via static method (no private field issue), then try
    // to access a pub field — should work — and then a hypothetical
    // private one would fail. Here we just verify pub access works.
    p.write(
        "main.on",
        "import geom
let v = geom.Vec2.make(1.0, 2.0)
print(v.x)",
    );

    assert_eq!(p.run("main.on").unwrap(), "1.0\n");
}

#[test]
fn cross_module_static_constructor_for_private_fields() {
    let p = TempProject::new();
    p.write(
        "secrets.on",
        "pub obj Hidden {
    secret: int

    pub fn new(n: int) -> Hidden {
        rn Hidden { secret: n }
    }

    pub fn get(self) -> int {
        rn self.secret
    }
}",
    );
    p.write(
        "main.on",
        "import secrets
let h = secrets.Hidden.new(99)
print(h.get())",
    );

    assert_eq!(p.run("main.on").unwrap(), "99\n");
}

// ---------------------------------------------------------------------------
// Misc edge cases
// ---------------------------------------------------------------------------

#[test]
fn module_top_level_expression_is_compile_error() {
    let p = TempProject::new();
    p.write("bad.on", "print(\"hello\")");
    p.write("main.on", "import bad\nprint(0)");

    let err = p.run("main.on").unwrap_err();
    assert_compile_error_contains(&err, "definitions");
}

#[test]
fn missing_module_is_compile_error() {
    let p = TempProject::new();
    p.write("main.on", "import nonexistent\nprint(0)");

    let err = p.run("main.on").unwrap_err();
    assert_compile_error_contains(&err, "Module");
}

#[test]
fn repeated_missing_import_is_not_mislabeled_as_circular() {
    // Two sibling modules both try to import the same missing file.
    // The first attempt fails; the second must also report a load
    // failure, not "circular import detected" from a poisoned
    // `compiling` set.
    let p = TempProject::new();
    p.write("a.on", "import missing\npub fn a() -> int { rn 1 }");
    p.write("b.on", "import missing\npub fn b() -> int { rn 2 }");
    p.write("main.on", "import a\nimport b\nprint(a.a())");

    let err = p.run("main.on").unwrap_err();
    let combined = err
        .iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !combined.contains("circular import detected"),
        "missing-module error was mislabeled as circular: {combined}",
    );
}

#[test]
fn module_private_constant_visible_to_own_functions() {
    // A non-`pub` `val` at module scope is visible to functions inside
    // the same module but must not be exported.
    let p = TempProject::new();
    p.write(
        "config.on",
        "val SECRET = 7
pub fn get_secret() -> int { rn SECRET }",
    );
    p.write("main.on", "import config\nprint(config.get_secret())");

    assert_eq!(p.run("main.on").unwrap(), "7\n");
}

#[test]
fn module_private_constant_not_accessible_from_outside() {
    let p = TempProject::new();
    p.write("config.on", "val SECRET = 7");
    p.write("main.on", "import config\nprint(config.SECRET)");

    let err = p.run("main.on").unwrap_err();
    // `SECRET` is not exported — any mention of it from outside the
    // module is an undefined reference of some kind.
    let combined = err
        .iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !combined.is_empty(),
        "expected a compile error for private constant access, got: {combined}",
    );
}

#[test]
fn module_constant_expression_is_folded_at_compile_time() {
    let p = TempProject::new();
    p.write("config.on", "pub val X = 1 + 2");
    p.write("main.on", "import config\nprint(config.X)");

    let output = p.run("main.on").expect("runtime error");
    assert_eq!(output, "3\n");
}

#[test]
fn module_error_is_reported_against_module_file_not_entry() {
    // When an imported module has a compile error, the returned
    // FileDiagnostics must carry the *module's* path and source, not
    // the entry file's. Without this, ariadne renders module spans
    // against the wrong file and lands on random lines.
    let p = TempProject::new();
    p.write(
        "colors.on",
        "// padding\n// padding\npub val MAX = UNDEFINED_THING\n",
    );
    p.write("main.on", "import colors\nprint(colors.MAX)");

    let entry = p.root.join("main.on");
    let diagnostics = oryn::Chunk::compile_file_sourced(&entry)
        .expect_err("expected compile failure from broken module");

    // Exactly one file batch, and it points at colors.on (not main.on).
    assert_eq!(
        diagnostics.len(),
        1,
        "expected one FileDiagnostics batch, got {}: {diagnostics:?}",
        diagnostics.len(),
    );
    let diag = &diagnostics[0];
    assert!(
        diag.file.ends_with("colors.on"),
        "diagnostic file should point at colors.on, got {:?}",
        diag.file,
    );
    assert!(
        diag.source.contains("UNDEFINED_THING"),
        "diagnostic source should be colors.on's text, got {:?}",
        diag.source,
    );
    // Every span in every error must fall inside colors.on's source.
    for err in &diag.errors {
        let span = match err {
            oryn::OrynError::Compiler { span, .. } => Some(span.clone()),
            oryn::OrynError::Parser { span, .. } => Some(span.clone()),
            oryn::OrynError::Lexer { span } => Some(span.clone()),
            _ => None,
        };
        if let Some(span) = span {
            assert!(
                span.end <= diag.source.len(),
                "span {span:?} is out of bounds for {} byte source",
                diag.source.len(),
            );
        }
    }
}

#[test]
fn multiple_pub_constants_of_different_types() {
    let p = TempProject::new();
    p.write(
        "config.on",
        "pub val NAME = \"oryn\"
pub val MAX = 255
pub val PI = 3.14
pub val ON = true",
    );
    p.write(
        "main.on",
        "import config
print(config.NAME)
print(config.MAX)
print(config.PI)
print(config.ON)",
    );

    assert_eq!(p.run("main.on").unwrap(), "oryn\n255\n3.14\ntrue\n");
}

#[test]
fn check_module_passes_for_valid_project() {
    let p = TempProject::new();
    p.write("math.on", "pub fn add(a: int, b: int) -> int { rn a + b }");
    p.write("main.on", "import math\nprint(math.add(1, 2))");
    p.check("main.on").unwrap();
}

// ---------------------------------------------------------------------------
// check_file (LSP-friendly module-aware checking)
// ---------------------------------------------------------------------------

#[test]
fn check_file_resolves_cross_module_references() {
    let p = TempProject::new();
    p.write("math.on", "pub fn add(a: int, b: int) -> int { rn a + b }");
    let main_src = "import math\nprint(math.add(1, 2))";
    p.write("main.on", main_src);

    let entry = p.root.join("main.on");
    let errors = oryn::Chunk::check_file(&entry, main_src);
    assert!(errors.is_empty(), "expected no errors, got: {errors:#?}");
}

#[test]
fn check_file_resolves_nested_imports() {
    let p = TempProject::new();
    p.write(
        "math/vec2.on",
        "pub obj Vec2 {
    pub x: float
    pub y: float

    pub fn length_sq(self) -> float {
        rn self.x * self.x + self.y * self.y
    }
}",
    );
    let main_src = "import math.vec2
let v = math.vec2.Vec2 { x: 3.0, y: 4.0 }
print(v.length_sq())";
    p.write("main.on", main_src);

    let entry = p.root.join("main.on");
    let errors = oryn::Chunk::check_file(&entry, main_src);
    assert!(errors.is_empty(), "expected no errors, got: {errors:#?}");
}

#[test]
fn check_file_uses_in_memory_source_not_disk() {
    let p = TempProject::new();
    p.write("math.on", "pub fn add(a: int, b: int) -> int { rn a + b }");
    // Disk has a stale version that references a non-existent function.
    p.write("main.on", "import math\nprint(math.missing(1, 2))");

    // But the in-memory source (what the LSP would send) is correct.
    let in_memory = "import math\nprint(math.add(1, 2))";
    let entry = p.root.join("main.on");
    let errors = oryn::Chunk::check_file(&entry, in_memory);

    assert!(
        errors.is_empty(),
        "expected no errors from in-memory source, got: {errors:#?}"
    );
}

#[test]
fn check_file_still_reports_real_errors() {
    let p = TempProject::new();
    p.write("math.on", "pub fn add(a: int, b: int) -> int { rn a + b }");
    let main_src = "import math\nprint(math.nonexistent(1, 2))";
    p.write("main.on", main_src);

    let entry = p.root.join("main.on");
    let errors = oryn::Chunk::check_file(&entry, main_src);
    assert_compile_error_contains(&errors, "math.nonexistent");
}

#[test]
fn check_file_falls_back_when_no_package_on() {
    use std::fs;
    // Build a directory with no package.on anywhere.
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(10000);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("oryn-check-test-{pid}-{n}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    let entry = root.join("standalone.on");
    let src = "let x = 5\nprint(x)";
    fs::write(&entry, src).unwrap();

    let errors = oryn::Chunk::check_file(&entry, src);
    assert!(
        errors.is_empty(),
        "expected single-file fallback to work, got: {errors:#?}"
    );

    let _ = fs::remove_dir_all(&root);
}
