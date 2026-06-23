//! Alloy CLI: tree-walking interpreter for Copper.
//!
//! * `alloy run <file>` — runs instantly (node/python style): interprets a
//!   `.crs` directly (no compilation step), merges local `.crs` imports,
//!   or executes a `.loy`. Imported Rust (`.rs`) runs via embedded wasm
//!   (compiled once + cached); non-exported `.rs` falls back to `cforge`.
//! * `alloy build <file.crs>` — compiles to a portable `.loy` artifact
//!   (serialized AST + any imported `.rs` compiled to embedded wasm, so the
//!   artifact runs on any `alloy` without rustc).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::rc::Rc;

use alloy_vm::interp::Interpreter;
use alloy_vm::loader;
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
    // Resolve the Copper program + Rust to run via wasm (`.rs` to compile, or
    // modules already embedded in a `.loy`).
    let resolved = match loader::resolve_runnable_wasm(file) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("alloy: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut interp = Interpreter::new();

    // Wasm already embedded in a `.loy` — no rustc needed.
    for m in &resolved.wasm_modules {
        if let Err(e) = interp.register_wasm_bytes(&m.names, &m.bytes) {
            eprintln!("alloy: embedded wasm failed to load: {e}");
            return ExitCode::FAILURE;
        }
    }

    // `.rs` imports — compile to wasm (cached) and instantiate.
    for imp in &resolved.rs_imports {
        let rt = match alloy_vm::wasm::WasmRuntime::from_rs(&imp.rs_path) {
            Ok(rt) => rt,
            Err(e) => {
                // Couldn't compile/run the Rust as wasm → fall back to cforge.
                eprintln!(
                    "alloy: could not run {} via wasm ({e}); delegating to cforge…",
                    imp.rs_path.display()
                );
                return delegate_to_cforge(file);
            }
        };
        // Every imported name must be a wasm export. Plain `pub fn` (no
        // `#[no_mangle] pub extern \"C\"`) isn't exported — those `.rs` files
        // need the richer ABI that isn't in the prototype yet, so delegate.
        if let Some(missing) = imp.names.iter().find(|n| !rt.exports(n)) {
            eprintln!(
                "alloy: `{missing}` from {} isn't a wasm export (needs `#[no_mangle] pub extern \"C\"`); delegating to cforge…",
                imp.rs_path.display()
            );
            return delegate_to_cforge(file);
        }
        interp.register_wasm(&imp.names, Rc::new(RefCell::new(rt)));
    }

    match interp.run_program(&resolved.program) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!(
                "alloy: runtime error @ {}..{}: {}",
                e.span.start, e.span.end, e.message
            );
            ExitCode::FAILURE
        }
    }
}

fn build(file: &Path, output: Option<&Path>) -> ExitCode {
    let resolved = match loader::resolve_runnable_wasm(file) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("alloy: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Compile each imported `.rs` to wasm and embed it in the `.loy` so the
    // artifact is self-contained (runs on any `alloy` without rustc).
    let mut modules = resolved.wasm_modules.clone();
    for imp in &resolved.rs_imports {
        match alloy_vm::wasm::compile_rs_to_wasm(&imp.rs_path) {
            Ok(bytes) => modules.push(alloy_vm::bytecode::WasmModule {
                names: imp.names.clone(),
                bytes,
            }),
            Err(e) => {
                eprintln!(
                    "alloy build: could not compile {} to wasm: {e}",
                    imp.rs_path.display()
                );
                return ExitCode::FAILURE;
            }
        }
    }

    let out = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| file.with_extension("loy"));
    let bytes = alloy_vm::bytecode::compile(&resolved.program.items, &modules);
    match std::fs::write(&out, bytes) {
        Ok(()) => {
            if modules.is_empty() {
                println!("compiled: {}", out.display());
            } else {
                println!(
                    "compiled: {} ({} embedded wasm module(s))",
                    out.display(),
                    modules.len()
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("alloy: could not write {}: {e}", out.display());
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
