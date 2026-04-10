use std::path::Path;

/// Compile-only command: runs the full lex/parse/compile pipeline on a
/// file (and its imports) and reports any errors via ariadne, but never
/// executes the resulting bytecode. Useful for editor "save and check"
/// hooks, CI lint steps, and confirming a project type-checks without
/// running the program.
///
/// Exits with status 0 if compilation succeeds, 1 if any errors were
/// reported.
pub fn run(file: &Path) {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to read {}: {e}", file.display());

            std::process::exit(1);
        }
    };

    let filename = file.display().to_string();

    if let Err(errors) = oryn::Chunk::compile_file(file) {
        if let Err(e) = crate::errors::report_errors(&filename, &source, &errors) {
            eprintln!("error: failed to print diagnostics: {e}");
        }
        std::process::exit(1);
    }

    println!("ok: {}", file.display());
}
