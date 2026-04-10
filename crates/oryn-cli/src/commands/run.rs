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

    let chunk = match oryn::Chunk::compile_file(file) {
        Ok(chunk) => chunk,
        Err(errors) => {
            if let Err(e) = crate::errors::report_errors(&filename, &source, &errors) {
                eprintln!("error: failed to print diagnostics: {e}");
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
