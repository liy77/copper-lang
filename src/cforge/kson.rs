use std::{env, fs};
use std::process::{exit, Command};
use serde_json::Value;

fn parse(text: &str) -> Value {
    let path = if cfg!(target_os = "windows") {
        "./lson/win32/lson.exe"
    } else {
        "./lson/linux/lson"
    };

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
        let cmd = cmd.unwrap();
        let stderr = String::from_utf8_lossy(&cmd.stderr);
        println!("Invalid configuration file: {}", stderr);
        exit(1);
    }
}

pub fn read_properties(file: &str) -> Value {
    let c = fs::read(file).unwrap();
    let text = String::from_utf8(c).unwrap();

    // text = "@kmodel(./src/models/cforge.kmodel)\n".to_string() + &text;
    parse(&text)
}