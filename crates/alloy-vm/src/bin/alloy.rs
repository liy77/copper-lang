//! Alloy CLI: tree-walking interpreter for Copper.
//!
//! * `alloy run <file>` — runs instantly (node/python style): interprets a
//!   `.crs` directly (no compilation step), merges local `.crs` imports,
//!   or executes a `.loy`. If the program imports Rust (`.rs`), delegates to
//!   `cforge` (transpile + native compile).
//! * `alloy build <file.crs>` — compiles to a portable `.loy` artifact
//!   (serialized AST, with local `.crs` files already merged).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use alloy_vm::interp::Interpreter;
use alloy_vm::loader::{self, LoadOutcome};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "alloy", about = "Alloy interpreter for Copper")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Interprets a `.crs` (instantly) or executes a `.loy`.
    Run { file: PathBuf },
    /// Compiles a `.crs` to a portable `.loy` artifact.
    Build {
        file: PathBuf,
        /// Output path (default: same name with `.loy` extension).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Checks a `.crs` with real Rust (borrow checker + Miri) — delegates to cforge,
    /// which provisions the toolchain automatically.
    Check {
        file: PathBuf,
        /// Type + borrow check only (cargo check), without running Miri.
        #[arg(long = "no-miri")]
        no_miri: bool,
    },
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Run { file } => run(&file),
        Cmd::Build { file, output } => build(&file, output.as_deref()),
        Cmd::Check { file, no_miri } => check(&file, no_miri),
    }
}

/// `alloy check` delegates to `cforge check` (which transpiles + runs the Rust
/// checker with an automatically provisioned toolchain).
fn check(file: &Path, no_miri: bool) -> ExitCode {
    for cf in cforge_candidates() {
        let mut cmd = Command::new(&cf);
        cmd.arg("check").arg(file);
        if no_miri {
            cmd.arg("--no-miri");
        }
        if let Ok(status) = cmd.status() {
            return ExitCode::from(status.code().unwrap_or(1) as u8);
        }
    }
    eprintln!(
        "alloy: could not find `cforge` to check. Install cforge and run: cforge check {}",
        file.display()
    );
    ExitCode::FAILURE
}

fn run(file: &Path) -> ExitCode {
    match loader::load_runnable(file) {
        Ok(LoadOutcome::Program(prog)) => match Interpreter::new().run_program(&prog) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!(
                    "alloy: runtime error @ {}..{}: {}",
                    e.span.start, e.span.end, e.message
                );
                ExitCode::FAILURE
            }
        },
        // Imported Rust: the interpreter does not run `.rs` → delegate to cforge.
        Ok(LoadOutcome::NeedsCforge { module, .. }) => {
            eprintln!("alloy: `{module}` is Rust (.rs); delegating to cforge…");
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
            eprintln!("alloy: could not read {}: {e}", file.display());
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
                    println!("compiled: {}", out.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("alloy: could not write {}: {e}", out.display());
                    ExitCode::FAILURE
                }
            }
        }
        Ok(LoadOutcome::NeedsCforge { module, .. }) => {
            eprintln!(
                "alloy build: `{module}` is Rust (.rs) — `.loy` does not embed Rust.\n\
                 Compile natively with: cforge build {}",
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

/// Locates the `cforge` binary and runs `cforge run <file>`, propagating the
/// exit code. Looks beside the `alloy` binary itself, then in PATH.
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
        "alloy: could not find `cforge` to compile the Rust. \
         Install cforge and run: cforge run {}",
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
    // fallback: resolve via PATH
    v.push(PathBuf::from("cforge"));
    v
}
