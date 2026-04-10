use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Compile-only command: runs the full lex/parse/compile pipeline on
/// each path (walking directories for `.on` files) and reports any
/// errors via ariadne without executing anything. Useful for CI, the
/// pre-commit hook, and editor "save and check" loops.
///
/// When a directory is walked, files that are reachable via `import`
/// from another file in the batch are skipped — they'll be
/// type-checked transitively when their importer is checked, and
/// compiling a module file as its own entry point misbehaves (module
/// constants only exist in module-compilation mode).
///
/// Empty `package.on` marker files are also skipped.
///
/// Exits with status 0 if every file compiles cleanly, 1 if any errors
/// were reported.
pub fn run(paths: &[PathBuf]) {
    let mut files: Vec<PathBuf> = Vec::new();
    for path in paths {
        if let Err(e) = collect_on_files(path, &mut files) {
            eprintln!("error: {}: {e}", path.display());
            std::process::exit(1);
        }
    }

    if files.is_empty() {
        eprintln!("error: no .on files found in the given paths");
        std::process::exit(1);
    }

    // Compute the set of files that are imported (directly or
    // indirectly) by some OTHER file in the batch. Those are modules;
    // the importer's `compile_file` will type-check them transitively.
    let imported = collect_imported_files(&files);

    let mut any_errors = false;
    for file in &files {
        if is_package_marker(file) {
            continue;
        }
        if let Ok(canonical) = file.canonicalize()
            && imported.contains(&canonical)
        {
            continue;
        }
        if !check_file(file) {
            any_errors = true;
        }
    }

    if any_errors {
        std::process::exit(1);
    }
}

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

/// Type-check a single file. Returns `true` on success, `false` if
/// any diagnostics were reported.
fn check_file(file: &Path) -> bool {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to read {}: {e}", file.display());
            return false;
        }
    };

    let filename = file.display().to_string();

    if let Err(errors) = oryn::Chunk::compile_file(file) {
        if let Err(e) = crate::errors::report_errors(&filename, &source, &errors) {
            eprintln!("error: failed to print diagnostics: {e}");
        }
        return false;
    }

    println!("ok: {}", file.display());
    true
}
