pub mod tokenizer;
pub mod utils;
pub mod parser;
pub mod cforge;

use std::env;
use std::process::Command;

pub use utils::*;

fn is_command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[tokio::main]
async fn main() {
    if !is_command_available("cargo") {
        println!("🦀 Cargo is not installed. Please install it to continue.");
        return;
    }

    if !is_command_available("rustc") {
        println!("🦀 Rust is not installed. Please install it to continue.");
        return;
    }

    env::set_var("CFORGE_VERSION", "0.1.0");
    env::set_var("COPPER_VERSION", "0.1.0");
    env::set_var("COPPER_PATH", env::current_exe().unwrap().parent().unwrap().to_str().unwrap());
    
    cforge::print();
    cforge::compile(vec!["./main.crs".to_string()]);
    cforge::generate_toml().await;
    cforge::run();
}
