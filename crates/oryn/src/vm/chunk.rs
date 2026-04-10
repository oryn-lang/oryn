use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::Statement;
use crate::compiler::{
    self, CompiledFunction, CompilerOutput, Instruction, ModuleExports, ModuleTable, ObjDefInfo,
    TestInfo, TypeMap,
};
use crate::errors::{FileDiagnostics, OrynError};
use crate::lexer;
use crate::modules::{find_project_root, load_module, resolve_import};
use crate::parser;

/// Compiled bytecode ready to be run by a [`super::VM`].
///
/// ```
/// let chunk = oryn::Chunk::compile("let x = 5\nprint(x)").unwrap();
/// let mut vm = oryn::VM::new();
///
/// vm.run(&chunk).unwrap();
/// ```
#[derive(Debug)]
pub struct Chunk {
    pub(crate) instructions: Vec<Instruction>,
    pub(crate) spans: Vec<Range<usize>>,
    pub(crate) functions: Vec<CompiledFunction>,
    pub(crate) obj_defs: Vec<ObjDefInfo>,
    /// Test blocks defined in this chunk's entry file (never in imported
    /// modules). The `oryn test` runner reads this vec and invokes each
    /// entry by `function_idx`.
    pub(crate) tests: Vec<TestInfo>,
}

impl Chunk {
    /// Returns the test blocks discovered in this chunk's entry file.
    /// Consumers (the `oryn test` runner) get a read-only view; the
    /// internal fields stay `pub(crate)` so the chunk's bytecode layout
    /// is not part of the public API.
    pub fn tests(&self) -> &[TestInfo] {
        &self.tests
    }
}

impl Chunk {
    /// Compiles source code into a [`Chunk`].
    ///
    /// ```
    /// let chunk = oryn::Chunk::compile("let x = 1 + 2").unwrap();
    /// ```
    ///
    /// Returns lex/parse errors if the source is invalid:
    ///
    /// ```
    /// let err = oryn::Chunk::compile("let = @").unwrap_err();
    ///
    /// assert!(!err.is_empty());
    /// ```
    pub fn compile(source: &str) -> Result<Self, Vec<OrynError>> {
        let (tokens, lex_errors) = lexer::lex(source);
        let (statements, parse_errors) = parser::parse(tokens);

        let errors: Vec<_> = lex_errors.into_iter().chain(parse_errors).collect();
        if !errors.is_empty() {
            return Err(errors);
        }

        let output = compiler::compile(statements, ModuleTable::default(), 0, 0, vec![]);
        if !output.errors.is_empty() {
            return Err(output.errors);
        }

        Ok(Self {
            instructions: output.instructions,
            spans: output.spans,
            functions: output.functions,
            obj_defs: output.obj_defs,
            tests: output.tests,
        })
    }

    /// Compiles an entry file and all of its transitive imports into a single [`Chunk`].
    ///
    /// 1. Finds the project root by walking up from `path` looking for
    ///    `package.on`.
    /// 2. Lexes and parses the entry file.
    /// 3. Recursively compiles every imported module (depth-first,
    ///    cycle-safe via an in-progress set keyed on canonical paths).
    /// 4. Merges all module outputs into one [`CompilerOutput`]. Function
    ///    and object indices are absolute from the start because each
    ///    module compiles with offsets pointing past everything already
    ///    merged — no post-hoc instruction remapping is needed.
    /// 5. Compiles the entry file with the populated module table so
    ///    cross-module function calls, object literals, and constants
    ///    resolve to the right merged indices.
    ///
    /// Each imported module is compiled with its **own** module table
    /// containing only its direct imports, matching Rust/Zig's
    /// non-transitive module visibility.
    pub fn compile_file(path: &Path) -> Result<Self, Vec<OrynError>> {
        Self::compile_file_sourced(path).map_err(|diagnostics| {
            diagnostics
                .into_iter()
                .flat_map(|d| d.errors)
                .collect::<Vec<_>>()
        })
    }

