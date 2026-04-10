mod commands;
mod errors;
mod ui;

use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};

/// Top-level CLI. `oryn <FILE>` with no subcommand is shorthand for
/// `oryn run <FILE>` so the common case stays short.
#[derive(Parser)]
#[command(name = "oryn", about = "✦ Oryn - A tiny language for game scripting.")]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Oryn source file to run. Only used when no subcommand is given —
    /// `oryn path/to/foo.on` is equivalent to `oryn run path/to/foo.on`.
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Compile and run an Oryn source file
    Run { file: PathBuf },
    /// Compile one or more files (or directories) and report any errors
    /// without running them. Directories are walked recursively for
    /// `.on` files, so `oryn check examples/` type-checks the whole
    /// tree. Exits non-zero if any file fails to compile.
    Check {
        /// Files and/or directories containing `.on` sources.
        #[arg(value_name = "PATH", required = true)]
        paths: Vec<PathBuf>,
    },
    /// Disassemble a compiled chunk and print its bytecode
    Disasm { file: PathBuf },
    /// Format Oryn source files in place
    Fmt { path: String },
    /// Start the LSP server (stdio transport)
    Lsp,
}

fn main() {
    let cli = Cli::parse();

    match (cli.command, cli.file) {
        (Some(Command::Run { file }), _) => commands::run::run(&file),
        (Some(Command::Check { paths }), _) => commands::check::run(&paths),
        (Some(Command::Disasm { file }), _) => commands::disasm::run(&file),
        (Some(Command::Fmt { path }), _) => commands::fmt::run(&path),
        (Some(Command::Lsp), _) => commands::lsp::run(),
        (None, Some(file)) => commands::run::run(&file),
        (None, None) => {
            // No subcommand, no file — print help and exit with the
            // "missing argument" status, same as clap's default.
            let mut cmd = Cli::command();
            let _ = cmd.print_help();
            println!();
            std::process::exit(2);
        }
    }
}
