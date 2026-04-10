use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use console::style;
use glob::glob;
use oryn::OrynError;
use oryn_fmt::format_source;

use crate::ui;

pub fn run(target: &str) -> Result<(), FmtCommandError> {
    let start = Instant::now();
    let spinner = ui::spinner("formatting…");

    let paths = resolve_targets(target)?;
    if paths.is_empty() {
        return Err(FmtCommandError::NoMatches {
            target: target.to_string(),
        });
    }

    let mut changed = Vec::new();
    for path in paths {
        if format_file(&path)? {
            changed.push(path);
        }
    }

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

    Ok(())
}

pub fn report(error: &FmtCommandError) {
    println!();

    match error {
        FmtCommandError::Io { path, source } => {
            ui::error(&format!("{}: {source}", path.display()));
            println!();
        }
        FmtCommandError::GlobPattern { pattern, source } => {
            ui::error(&format!("invalid glob `{pattern}`: {source}"));
            println!();
        }
        FmtCommandError::Glob { pattern, source } => {
            ui::error(&format!("glob failed `{pattern}`: {source}"));
            println!();
        }
        FmtCommandError::NoMatches { target } => {
            ui::error(&format!("no .on files matched `{target}`"));
            println!();
        }
        FmtCommandError::Format {
            path,
            source,
            errors,
        } => {
            let filename = path.display().to_string();
            let _ = std::io::stdout().flush();

            if let Err(io_err) = crate::errors::report_errors(&filename, source, errors) {
                eprintln!("error: failed to print diagnostics: {io_err}");
            }

            println!();
        }
    }
}

#[derive(Debug)]
pub enum FmtCommandError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    GlobPattern {
        pattern: String,
        source: glob::PatternError,
    },
    Glob {
        pattern: String,
        source: glob::GlobError,
    },
    NoMatches {
        target: String,
    },
    Format {
        path: PathBuf,
        source: String,
        errors: Vec<OrynError>,
    },
}

fn resolve_targets(target: &str) -> Result<Vec<PathBuf>, FmtCommandError> {
    let target_path = Path::new(target);
    if target_path.is_dir() {
        return collect_glob_paths(&format!("{}/**/*.on", target_path.display()));
    }

    if target_path.is_file() {
        return Ok(vec![target_path.to_path_buf()]);
    }

    collect_glob_paths(target)
}

fn collect_glob_paths(pattern: &str) -> Result<Vec<PathBuf>, FmtCommandError> {
    let mut paths = Vec::new();
    let entries = glob(pattern).map_err(|source| FmtCommandError::GlobPattern {
        pattern: pattern.to_string(),
        source,
    })?;

    for entry in entries {
        match entry {
            Ok(path) if path.is_file() && is_oryn_file(&path) => paths.push(path),
            Ok(path) if path.is_dir() => {
                paths.extend(collect_glob_paths(&format!("{}/**/*.on", path.display()))?);
            }
            Ok(_) => {}
            Err(source) => {
                return Err(FmtCommandError::Glob {
                    pattern: pattern.to_string(),
                    source,
                });
            }
        }
    }

    Ok(paths)
}

fn format_file(path: &Path) -> Result<bool, FmtCommandError> {
    let source = fs::read_to_string(path).map_err(|source| FmtCommandError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let formatted = format_source(&source).map_err(|errors| FmtCommandError::Format {
        path: path.to_path_buf(),
        source: source.clone(),
        errors,
    })?;

    if formatted == source {
        return Ok(false);
    }

    fs::write(path, &formatted).map_err(|source| FmtCommandError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(true)
}

fn is_oryn_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "on")
}
