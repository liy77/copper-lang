use serde_json::Value;
use std::path::PathBuf;
use std::process::{exit, Command};
use std::{env, fs};

use crate::cforge::COPPER_PATH;

/// Locate the lson binary, building it from lson-src/ if necessary.
///
/// Resolution order:
///   1. `{COPPER_PATH}/lson/lson[.exe]`                         — installed
///   2. `{project_root}/lson-src/target/release/lson[.exe]`     — dev build
///   3. Auto-build via `cargo build --release` in lson-src/     — first run
fn find_lson_binary() -> PathBuf {
    let binary_name = if cfg!(windows) { "lson.exe" } else { "lson" };
    let copper_path = PathBuf::from(&*COPPER_PATH);

    // 1. Installed path: {COPPER_PATH}/lson/lson[.exe]
    let installed = copper_path.join("lson").join(binary_name);
    if installed.exists() {
        return installed;
    }

    // Resolve project root in dev/debug mode (exe lives in target/debug/).
    let exe_dir = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let in_debug = exe_dir
        .to_str()
        .map(|s| {
            s.contains(&format!("target{}debug", std::path::MAIN_SEPARATOR))
                || s.contains("target/debug")
        })
        .unwrap_or(false);

    let project_root: Option<PathBuf> = if in_debug {
        exe_dir.join("../..").canonicalize().ok()
    } else {
        None
    };

    // 2. Dev build already present.
    if let Some(ref root) = project_root {
        let dev_bin = root
            .join("lson-src")
            .join("target")
            .join("release")
            .join(binary_name);
        if dev_bin.exists() {
            return dev_bin;
        }
    }

    // 3. Auto-build from lson-src/ (first-run or missing binary).
    let lson_src = project_root
        .as_ref()
        .map(|r| r.join("lson-src"))
        .filter(|p| p.join("Cargo.toml").exists())
        .or_else(|| {
            let p = copper_path.join("lson-src");
            if p.join("Cargo.toml").exists() {
                Some(p)
            } else {
                None
            }
        });

    if let Some(src) = lson_src {
        println!("lson not found — building from lson-src/ (first run)…");
        let manifest = src.join("Cargo.toml");
        let target_dir = src.join("target");
        let status = Command::new("cargo")
            .args(["build", "--release"])
            .arg("--manifest-path")
            .arg(&manifest)
            .arg("--target-dir")
            .arg(&target_dir)
            .status();
        match status {
            Ok(s) if s.success() => {
                let built = src.join("target").join("release").join(binary_name);
                if built.exists() {
                    return built;
                }
            }
            Ok(s) => eprintln!("Warning: lson build exited with {:?}", s.code()),
            Err(e) => eprintln!("Warning: could not run cargo to build lson: {e}"),
        }
    }

    // Return the expected installed path — caller will get a clear OS error.
    installed
}

fn parse(text: &str) -> Value {
    let lson_bin = find_lson_binary();

    // In debug mode the exe lives in target/debug/; step up to the project root
    // so lson resolves relative file references correctly.
    let exe_dir = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let current_dir = if exe_dir
        .to_str()
        .unwrap()
        .contains(&format!("target{}debug", std::path::MAIN_SEPARATOR))
    {
        exe_dir.join("../../")
    } else {
        exe_dir
    };

    let cmd = Command::new(&lson_bin)
        .current_dir(current_dir)
        .arg("raw")
        .arg("compile")
        .args(["-t", "json"])
        .args(["--text", text])
        .output();

    match cmd {
        Ok(out) => {
            let mut stdout = String::from_utf8_lossy(&out.stdout);
            if stdout
                .lines()
                .next()
                .map(|l| l.starts_with("warning "))
                .unwrap_or(false)
            {
                stdout = stdout.lines().skip(1).collect();
            }
            serde_json::from_str(&stdout).expect("Invalid KSON file")
        }
        Err(e) => {
            println!("Failed to execute lson: {}", e);
            exit(1);
        }
    }
}

/// Read properties from a KSON or TOML file.
/// If both files exist, KSON takes precedence.
///
/// Returns (is_toml, Value)
pub fn read_properties(file: &str) -> (bool, Value) {
    let mut file = file.to_string();
    let mut c = fs::read(&file);
    if c.is_err() {
        file = file.replace("properties.kson", "Cargo.toml");
        c = fs::read(&file);
        if c.is_err() {
            println!("Error: Missing properties.kson or Cargo.toml. Please ensure the file exists and is readable.");
            exit(1);
        }
    }

    if file.ends_with("Cargo.toml") {
        let text = String::from_utf8(c.unwrap()).unwrap();
        let parsed: toml::Value = toml::from_str(&text).expect("Invalid Cargo.toml file");
        return (true, serde_json::to_value(parsed).unwrap());
    }

    let text = String::from_utf8(c.unwrap()).unwrap();
    (false, parse(&text))
}
