use std::path::Path;
use std::{env, fs};
use std::process::{exit, Command};
use serde_json::Value;

use crate::cforge::COPPER_PATH;

fn parse(text: &str) -> Value {
    let exe_dir = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let binding = COPPER_PATH;
    let cop_path = Path::new(binding.as_str());
    let path = if cfg!(target_os = "windows") {
        exe_dir.join(cop_path.join(format!("lson{}win32{}lson.exe", std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR)))
    } else {
        exe_dir.join(cop_path.join(format!("lson{}unix{}lson", std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR)))
    };

    let path = path.as_os_str();

    let dir_binding = env::current_exe().unwrap();
    let current_dir = dir_binding.parent().unwrap();
    let current_dir = if current_dir.to_str().unwrap().contains(&format!("target{}debug", std::path::MAIN_SEPARATOR)) {
        current_dir.join("../../")
    } else {
        current_dir.to_path_buf()
    };

    let cmd = Command::new(path)
        .current_dir(current_dir)
        .arg("raw")
        .arg("compile")
        .args(["-t", "json"])
        .args(["--text", text])
        .output();

    if cmd.is_ok() {
        let cmd = cmd.unwrap();
        let mut stdout = String::from_utf8_lossy(&cmd.stdout);

        if stdout.lines().nth(0).unwrap().starts_with("warning ") {
            stdout = stdout.lines().skip(1).collect();
        }

        let json = stdout.to_string();
        let parsed_json: Value = serde_json::from_str(&json).expect("Invalid KSON file");

        return parsed_json;
    } else {
        let cmd = cmd;
        if cmd.is_err() {
            let err = cmd.err().unwrap();
            println!("Failed to execute lson: {}", err);
            exit(1);
        }
        let cmd = cmd.unwrap();
        let stderr = String::from_utf8_lossy(&cmd.stderr);
        println!("Invalid configuration file: {}", stderr);
        exit(1);
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
        // No properties.kson file found, trying Cargo.toml
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

    // text = "@kmodel(./src/models/cforge.kmodel)\n".to_string() + &text;
    (false, parse(&text))
}