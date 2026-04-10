use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use console::style;

use crate::ui;

/// Runs every `test "..." { ... }` block in the given set of files.
///
/// File discovery:
/// - If one or more patterns are supplied, each is expanded via the
///   [`glob`] crate. Bare directory paths are rewritten to `dir/**/*.on`;
///   bare file paths are passed through verbatim.
/// - If no patterns are supplied, the command finds the project root
///   (nearest ancestor containing a `package.on`) and walks it for all
///   `.on` files. This mirrors `cargo test` / `go test ./...` — "run
///   everything relevant to this project".
///
/// For each matched file the command:
/// 1. Lexes + parses to check whether the file contains any test blocks.
///    Files with zero tests are silently skipped so the output stays
///    focused on files that actually have something to run.
/// 2. Compiles the file via [`oryn::Chunk::compile_file_sourced`]. Any
///    compile error is reported with the existing pretty ariadne output
///    and the file's tests are skipped.
/// 3. Invokes each discovered test in isolation via
///    [`oryn::VM::run_function`] — a fresh VM for every test so state
///    from earlier tests cannot leak into later ones. Top-level code
///    in the file is never executed; only the compiled test bodies run.
/// 4. Times each test and prints `✓ name` / `✗ name` with elapsed ms.
/// 5. Renders a failure diagnostic (ariadne, span-aware) after each
///    file's summary for any failed test.
///
/// Exit code: `0` if every test passes and every file compiles cleanly,
/// `1` otherwise.
pub fn run(patterns: &[String]) {
    let start = Instant::now();

    // ── collect files ───────────────────────────────────────────────
    let spinner = ui::spinner("collecting test files…");

    let files = match collect_candidate_files(patterns) {
        Ok(files) => files,
        Err(message) => {
            spinner.finish_and_clear();
            ui::error(&message);
            std::process::exit(1);
        }
    };

    // Pre-filter to only files that actually contain test blocks. This
    // keeps the output signal-to-noise high — no `0 tests` lines for
    // every source file in the project.
    let mut test_files: Vec<PathBuf> = Vec::new();
    for file in &files {
        if file_has_tests(file) {
            test_files.push(file.clone());
        }
    }
    test_files.sort();

    spinner.finish_and_clear();

    if test_files.is_empty() {
        println!();
        ui::error("no test blocks found in the matched files");
        println!();
        std::process::exit(1);
    }

    // ── header ──────────────────────────────────────────────────────
    println!();
    ui::header("testing", test_files.len(), "file");
    println!();

    // ── per-file test execution ─────────────────────────────────────
    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut failure_reports: Vec<(String, String, oryn::OrynError)> = Vec::new();
    let mut had_compile_error = false;

    for file in &test_files {
        println!("    {}", style(file.display()).dim());

        // Compile the file. Compile errors are reported immediately and
        // the file's tests are skipped — there are no tests to run if
        // the source doesn't even parse.
        let chunk = match oryn::Chunk::compile_file_sourced(file) {
            Ok(chunk) => chunk,
            Err(diagnostics) => {
                println!(
                    "      {} {}",
                    style("✗").red().bold(),
                    style("failed to compile").red(),
                );
                println!();
                let _ = std::io::stdout().flush();
                for diag in diagnostics {
                    let origin = diag.file.display().to_string();
                    if let Err(e) =
                        crate::errors::report_errors(&origin, &diag.source, &diag.errors)
                    {
                        eprintln!("error: failed to print diagnostics: {e}");
                    }
                }
                had_compile_error = true;
                println!();
                continue;
            }
        };

        // Re-read the file's source so assertion failures can slice the
        // asserted expression's snippet. `compile_file_sourced` does
        // own the source transiently but doesn't return it; reading
        // again is cheap and keeps the API surface small.
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                ui::error(&format!("{}: {e}", file.display()));
                had_compile_error = true;
                continue;
            }
        };
        let filename = file.display().to_string();

        for test in chunk.tests() {
            let mut vm = oryn::VM::new();
            let t0 = Instant::now();
            let result = vm.run_function(&chunk, test.function_idx);
            let elapsed = t0.elapsed();

            match result {
                Ok(()) => {
                    println!(
                        "      {} {}  {}",
                        style("✓").green().bold(),
                        style(&test.display_name),
                        style(format_elapsed(elapsed)).dim(),
                    );
                    total_passed += 1;
                }
                Err(e) => {
                    println!(
                        "      {} {}  {}",
                        style("✗").red().bold(),
                        style(&test.display_name).red(),
                        style(format_elapsed(elapsed)).dim(),
                    );
                    failure_reports.push((
                        filename.clone(),
                        source.clone(),
                        oryn::OrynError::from(e),
                    ));
                    total_failed += 1;
                }
            }
        }

        println!();
    }

    // ── failure diagnostics ─────────────────────────────────────────
    if !failure_reports.is_empty() {
        let _ = std::io::stdout().flush();
        for (filename, source, error) in &failure_reports {
            if let Err(e) =
                crate::errors::report_errors(filename, source, std::slice::from_ref(error))
            {
                eprintln!("error: failed to print diagnostics: {e}");
            }
        }
    }

    // ── summary ─────────────────────────────────────────────────────
    let elapsed = start.elapsed();
    if total_failed == 0 && !had_compile_error {
        ui::success(&format!("{total_passed} passed"), elapsed);
    } else {
        ui::failure_summary(total_passed, total_failed, elapsed);
    }
    println!();

    if total_failed > 0 || had_compile_error {
        std::process::exit(1);
    }
}

