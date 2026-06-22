//! `cforge check` — verificação Rust real (type + borrow checker + UB via Miri)
//! para um programa Copper, com o toolchain **auto-provisionado**: o usuário não
//! instala nada manualmente; o nightly + componente `miri` ficam num rustup
//! isolado em `~/.alloy/rustup`.
//!
//! Assume que o programa já foi transpilado para `dist/rust/` (o chamador roda
//! `cforge::compile` + `generate_toml` antes).

use std::path::PathBuf;
use std::process::Command;

/// Diretório do rustup isolado do Alloy (não mexe no rustup do usuário).
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

/// Garante nightly + componente `miri` no rustup isolado. Idempotente: só baixa
/// na primeira vez. Retorna o nome do toolchain a usar (`nightly`).
pub fn ensure_miri() -> Result<String, String> {
    if !rustup_available() {
        return Err(
            "o verificador usa o rustup (instalador padrão do Rust), que não está no PATH.\n\
             Instale uma vez em https://rustup.rs e rode de novo."
                .into(),
        );
    }
    let home = alloy_rustup_home();
    let _ = std::fs::create_dir_all(&home);

    // nightly (perfil mínimo) — idempotente.
    eprintln!("cforge check: garantindo toolchain de verificação (nightly + miri)…");
    let r = Command::new("rustup")
        .env("RUSTUP_HOME", &home)
        .args(["toolchain", "install", "nightly", "--profile", "minimal"])
        .status()
        .map_err(|e| format!("falha ao rodar rustup: {e}"))?;
    if !r.success() {
        return Err("não consegui instalar o toolchain nightly".into());
    }
    // componente miri.
    let r = Command::new("rustup")
        .env("RUSTUP_HOME", &home)
        .args(["component", "add", "miri", "--toolchain", "nightly"])
        .status()
        .map_err(|e| format!("falha ao adicionar miri: {e}"))?;
    if !r.success() {
        return Err("não consegui adicionar o componente miri".into());
    }
    Ok("nightly".into())
}

/// Roda a verificação sobre o crate já transpilado em `dist/rust/`.
/// `no_miri = true` faz só `cargo +nightly check` (type + borrow), sem interpretar.
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
            eprintln!("cforge check: OK — passou no verificador do Rust");
            0
        }
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("cforge check: falha ao executar o verificador: {e}");
            1
        }
    }
}