    /// Like [`Chunk::compile_file`], but returns errors grouped by the
    /// file they originated from. Each [`FileDiagnostics`] carries the
    /// file path, its full source text, and the errors whose spans
    /// index into that source.
    ///
    /// Prefer this over [`Chunk::compile_file`] when rendering
    /// diagnostics: errors from imported modules would otherwise be
    /// rendered against the importer's file and land on the wrong
    /// lines.
    pub fn compile_file_sourced(path: &Path) -> Result<Self, Vec<FileDiagnostics>> {
        let parent = path.parent().unwrap_or(path);
        let project_root = find_project_root(parent).ok_or_else(|| {
            vec![FileDiagnostics {
                file: path.to_path_buf(),
                source: String::new(),
                errors: vec![OrynError::Module {
                    path: path.to_string_lossy().into_owned(),
                    message: "no package.on found in parent directories".to_string(),
                }],
            }]
        })?;

        // Read the entry file
        let source = std::fs::read_to_string(path).map_err(|e| {
            vec![FileDiagnostics {
                file: path.to_path_buf(),
                source: String::new(),
                errors: vec![OrynError::Module {
                    path: path.to_string_lossy().into_owned(),
                    message: e.to_string(),
                }],
            }]
        })?;

        // Lex & parse
        let (tokens, lex_errors) = lexer::lex(&source);
        let (statements, parse_errors) = parser::parse(tokens);
        let errors: Vec<OrynError> = lex_errors.into_iter().chain(parse_errors).collect();
        if !errors.is_empty() {
            return Err(vec![FileDiagnostics {
                file: path.to_path_buf(),
                source,
                errors,
            }]);
        }

        // Collect imports from the AST
        let imports: Vec<Vec<String>> = statements
            .iter()
            .filter_map(|s| match &s.node {
                Statement::Import { path } => Some(path.clone()),
                _ => None,
            })
            .collect();

        // Cycle-detection set (canonical paths currently being compiled)
        let mut compiling: HashSet<PathBuf> = HashSet::new();
        compiling.insert(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));

        // Accumulated output from all compiled modules.
        let mut merged = CompilerOutput::default();
        // Cache of already-compiled modules keyed by canonical file path.
        let mut compiled_modules: HashMap<PathBuf, ModuleExports> = HashMap::new();
        // Module table that the entry file will be compiled with.
        let mut module_table = ModuleTable::default();

        // Recursively compile every imported module. Errors from failing
        // modules accumulate as file-scoped [`FileDiagnostics`] batches so
        // callers know which file's source each span indexes into.
        let mut diagnostics: Vec<FileDiagnostics> = Vec::new();
        for import in &imports {
            match compile_module(
                &project_root,
                import,
                &mut compiling,
                &mut merged,
                &mut compiled_modules,
            ) {
                Ok(exports) => {
                    // Dot-joined full path: "math" for flat, "math.nested.lib"
                    // for nested. The compiler looks up modules by this key.
                    let module_name = import.join(".");
                    module_table.modules.insert(module_name, exports);
                }
                Err(diags) => diagnostics.extend(diags),
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }

        // Compile the entry file with offsets pointing past all merged modules.
        // The compiler will emit absolute indices directly, so no post-hoc
        // remapping is needed — we can just extend the merged output.
        let fn_offset = merged.functions.len();
        let obj_offset = merged.obj_defs.len();

        // Entry file: current_module_path is empty.
        let entry_output =
            compiler::compile(statements, module_table, fn_offset, obj_offset, vec![]);
        if !entry_output.errors.is_empty() {
            return Err(vec![FileDiagnostics {
                file: path.to_path_buf(),
                source,
                errors: entry_output.errors,
            }]);
        }

        merged.instructions = entry_output.instructions;
        merged.spans = entry_output.spans;
        merged.functions.extend(entry_output.functions);
        merged.obj_defs.extend(entry_output.obj_defs);
        // Tests come exclusively from the entry file. Imported modules
        // may also define tests, but their `TestInfo` entries stay
        // isolated so `oryn test <file>` only runs tests in the
        // explicitly-matched file.
        let entry_tests = entry_output.tests;

        Ok(Chunk {
            instructions: merged.instructions,
            spans: merged.spans,
            functions: merged.functions,
            obj_defs: merged.obj_defs,
            tests: entry_tests,
        })
    }

    /// Returns all lex, parse, and compile errors. An empty
    /// vec means the source is valid.
    ///
    /// ```
    /// assert!(oryn::Chunk::check("let x = 5").is_empty());
    /// assert!(!oryn::Chunk::check("let = @").is_empty());
    /// ```
    pub fn check(source: &str) -> Vec<OrynError> {
        let (errors, _types) = Self::check_with_types(source);
        errors
    }

