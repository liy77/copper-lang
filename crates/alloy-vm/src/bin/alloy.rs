//! CLI do Alloy: interpretador tree-walking de Copper.

use std::path::PathBuf;
use std::process::ExitCode;

use alloy_vm::interp::Interpreter;
use clap::{Parser, Subcommand};
use copper_syntax::program::parse_program;

#[derive(Parser)]
#[command(name = "alloy", about = "Interpretador Alloy para Copper")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Interpreta um arquivo .crs.
    Run { file: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run { file } => run(&file),
    }
}

fn run(file: &PathBuf) -> ExitCode {
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("alloy: não consegui ler {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    let prog = parse_program(&src);
    if !prog.errors.is_empty() {
        for err in &prog.errors {
            eprintln!(
                "alloy: erro de sintaxe @ {}..{}: {}",
                err.span.start, err.span.end, err.message
            );
        }
        return ExitCode::FAILURE;
    }
    match Interpreter::new().run_program(&prog) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!(
                "alloy: erro de runtime @ {}..{}: {}",
                e.span.start, e.span.end, e.message
            );
            ExitCode::FAILURE
        }
    }
}
