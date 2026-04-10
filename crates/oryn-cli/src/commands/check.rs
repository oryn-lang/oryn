use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use console::style;

use crate::ui;

/// Compile-only command: runs the full lex/parse/compile pipeline on
/// each path (walking directories for `.on` files) and reports any
/// errors via ariadne without executing anything. Useful for CI, the
/// pre-commit hook, and editor "save and check" loops.
///
/// When a directory is walked, files that are reachable via `import`
/// from another file in the batch are checked transitively through
/// their importer rather than as standalone entry points. They still
/// appear in the output with a `·` glyph so you can see everything
/// that was covered.
///
/// Empty `package.on` marker files are skipped.
///
/// Exits with status 0 if every file compiles cleanly, 1 if any errors
/// were reported.
pub fn run(paths: &[PathBuf]) {
    let start = Instant::now();

    // ── collect files ───────────────────────────────────────────────
    let spinner = ui::spinner("collecting files…");

    let mut files: Vec<PathBuf> = Vec::new();
    for path in paths {
        if let Err(e) = collect_on_files(path, &mut files) {
            spinner.finish_and_clear();
            ui::error(&format!("{}: {e}", path.display()));
            std::process::exit(1);
        }
    }

    if files.is_empty() {
        spinner.finish_and_clear();
        ui::error("no .on files found in the given paths");
        std::process::exit(1);
    }

    // Compute the set of files that are imported (directly or
    // indirectly) by some OTHER file in the batch. Those are modules;
    // the importer's `compile_file` will type-check them transitively.
    let imported = collect_imported_files(&files);

    // Split into entry points and imported modules, sorted for stable
    // output. Package markers are excluded entirely.
    let mut entry_points: Vec<&PathBuf> = Vec::new();
    let mut module_files: Vec<&PathBuf> = Vec::new();

    for file in &files {
        if is_package_marker(file) {
            continue;
        }
        if let Ok(canonical) = file.canonicalize()
            && imported.contains(&canonical)
        {
            module_files.push(file);
        } else {
            entry_points.push(file);
        }
    }

    entry_points.sort();
    module_files.sort();

    let total_count = entry_points.len() + module_files.len();
    spinner.finish_and_clear();

    // ── header ──────────────────────────────────────────────────────
    println!();
    ui::header("checking", total_count, "file");
    println!();

    // ── per-file check ──────────────────────────────────────────────
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut error_reports: Vec<(String, String, Vec<oryn::OrynError>)> = Vec::new();

    for file in &entry_points {
        let sp = ui::file_spinner(&file.display().to_string());

        match oryn::Chunk::compile_file(file) {
            Ok(_) => {
                sp.finish_and_clear();
                println!(
                    "    {} {}",
                    style("✓").green().bold(),
                    style(file.display()).dim(),
                );
                passed += 1;
            }
            Err(errors) => {
                sp.finish_and_clear();
                println!("    {} {}", style("✗").red().bold(), file.display());
                let source = std::fs::read_to_string(file).unwrap_or_default();
                error_reports.push((file.display().to_string(), source, errors));
                failed += 1;
            }
        }
    }

    // Imported modules — checked transitively via their importer.
    for file in &module_files {
        println!(
            "    {} {} {}",
            style("·").cyan(),
            style(file.display()).dim(),
            style("(via import)").dim(),
        );
    }

    // ── error reports ───────────────────────────────────────────────
    if !error_reports.is_empty() {
        println!();
        // Flush stdout so ariadne's stderr output doesn't interleave.
        let _ = std::io::stdout().flush();
        for (filename, source, errors) in &error_reports {
            if let Err(e) = crate::errors::report_errors(filename, source, errors) {
                eprintln!("error: failed to print diagnostics: {e}");
            }
        }
    }

    // ── summary ─────────────────────────────────────────────────────
    let elapsed = start.elapsed();
    println!();
    if failed == 0 {
        ui::success("all clear", elapsed);
    } else {
        ui::failure_summary(passed, failed, elapsed);
    }
    println!();

    if failed > 0 {
        std::process::exit(1);
    }
}

// ── helpers ─────────────────────────────────────────────────────────

/// True for a file named `package.on` (the empty project-root marker).
/// These carry no source to compile and are never meaningful as an
/// entry point.
fn is_package_marker(file: &Path) -> bool {
    file.file_name().and_then(|n| n.to_str()) == Some("package.on")
}

/// Parse each file's `import` statements and resolve them against the
/// project root to a canonicalized filesystem path. Returns the set of
/// files that some other file in `files` pulls in as a module.
fn collect_imported_files(files: &[PathBuf]) -> HashSet<PathBuf> {
    let mut imported: HashSet<PathBuf> = HashSet::new();

    for file in files {
        let Ok(source) = std::fs::read_to_string(file) else {
            continue;
        };
        let (tokens, _) = oryn::lex(&source);
        let (stmts, _) = oryn::parse(tokens);

        // Project root = nearest ancestor directory containing a
        // package.on. No root → no imports to resolve.
        let Some(parent) = file.parent() else {
            continue;
        };
        let Some(root) = oryn::find_project_root(parent) else {
            continue;
        };

        for stmt in &stmts {
            if let oryn::Statement::Import { path } = &stmt.node {
                let resolved = oryn::resolve_import(&root, path);
                if let Ok(canonical) = resolved.canonicalize() {
                    imported.insert(canonical);
                }
            }
        }
    }

    imported
}

/// Walk `path`, appending every `.on` file encountered to `out`. If
/// `path` is a file, it's added directly (regardless of extension —
/// the caller explicitly asked for it). Directories are traversed
/// recursively, skipping anything that isn't a `.on` file.
fn collect_on_files(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let meta = std::fs::metadata(path)?;
    if meta.is_file() {
        out.push(path.to_path_buf());
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
        } else if file_type.is_file() && entry_path.extension().is_some_and(|ext| ext == "on") {
            out.push(entry_path);
        }
    }
    Ok(())
}
