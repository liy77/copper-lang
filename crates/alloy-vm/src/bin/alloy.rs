//! CLI do Alloy: interpretador tree-walking de Copper.
//!
//! * `alloy run <arquivo>` — roda instantâneo (estilo node/python): interpreta
//!   um `.crs` direto (sem etapa de compilação), funde imports de `.crs` locais,
//!   ou executa um `.loy`. Se o programa importar Rust (`.rs`), delega ao
//!   `cforge` (transpila + compila nativo).
//! * `alloy build <arquivo.crs>` — compila para um artefato portátil `.loy`
//!   (AST serializada, com os `.crs` locais já fundidos).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use alloy_vm::interp::Interpreter;
use alloy_vm::loader::{self, LoadOutcome};
use clap::{Parser, Subcommand};

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

fn run(file: &Path) -> ExitCode {
    match loader::load_runnable(file) {
        Ok(LoadOutcome::Program(prog)) => match Interpreter::new().run_program(&prog) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!(
                    "alloy: erro de runtime @ {}..{}: {}",
                    e.span.start, e.span.end, e.message
                );
                ExitCode::FAILURE
            }
        },
        // Importou Rust: o interpretador não roda `.rs` → delega ao cforge.
        Ok(LoadOutcome::NeedsCforge { module, .. }) => {
            eprintln!("alloy: `{module}` é Rust (.rs); delegando para o cforge…");
            delegate_to_cforge(file)
        }
        Err(e) => {
            eprintln!("alloy: {e}");
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
    match loader::resolve_source(&src, file) {
        Ok(LoadOutcome::Program(prog)) => {
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
        Ok(LoadOutcome::NeedsCforge { module, .. }) => {
            eprintln!(
                "alloy build: `{module}` é Rust (.rs) — o `.loy` não embute Rust.\n\
                 Compile nativo com: cforge build {}",
                file.display()
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("alloy: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Localiza o binário `cforge` e roda `cforge run <file>`, propagando o código
/// de saída. Procura ao lado do próprio `alloy`, depois no PATH.
fn delegate_to_cforge(file: &Path) -> ExitCode {
    let candidates = cforge_candidates();
    for cf in &candidates {
        match Command::new(cf).arg("run").arg(file).status() {
            Ok(status) => {
                return ExitCode::from(status.code().unwrap_or(1) as u8);
            }
            Err(_) => continue,
        }
    }
    eprintln!(
        "alloy: não encontrei o `cforge` para compilar o Rust. \
         Instale o cforge e rode: cforge run {}",
        file.display()
    );
    ExitCode::FAILURE
}

fn cforge_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let name = if cfg!(windows) {
                "cforge.exe"
            } else {
                "cforge"
            };
            v.push(dir.join(name));
        }
    }
    // fallback: resolve via PATH.
    v.push(PathBuf::from("cforge"));
    v
}