// ── helpers ─────────────────────────────────────────────────────────

/// Expand `patterns` into the concrete set of `.on` files to consider.
/// No patterns means "walk the project root"; explicit patterns are
/// expanded via glob + directory-walk.
fn collect_candidate_files(patterns: &[String]) -> Result<Vec<PathBuf>, String> {
    if patterns.is_empty() {
        return discover_from_project_root();
    }

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut files: Vec<PathBuf> = Vec::new();

    for pattern in patterns {
        let expanded = expand_pattern(pattern)?;
        for path in expanded {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if seen.insert(canonical) {
                files.push(path);
            }
        }
    }

    if files.is_empty() {
        return Err(format!(
            "no .on files matched the supplied pattern{}",
            if patterns.len() == 1 { "" } else { "s" }
        ));
    }

    Ok(files)
}

/// Walk `package.on` → project root → recurse for `.on` files.
fn discover_from_project_root() -> Result<Vec<PathBuf>, String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("failed to read current directory: {e}"))?;
    let root = oryn::find_project_root(&cwd).ok_or_else(|| {
        "no package.on found in current directory or any parent \
         — pass an explicit path or glob instead"
            .to_string()
    })?;

    let mut files: Vec<PathBuf> = Vec::new();
    collect_on_files(&root, &mut files).map_err(|e| format!("{}: {e}", root.display()))?;
    Ok(files)
}

/// Expand a single CLI pattern. Directories become `dir/**/*.on`, plain
/// files pass through, and anything else is treated as a glob. Missing
/// files produce a clear error rather than a silent empty result.
fn expand_pattern(pattern: &str) -> Result<Vec<PathBuf>, String> {
    let as_path = Path::new(pattern);

    if as_path.is_dir() {
        let dir_pattern = format!("{}/**/*.on", as_path.display());
        return expand_glob(&dir_pattern);
    }

    if as_path.is_file() {
        return Ok(vec![as_path.to_path_buf()]);
    }

    // Not a concrete file or directory — treat as a glob pattern.
    expand_glob(pattern)
}

fn expand_glob(pattern: &str) -> Result<Vec<PathBuf>, String> {
    let entries = glob::glob(pattern).map_err(|e| format!("invalid glob `{pattern}`: {e}"))?;

    let mut out: Vec<PathBuf> = Vec::new();
    for entry in entries {
        match entry {
            Ok(path) => {
                if path.is_file() && is_oryn_file(&path) {
                    out.push(path);
                } else if path.is_dir() {
                    collect_on_files(&path, &mut out)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                }
            }
            Err(e) => return Err(format!("glob error: {e}")),
        }
    }
    Ok(out)
}

fn is_oryn_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "on")
}

/// Recursive directory walk, collecting every `.on` file (excluding the
/// `package.on` marker).
fn collect_on_files(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let meta = std::fs::metadata(path)?;
    if meta.is_file() {
        if is_oryn_file(path) && path.file_name().and_then(|n| n.to_str()) != Some("package.on") {
            out.push(path.to_path_buf());
        }
        return Ok(());
    }
    if !meta.is_dir() {
        return Ok(());
    }

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_on_files(&entry_path, out)?;
        } else if file_type.is_file()
            && is_oryn_file(&entry_path)
            && entry_path.file_name().and_then(|n| n.to_str()) != Some("package.on")
        {
            out.push(entry_path);
        }
    }
    Ok(())
}

/// True iff `file` parses to at least one `Statement::Test`. A file
/// that fails to lex or parse is considered to have tests so the real
/// compile error is reported later rather than silently skipped.
fn file_has_tests(file: &Path) -> bool {
    let Ok(source) = std::fs::read_to_string(file) else {
        return false;
    };
    let (tokens, _) = oryn::lex(&source);
    let (stmts, parse_errors) = oryn::parse(tokens);
    if !parse_errors.is_empty() {
        // Keep the file in the list so the runner surfaces the real
        // parse error instead of quietly hiding it.
        return true;
    }
    stmts
        .iter()
        .any(|s| matches!(s.node, oryn::Statement::Test { .. }))
}

/// Compact human-readable duration for per-test timing.
fn format_elapsed(d: Duration) -> String {
    let micros = d.as_micros();
    if micros < 1000 {
        format!("{micros}µs")
    } else if micros < 1_000_000 {
        format!("{:.1}ms", micros as f64 / 1000.0)
    } else {
        format!("{:.2}s", d.as_secs_f64())
    }
}
