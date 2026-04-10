use std::io::Write;
use std::time::Instant;

use console::style;
use oryn_fmt::{FormatPathError, format_target};

use crate::ui;

pub fn run(target: &str) {
    let start = Instant::now();
    let spinner = ui::spinner("formatting…");

    match format_target(target) {
        Ok(changed) => {
            spinner.finish_and_clear();
            let elapsed = start.elapsed();

            println!();
            if changed.is_empty() {
                ui::success("already formatted", elapsed);
            } else {
                ui::header("formatted", changed.len(), "file");
                println!();

                for path in &changed {
                    println!(
                        "    {} {}",
                        style("✎").magenta().bold(),
                        style(path.display()).dim(),
                    );
                }

                println!();
                ui::success("done", elapsed);
            }
            println!();
        }

        Err(FormatPathError::Io { path, source }) => {
            spinner.finish_and_clear();
            println!();
            ui::error(&format!("{}: {source}", path.display()));
            println!();
            std::process::exit(1);
        }

        Err(FormatPathError::GlobPattern { pattern, source }) => {
            spinner.finish_and_clear();
            println!();
            ui::error(&format!("invalid glob `{pattern}`: {source}"));
            println!();
            std::process::exit(1);
        }

        Err(FormatPathError::Glob { pattern, source }) => {
            spinner.finish_and_clear();
            println!();
            ui::error(&format!("glob failed `{pattern}`: {source}"));
            println!();
            std::process::exit(1);
        }

        Err(FormatPathError::NoMatches { target }) => {
            spinner.finish_and_clear();
            println!();
            ui::error(&format!("no .on files matched `{target}`"));
            println!();
            std::process::exit(1);
        }

        Err(FormatPathError::Format {
            path,
            source,
            errors,
        }) => {
            spinner.finish_and_clear();
            let filename = path.display().to_string();

            println!();
            let _ = std::io::stdout().flush();

            if let Err(io_err) = crate::errors::report_errors(&filename, &source, &errors) {
                eprintln!("error: failed to print diagnostics: {io_err}");
            }

            println!();
            std::process::exit(1);
        }
    }
}
