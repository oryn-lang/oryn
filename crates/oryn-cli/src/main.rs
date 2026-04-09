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
    Run { file: std::path::PathBuf },
    Disasm { file: std::path::PathBuf },
    Fmt { path: String },
    Lsp,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Run { file } => commands::run::run(&file),
        Command::Disasm { file } => commands::disasm::run(&file),
        Command::Fmt { path } => commands::fmt::run(&path),
        Command::Lsp => commands::lsp::run(),
    }
}
