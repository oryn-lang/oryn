use std::path::Path;

pub fn run(file: &Path) {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to read {}: {e}", file.display());
            std::process::exit(1);
        }
    };

    let filename = file.display().to_string();

    let chunk = match oryn::Chunk::compile(&source) {
        Ok(chunk) => chunk,
        Err(errors) => {
            if let Err(e) = crate::errors::report_errors(&filename, &source, &errors) {
                eprintln!("error: failed to print diagnostics: {e}");
            }
            std::process::exit(1);
        }
    };

    print!("{}", chunk.disassemble());
}