    /// Like [`Chunk::check`], but also returns a [`TypeMap`] with every
    /// declaration's inferred type. Used by the LSP to power hover and
    /// inlay hints without rebuilding its own type inference.
    pub fn check_with_types(source: &str) -> (Vec<OrynError>, TypeMap) {
        let (tokens, lex_errors) = lexer::lex(source);
        let (statements, parse_errors) = parser::parse(tokens);

        let mut errors: Vec<OrynError> = lex_errors.into_iter().chain(parse_errors).collect();

        // Run the compiler even if there are lex/parse errors so that
        // compile-time checks (e.g. val reassignment) are reported too.
        let output = compiler::compile(statements, ModuleTable::default(), 0, 0, vec![]);
        errors.extend(output.errors);

        (errors, output.type_map)
    }

    /// Like [`Chunk::check`], but module-aware. Used by the LSP and other
    /// editor tooling: they hand in the in-memory source for the entry
    /// file (so unsaved changes are honored) plus the file path on disk
    /// (so we can locate `package.on` and resolve imports against it).
    ///
    /// If the file is part of a project (its directory or some ancestor
    /// contains `package.on`), imports are resolved by reading sibling
    /// files from disk. If no project root is found, we fall back to
    /// pure single-file checking, which means cross-module references
    /// in the source will report as undefined.
    pub fn check_file(path: &Path, source: &str) -> Vec<OrynError> {
        let (errors, _types) = Self::check_file_with_types(path, source);
        errors
    }

