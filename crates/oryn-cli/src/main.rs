mod commands;
mod errors;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "oryn", about = "✦ Oryn - A tiny language for game scripting.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Run { file: PathBuf },
    Disasm { file: PathBuf },
    Lsp,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Run { file } => commands::run::run(&file),
        Command::Disasm { file } => commands::disasm::run(&file),
        Command::Lsp => commands::lsp::run(),
    }
}
