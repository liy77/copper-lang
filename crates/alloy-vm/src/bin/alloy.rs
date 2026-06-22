//! CLI do Alloy: interpretador tree-walking de Copper.
//!
//! * `alloy run <arquivo>` — roda instantâneo (estilo node/python): interpreta
//!   um `.crs` direto (sem etapa de compilação) OU executa um `.loy`.
//! * `alloy build <arquivo.crs>` — compila para um artefato portátil `.loy`
//!   (AST serializada) que roda em qualquer `alloy` de qualquer SO.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use alloy_vm::interp::Interpreter;
use clap::{Parser, Subcommand};
use copper_syntax::program::{parse_program, Program};

#[derive(Parser)]
#[command(name = "alloy", about = "Interpretador Alloy para Copper")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Interpreta um `.crs` (instantâneo) ou executa um `.loy`.
    Run { file: PathBuf },
    /// Compila um `.crs` para um artefato portátil `.loy`.
    Build {
        file: PathBuf,
        /// Caminho de saída (default: mesmo nome com extensão `.loy`).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Run { file } => run(&file),
        Cmd::Build { file, output } => build(&file, output.as_deref()),
    }
}

/// Carrega um `Program` de um arquivo: bytecode `.loy` ou fonte `.crs`.
fn load_program(file: &Path) -> Result<Program, String> {
    let bytes =
        std::fs::read(file).map_err(|e| format!("não consegui ler {}: {e}", file.display()))?;
    if alloy_vm::bytecode::is_bytecode(&bytes) {
        return alloy_vm::bytecode::load(&bytes);
    }
    let src = String::from_utf8(bytes).map_err(|_| "arquivo não é UTF-8 nem .loy".to_string())?;
    let prog = parse_program(&src);
    if !prog.errors.is_empty() {
        let msg = prog
            .errors
            .iter()
            .map(|e| format!("sintaxe @ {}..{}: {}", e.span.start, e.span.end, e.message))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(msg);
    }
    Ok(prog)
}

fn run(file: &Path) -> ExitCode {
    let prog = match load_program(file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("alloy: {e}");
            return ExitCode::FAILURE;
        }
    };
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

fn build(file: &Path, output: Option<&Path>) -> ExitCode {
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
    let out = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| file.with_extension("loy"));
    let bytes = alloy_vm::bytecode::compile(&prog.items);
    match std::fs::write(&out, bytes) {
        Ok(()) => {
            println!("compilado: {}", out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("alloy: não consegui gravar {}: {e}", out.display());
            ExitCode::FAILURE
        }
    }
}