    /// Like [`Chunk::check_file`], but also returns a [`TypeMap`] for
    /// the entry file. Types from imported modules are not included.
    pub fn check_file_with_types(path: &Path, source: &str) -> (Vec<OrynError>, TypeMap) {
        let mut errors: Vec<OrynError> = Vec::new();

        let (tokens, lex_errors) = lexer::lex(source);
        let (statements, parse_errors) = parser::parse(tokens);
        errors.extend(lex_errors);
        errors.extend(parse_errors);

        // Try to find a project root. If there isn't one, we just compile
        // the file in isolation — same behavior as `check`.
        let parent = path.parent().unwrap_or(path);
        let project_root = find_project_root(parent);

        let module_table = if let Some(root) = project_root {
            // Walk imports and compile each one. Errors from module
            // compilation get folded into the main error list rather
            // than aborting, so the user still sees diagnostics on the
            // main file even if a sibling module is broken.
            let imports: Vec<Vec<String>> = statements
                .iter()
                .filter_map(|s| match &s.node {
                    Statement::Import { path } => Some(path.clone()),
                    _ => None,
                })
                .collect();

            let mut compiling: HashSet<PathBuf> = HashSet::new();
            compiling.insert(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
            let mut merged = CompilerOutput::default();
            let mut compiled_modules: HashMap<PathBuf, ModuleExports> = HashMap::new();
            let mut module_table = ModuleTable::default();

            for import in &imports {
                match compile_module(
                    &root,
                    import,
                    &mut compiling,
                    &mut merged,
                    &mut compiled_modules,
                ) {
                    Ok(exports) => {
                        module_table.modules.insert(import.join("."), exports);
                    }
                    Err(diags) => {
                        // `check_file` returns a flat error list, so flatten
                        // per-file diagnostics back to individual errors.
                        // The LSP renders against the entry file's buffer,
                        // which means module-origin spans may still land on
                        // the wrong lines — that's a separate, known
                        // limitation of this flat API.
                        for diag in diags {
                            errors.extend(diag.errors);
                        }
                    }
                }
            }

            module_table
        } else {
            ModuleTable::default()
        };

        // Compile the in-memory source with the populated (or empty)
        // module table. Offsets don't matter for checking — we only
        // care about the errors and type map collected on the way.
        let output = compiler::compile(statements, module_table, 0, 0, vec![]);
        errors.extend(output.errors);

        (errors, output.type_map)
    }

    /// Returns a human-readable disassembly of the compiled bytecode.
    ///
    /// ```
    /// let chunk = oryn::Chunk::compile("let x = 5\nprint(x)").unwrap();
    /// let output = chunk.disassemble();
    ///
    /// assert!(output.contains("SetLocal"));
    /// assert!(output.contains("CallBuiltin print"));
    /// ```
    pub fn disassemble(&self) -> String {
        use std::fmt::Write;

        let mut out = String::new();

        writeln!(out, "== <main> ==").unwrap();
        disassemble_instructions(&mut out, &self.instructions);

        for func in &self.functions {
            let params = func.params.join(", ");
            writeln!(out, "\n== {}({}) ==", func.name, params).unwrap();
            disassemble_instructions(&mut out, &func.instructions);
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Module compilation (recursive helper)
// ---------------------------------------------------------------------------

/// Recursively compile a single imported module and append its output to
/// the accumulated `merged` output. Returns the `ModuleExports` so the
/// caller can register the module under its dotted-path key.
///
/// **Cycle detection**: `compiling` tracks canonical file paths currently
/// being compiled in the current recursion stack. Re-entering a module
/// returns an `OrynError::Module` with "circular import detected".
///
/// **Caching**: `compiled_modules` memoizes already-finished modules so
/// the same import from multiple parents doesn't recompile.
///
/// **Definitions-only**: imported modules can only contain `let`, `val`,
/// `fn`, `obj`, and `import` statements. Top-level expressions or
/// control flow produce a compile error.
///
/// **Offset-aware compilation**: each module compiles with `fn_offset`
/// and `obj_offset` set to the current `merged` lengths, so the
/// compiler emits absolute indices that line up with the merged output
/// after `extend`.
fn compile_module(
    project_root: &Path,
    import_path: &[String],
    compiling: &mut HashSet<PathBuf>,
    merged: &mut CompilerOutput,
    compiled_modules: &mut HashMap<PathBuf, ModuleExports>,
) -> Result<ModuleExports, Vec<FileDiagnostics>> {
    let file_path = resolve_import(project_root, import_path);
    let canonical = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.clone());

    // ---- Already compiled — return cached exports ----
    if let Some(exports) = compiled_modules.get(&canonical) {
        return Ok(exports.clone());
    }

    // ---- Cycle detection ----
    if compiling.contains(&canonical) {
        return Err(vec![FileDiagnostics {
            file: file_path.clone(),
            source: String::new(),
            errors: vec![OrynError::Module {
                path: file_path.to_string_lossy().into_owned(),
                message: "circular import detected".to_string(),
            }],
        }]);
    }
    compiling.insert(canonical.clone());

    // ---- Load source ----
    let source = match load_module(project_root, import_path) {
        Ok(s) => s,
        Err(e) => {
            compiling.remove(&canonical);
            return Err(vec![FileDiagnostics {
                file: file_path.clone(),
                source: String::new(),
                errors: vec![e],
            }]);
        }
    };

    // ---- Lex & parse ----
    let (tokens, lex_errors) = lexer::lex(&source);
    let (statements, parse_errors) = parser::parse(tokens);
    let errors: Vec<OrynError> = lex_errors.into_iter().chain(parse_errors).collect();
    if !errors.is_empty() {
        compiling.remove(&canonical);
        return Err(vec![FileDiagnostics {
            file: file_path.clone(),
            source,
            errors,
        }]);
    }

    // ---- Validate definitions-only ----
    // `test` blocks are allowed alongside regular definitions so that a
    // module can carry its own tests; they compile into the merged
    // function table but their metadata is NOT propagated into the
    // entry chunk's `tests` vec, so `oryn test importer.on` never
    // silently runs tests defined in modules it imports.
    let mut errors: Vec<OrynError> = Vec::new();
    for stmt in &statements {
        match &stmt.node {
            Statement::Let { .. }
            | Statement::Val { .. }
            | Statement::Function { .. }
            | Statement::ObjDef { .. }
            | Statement::Import { .. }
            | Statement::Test { .. } => {}
            _ => {
                errors.push(OrynError::Module {
                    path: file_path.to_string_lossy().into_owned(),
                    message:
                        "only definitions (let, val, fn, obj, import, test) are allowed in modules"
                            .to_string(),
                });
            }
        }
    }
    if !errors.is_empty() {
        compiling.remove(&canonical);
        return Err(vec![FileDiagnostics {
            file: file_path.clone(),
            source,
            errors,
        }]);
    }

    // ---- Collect this module's own imports ----
    let sub_imports: Vec<Vec<String>> = statements
        .iter()
        .filter_map(|s| match &s.node {
            Statement::Import { path } => Some(path.clone()),
            _ => None,
        })
        .collect();

    // ---- Build this module's ModuleTable (only its direct imports) ----
    let mut module_table = ModuleTable::default();
    for sub_import in &sub_imports {
        match compile_module(
            project_root,
            sub_import,
            compiling,
            merged,
            compiled_modules,
        ) {
            Ok(sub_exports) => {
                let sub_name = sub_import.join(".");
                module_table.modules.insert(sub_name, sub_exports);
            }
            Err(diagnostics) => {
                compiling.remove(&canonical);
                return Err(diagnostics);
            }
        }
    }

    // Compile the module with its own ModuleTable and the offsets pointing
    // past all currently-merged content. The compiler emits absolute
    // indices directly, so no remapping is needed — we can just extend.
    let fn_offset = merged.functions.len();
    let obj_offset = merged.obj_defs.len();

    // Module compilation: current_module_path is the full import path.
    let module_output = compiler::compile(
        statements,
        module_table,
        fn_offset,
        obj_offset,
        import_path.to_vec(),
    );
    if !module_output.errors.is_empty() {
        compiling.remove(&canonical);
        return Err(vec![FileDiagnostics {
            file: file_path.clone(),
            source,
            errors: module_output.errors,
        }]);
    }

    // Build exports (only pub items) before consuming the output.
    let exports = module_output.build_module_exports(fn_offset, obj_offset);

    // Append functions and obj_defs as-is — their indices are already absolute.
    merged.functions.extend(module_output.functions);
    merged.obj_defs.extend(module_output.obj_defs);

    // Note: module top-level instructions are intentionally NOT merged.
    // Modules are definitions-only; their functions and obj_defs are the
    // only outputs that matter for the consuming file.

    // ---- Done ----
    compiling.remove(&canonical);
    compiled_modules.insert(canonical, exports.clone());

    Ok(exports)
}

// ---------------------------------------------------------------------------
// Disassembly
// ---------------------------------------------------------------------------

fn disassemble_instructions(out: &mut String, instructions: &[Instruction]) {
    use std::fmt::Write;

    for (i, instr) in instructions.iter().enumerate() {
        let formatted = match instr {
            Instruction::PushBool(b) => format!("PushBool {b}"),
            Instruction::PushFloat(n) => format!("PushFloat {n}"),
            Instruction::PushInt(n) => format!("PushInt {n}"),
            Instruction::PushString(s) => format!("PushString {s}"),
            Instruction::ToString => "ToString".to_string(),
            Instruction::Concat(n) => format!("Concat {n}"),
            Instruction::MakeRange(inclusive) => format!("MakeRange inclusive={inclusive}"),
            Instruction::GetLocal(slot) => format!("GetLocal {slot}"),
            Instruction::SetLocal(slot) => format!("SetLocal {slot}"),
            Instruction::NewObject(type_idx, num_fields) => {
                format!("NewObject {type_idx} {num_fields}")
            }
            Instruction::GetField(field_idx) => format!("GetField {field_idx}"),
            Instruction::SetField(field_idx) => format!("SetField {field_idx}"),
            Instruction::Return => "Return".to_string(),
            Instruction::Equal => "Equal".to_string(),
            Instruction::NotEqual => "NotEqual".to_string(),
            Instruction::LessThan => "LessThan".to_string(),
            Instruction::GreaterThan => "GreaterThan".to_string(),
            Instruction::LessThanEquals => "LessThanEquals".to_string(),
            Instruction::GreaterThanEquals => "GreaterThanEquals".to_string(),
            Instruction::Not => "Not".to_string(),
            Instruction::Negate => "Negate".to_string(),
            Instruction::Add => "Add".to_string(),
            Instruction::Sub => "Sub".to_string(),
            Instruction::Mul => "Mul".to_string(),
            Instruction::Div => "Div".to_string(),
            Instruction::Call(idx, arity) => {
                let s = if *arity == 1 { "arg" } else { "args" };
                format!("Call fn#{idx} ({arity} {s})")
            }
            Instruction::CallMethod(name, arity) => {
                let s = if *arity == 1 { "arg" } else { "args" };
                format!("CallMethod \"{name}\" ({arity} {s} + self)")
            }
            Instruction::CallBuiltin(builtin, arity) => {
                let s = if *arity == 1 { "arg" } else { "args" };
                format!("CallBuiltin {} ({arity} {s})", builtin.name())
            }
            Instruction::Pop => "Pop".to_string(),
            Instruction::JumpIfFalse(target) => format!("JumpIfFalse -> {target:04}"),
            Instruction::Jump(target) => format!("Jump -> {target:04}"),
            Instruction::RangeHasNext => "RangeHasNext".to_string(),
            Instruction::RangeNext => "RangeNext".to_string(),
            Instruction::PushNil => "PushNil".to_string(),
            Instruction::JumpIfNil(target) => format!("JumpIfNil -> {target:04}"),
            Instruction::JumpIfError(target) => format!("JumpIfError -> {target:04}"),
            Instruction::UnwrapErrorOrTrap => "UnwrapErrorOrTrap".to_string(),
            Instruction::MakeError => "MakeError".to_string(),
            Instruction::Assert => "Assert".to_string(),
        };

        writeln!(out, "{i:04}  {formatted}").unwrap();
    }
}
