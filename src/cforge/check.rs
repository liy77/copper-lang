//! `cforge check` — real Rust verification (type + borrow checker + UB via Miri)
//! for a Copper program, with an **auto-provisioned** toolchain: the user installs
//! nothing manually; nightly + the `miri` component live in an isolated rustup
//! under `~/.alloy/rustup`.
//!
//! Assumes the program has already been transpiled to `dist/rust/` (the caller
//! runs `cforge::compile` + `generate_toml` first).

use std::path::PathBuf;
use std::process::Command;

/// Directory of Alloy's isolated rustup (does not touch the user's rustup).
fn alloy_rustup_home() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".alloy").join("rustup")
}

fn rustup_available() -> bool {
    Command::new("rustup")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Ensures nightly + `miri` component in the isolated rustup. Idempotent: only
/// downloads on first run. Returns the toolchain name to use (`nightly`).
pub fn ensure_miri() -> Result<String, String> {
    if !rustup_available() {
        return Err(
            "the checker uses rustup (the standard Rust installer), which is not in PATH.\n\
             Install it once from https://rustup.rs and try again."
                .into(),
        );
    }
    let home = alloy_rustup_home();
    let _ = std::fs::create_dir_all(&home);

    // nightly (minimal profile) — idempotent.
    eprintln!("cforge check: ensuring verification toolchain (nightly + miri)…");
    let r = Command::new("rustup")
        .env("RUSTUP_HOME", &home)
        .args(["toolchain", "install", "nightly", "--profile", "minimal"])
        .status()
        .map_err(|e| format!("failed to run rustup: {e}"))?;
    if !r.success() {
        return Err("could not install the nightly toolchain".into());
    }
    // miri component.
    let r = Command::new("rustup")
        .env("RUSTUP_HOME", &home)
        .args(["component", "add", "miri", "--toolchain", "nightly"])
        .status()
        .map_err(|e| format!("failed to add miri: {e}"))?;
    if !r.success() {
        return Err("could not add the miri component".into());
    }
    Ok("nightly".into())
}

/// Runs verification on the already-transpiled crate in `dist/rust/`.
/// `no_miri = true` runs only `cargo +nightly check` (type + borrow), without Miri.
pub fn run_check(no_miri: bool) -> i32 {
    let toolchain = match ensure_miri() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cforge check: {e}");
            return 1;
        }
    };
    let home = alloy_rustup_home();
    let mut cmd = Command::new("cargo");
    cmd.arg(format!("+{toolchain}"));
    if no_miri {
        cmd.arg("check");
    } else {
        cmd.arg("miri").arg("run");
    }
    cmd.current_dir("./dist/rust")
        .env("RUSTUP_HOME", &home)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    match cmd.status() {
        Ok(s) if s.success() => {
            eprintln!("cforge check: OK — passed the Rust verifier");
            0
        }
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("cforge check: failed to run the verifier: {e}");
            1
        }
    }
}
