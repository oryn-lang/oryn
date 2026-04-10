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

    let chunk = match oryn::Chunk::compile_file_sourced(file) {
        Ok(chunk) => chunk,
        Err(diagnostics) => {
            for diag in &diagnostics {
                let origin = diag.file.display().to_string();
                if let Err(e) = crate::errors::report_errors(&origin, &diag.source, &diag.errors) {
                    eprintln!("error: failed to print diagnostics: {e}");
                }
            }
            std::process::exit(1);
        }
    };

    let mut vm = oryn::VM::new();

    if let Err(e) = vm.run(&chunk) {
        let runtime_err = oryn::OrynError::from(e);

        if let Err(io_err) = crate::errors::report_errors(&filename, &source, &[runtime_err]) {
            eprintln!("error: failed to print diagnostics: {io_err}");
        }

        std::process::exit(1);
    }
}
