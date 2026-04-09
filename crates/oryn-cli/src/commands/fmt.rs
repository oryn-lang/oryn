use oryn_fmt::{FormatPathError, format_target};

pub fn run(target: &str) {
    match format_target(target) {
        Ok(_) => {}
        Err(FormatPathError::Io { path, source }) => {
            eprintln!("error: failed to access {}: {source}", path.display());

            std::process::exit(1);
        }
        Err(FormatPathError::GlobPattern { pattern, source }) => {
            eprintln!("error: invalid glob pattern `{pattern}`: {source}");

            std::process::exit(1);
        }
        Err(FormatPathError::Glob { pattern, source }) => {
            eprintln!("error: failed while expanding `{pattern}`: {source}");

            std::process::exit(1);
        }
        Err(FormatPathError::Format {
            path,
            source,
            errors,
        }) => {
            let filename = path.display().to_string();

            if let Err(io_err) = crate::errors::report_errors(&filename, &source, &errors) {
                eprintln!("error: failed to print diagnostics: {io_err}");
            }

            std::process::exit(1);
        }
    }
}
