mod commands;
mod errors;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "oryn", about = "✦ Oryn - A tiny language for game scripting.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compile and run an Oryn source file
    Run { file: std::path::PathBuf },
    /// Compile a file and report any errors without running it
    Check { file: std::path::PathBuf },
    /// Disassemble a compiled chunk and print its bytecode
    Disasm { file: std::path::PathBuf },
    /// Format Oryn source files in place
    Fmt { path: String },
    /// Start the LSP server (stdio transport)
    Lsp,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Run { file } => commands::run::run(&file),
        Command::Check { file } => commands::check::run(&file),
        Command::Disasm { file } => commands::disasm::run(&file),
        Command::Fmt { path } => commands::fmt::run(&path),
        Command::Lsp => commands::lsp::run(),
    }
}
