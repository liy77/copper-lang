// The full Copper parser is still being incrementally restored from a
// previous stub state; pre-existing untouched code carries a backlog of
// stylistic clippy warnings. We silence the categories that aren't bugs
// here so `cargo clippy -- -D warnings` stays green for CI. Tighten this
// list as the relevant code paths are rewritten.
#![allow(
    clippy::borrow_interior_mutable_const,
    clippy::declare_interior_mutable_const,
    clippy::doc_lazy_continuation,
    clippy::if_same_then_else,
    clippy::manual_strip,
    clippy::module_inception,
    clippy::needless_late_init,
    clippy::redundant_locals,
    clippy::to_string_trait_impl,
    clippy::unnecessary_unwrap,
    clippy::while_let_loop
)]

pub mod cforge;
pub mod utils;

// Re-export the tokenizer module from copper-syntax so existing paths
// (`crate::tokenizer::tokens::Token`, etc.) keep resolving. cforge drives
// the same tokenizer the parser and LSP consume.
pub use copper_syntax::tokenizer;

// The parser now lives in its own crate (`copper-parser`). Re-export it at
// the crate root so `crate::parser::Parser` keeps resolving in cforge.
pub use copper_parser::parser;

use clap::{Arg, Command as ClapCommand};
use std::process::Command as ProcessCommand;
use std::{env, fs, path};

use utils::parsed_command::{ParsedCommand, ParsedCommands};
pub use utils::*;

use once_cell::sync::Lazy;

static BASE_CMD: Lazy<ClapCommand> = Lazy::new(|| {
    ClapCommand::new("cforge")
        .arg(Arg::new("input")
            .short('i')
            .long("input")
            .help("Input files to compile"))
        .arg(Arg::new("output")
            .short('o')
            .long("output")
            .help("Output directory for compiled files"))
        .arg(Arg::new("compile")
            .short('c')
            .long("compile")
            .action(clap::ArgAction::SetTrue)
            .default_value("false")
            .help("Compile the project"))
        .arg(Arg::new("clean")
            .long("clean")
            .action(clap::ArgAction::SetTrue)
            .help("Clean the output directory before compiling"))
        .arg(Arg::new("release")
            .short('r')
            .long("release")
            .action(clap::ArgAction::SetTrue)
            .help("Compile in release mode"))
        .arg(Arg::new("bundle")
            .short('b')
            .long("bundle")
            .action(clap::ArgAction::SetTrue)
            .help("Embed the app.bundle (assets + config) into the executable for a self-contained binary (.mui only)"))
        .arg(Arg::new("version")
            .short('v')
            .long("version")
            .action(clap::ArgAction::SetTrue)
            .help("Show version information"))
        .arg(Arg::new("target")
            .long("target")
            .num_args(1)
            .value_name("TARGET")
            .help("Cross-compile target (friendly names: windows, mac, linux or full rust triple)")
        )
        .arg(Arg::new("verbose")
            .short('V')
            .long("verbose")
            .action(clap::ArgAction::SetTrue)
            .help("Enable verbose output"))
        .subcommand(ClapCommand::new("run")
            .about("Compile and run the project")
            .args([
                Arg::new("input")
                    .help("Input files to compile")
                    .value_name("FILE")
                    .required(false)
                    .index(1),
                Arg::new("output")
                    .short('o')
                    .long("output")
                    .help("Output directory for compiled files"),
            ])
        )
        .subcommand(ClapCommand::new("init")
            .about("Scaffold a new Copper project (properties.kson, main.crs, .gitignore)")
        )
        .subcommand(ClapCommand::new("install")
            .about("Install dependencies. With no NAME, resolves + downloads every dependency in properties.kson and writes properties.lock. With a NAME, adds/pins that one.")
            .arg(Arg::new("package")
                .help("Crate to install, optionally pinned: name or name@version. Omit to install everything in properties.kson.")
                .value_name("NAME[@VERSION]")
                .required(false)
                .index(1))
        )
});

/// Build date stamped in by `build.rs` at compile time (UTC, `YYYY-MM-DD`).
const BUILD_DATE: &str = env!("COPPER_BUILD_DATE");

/// Append the build date to a version string when it is a pre-release
/// (contains `alpha`, `beta`, or `rc`). Stable versions are returned as-is.
fn with_build_date(version: &str) -> String {
    let v = version.to_ascii_lowercase();
    if v.contains("alpha") || v.contains("beta") || v.contains("rc") {
        format!("{} (build {})", version, BUILD_DATE)
    } else {
        version.to_string()
    }
}

fn is_command_available(command: &str) -> bool {
    ProcessCommand::new(command)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn parse_commands() -> ParsedCommands {
    let matches = BASE_CMD.clone().get_matches();

    let mut parsed_args = ParsedCommands::new();

    // Handle flags (boolean arguments)
    for flag in [
        "version", "verbose", "clean", "compile", "release", "bundle",
    ] {
        if let Some(value) = matches.get_one::<bool>(flag) {
            let mut cmd = ParsedCommand::new(flag.to_string(), vec![]);
            cmd.set_valid(*value);
            parsed_args.add_command(cmd);
        }
    }

    // Process input files
    if let Some(file_path) = matches.get_one::<String>("input") {
        let path = path::Path::new(file_path);

        if !path.exists() {
            eprintln!("Error: '{}' does not exist", file_path);
            std::process::exit(1);
        }

        let is_dir = path.is_dir();
        let is_file = path.is_file();
        let mut files = Vec::new();

        if is_dir {
            // Add directory as first item to identify input as directory
            files.push(file_path.to_string());

            // Recursively collect all files in directory
            for entry in walkdir::WalkDir::new(path)
                .follow_links(true)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.path().is_file())
            {
                files.push(entry.path().to_string_lossy().into_owned());
            }
        } else if is_file {
            files.push(file_path.to_string());
        } else {
            eprintln!("Error: '{}' is neither a file nor directory", file_path);
            std::process::exit(1);
        }

        let mut cmd = ParsedCommand::new("input".to_string(), files);
        cmd.set_file(is_file);
        cmd.set_dir(is_dir);
        cmd.set_valid(true);
        parsed_args.add_command(cmd);
    }

    // Process output directory
    let output_dir = matches
        .get_one::<String>("output")
        .map(String::from)
        .unwrap_or_else(|| "./dist/".to_string());

    // Create output directory if it doesn't exist
    if !path::Path::new(&output_dir).exists() {
        match fs::create_dir_all(&output_dir) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Error creating output directory '{}': {}", output_dir, e);
                std::process::exit(1);
            }
        }
    } else if !path::Path::new(&output_dir).is_dir() {
        eprintln!("Error: '{}' exists but is not a directory", output_dir);
        std::process::exit(1);
    }

    let mut cmd = ParsedCommand::new("output".to_string(), vec![output_dir]);
    cmd.set_valid(true);
    parsed_args.add_command(cmd);

    // Handle subcommands
    if let Some(("run", run_matches)) = matches.subcommand() {
        // Process run subcommand
        let mut cmd = ParsedCommand::new("run".to_string(), vec![]);
        cmd.set_valid(true);
        parsed_args.add_command(cmd);

        // Override input if provided in run subcommand
        if let Some(file_path) = run_matches.get_one::<String>("input") {
            let path = path::Path::new(file_path);

            if !path.exists() {
                eprintln!("Error: '{}' does not exist", file_path);
                std::process::exit(1);
            }

            let is_dir = path.is_dir();
            let is_file = path.is_file();
            let mut files = Vec::new();

            if is_dir {
                files.push(file_path.to_string());
                for entry in walkdir::WalkDir::new(path)
                    .follow_links(true)
                    .into_iter()
                    .filter_map(Result::ok)
                    .filter(|e| e.path().is_file())
                {
                    files.push(entry.path().to_string_lossy().into_owned());
                }
            } else if is_file {
                files.push(file_path.to_string());
            }

            // Replace the existing input command with run-specific input
            let mut cmd = ParsedCommand::new("input".to_string(), files);
            cmd.set_file(is_file);
            cmd.set_dir(is_dir);
            cmd.set_valid(true);
            parsed_args.update_or_add_command(cmd);
        } else {
            // If no file specified for run, try using main.crs as default
            let default_file = "main.crs";
            if path::Path::new(default_file).exists() {
                let mut cmd =
                    ParsedCommand::new("input".to_string(), vec![default_file.to_string()]);
                cmd.set_file(true);
                cmd.set_dir(false);
                cmd.set_valid(true);
                parsed_args.update_or_add_command(cmd);
            } else {
                eprintln!(
                    "Error: No input file specified and '{}' does not exist",
                    default_file
                );
                std::process::exit(1);
            }
        }

        // Override output if provided in run subcommand
        if let Some(output_path) = run_matches.get_one::<String>("output") {
            let output_dir = output_path.to_string();

            if !path::Path::new(&output_dir).exists() {
                match fs::create_dir_all(&output_dir) {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("Error creating output directory '{}': {}", output_dir, e);
                        std::process::exit(1);
                    }
                }
            }

            let mut cmd = ParsedCommand::new("output".to_string(), vec![output_dir]);
            cmd.set_valid(true);
            parsed_args.update_or_add_command(cmd);
        }
    }

    parsed_args
}

#[tokio::main]
async fn main() {
    // Project-management subcommands run before the toolchain checks below:
    // `init` only writes files, and `install` only talks to crates.io —
    // neither needs cargo/rustc present.
    match BASE_CMD.clone().get_matches().subcommand() {
        Some(("init", _)) => {
            cforge::commands::init();
            return;
        }
        Some(("install", install_matches)) => {
            match install_matches.get_one::<String>("package") {
                // `cforge install <name>` — add/pin a single dependency.
                Some(package) => cforge::commands::install(package).await,
                // `cforge install` — resolve + download everything in
                // properties.kson and write properties.lock.
                None => cforge::commands::install_all().await,
            }
            return;
        }
        _ => {}
    }

    if !is_command_available("cargo") {
        println!("🦀 Cargo is not installed. Please install it to continue.");
        return;
    }

    if !is_command_available("rustc") {
        println!("🦀 Rust is not installed. Please install it to continue.");
        return;
    }

    env::set_var("CFORGE_VERSION", "0.1.0");
    env::set_var("COPPER_VERSION", "0.1.0-alpha.1");
    if env::var("COPPER_PATH").is_err() {
        env::set_var(
            "COPPER_PATH",
            env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .to_str()
                .unwrap(),
        );
    }

    let commands = parse_commands();

    if commands.get_command("version").unwrap().is_valid {
        let cforge_version = env::var("CFORGE_VERSION").unwrap();
        let copper_version = env::var("COPPER_VERSION").unwrap();
        println!("CForge v{}", with_build_date(&cforge_version));
        println!("Copper v{}", with_build_date(&copper_version));
        return;
    }

    if commands.get_command("verbose").unwrap().is_valid {
        env::set_var("CFORGE_VERBOSE", "1");
    } else {
        env::set_var("CFORGE_VERBOSE", "0");
    }

    let files_cmd = commands.get_command("input");
    if files_cmd.is_none() {
        println!("{}", BASE_CMD.clone().render_help());
        return;
    }

    // Input exists, continue
    let files_cmd = files_cmd.unwrap();
    let files = files_cmd.args.clone();
    let input_dir = if files_cmd.is_dir {
        Some(files[0].clone())
    } else {
        None
    };

    let output_dir = commands.get_command("output").unwrap().args.clone();
    let output_dir = if !output_dir.is_empty() {
        Some(output_dir[0].clone())
    } else {
        None
    };

    let files = if input_dir.is_some() {
        files[1..].to_vec() // Skip the first file which is the directory itself
    } else {
        files
    };

    cforge::print();

    if commands.get_command("clean").unwrap().is_valid {
        let output_dir = output_dir.clone().unwrap_or_else(|| "./dist/".to_string());
        if path::Path::new(&output_dir).exists() {
            fs::remove_dir_all(&output_dir).unwrap();
            fs::create_dir_all(&output_dir).unwrap();
            println!("🧹 Cleaned output directory: {}", output_dir);
        } else {
            println!("🧹 Output directory does not exist: {}", output_dir);
        }
    }

    let is_release = commands.get_command("release").unwrap().is_valid;
    let embed_bundle = commands
        .get_command("bundle")
        .map(|c| c.is_valid)
        .unwrap_or(false);

    // Resolve --target early so both the compile and run paths can see it.
    let target_triple: Option<&'static str> = BASE_CMD
        .clone()
        .get_matches()
        .get_one::<String>("target")
        .map(|t| cforge::resolve_target(t));

    if let Some(triple) = target_triple {
        env::set_var("CFORGE_TARGET", triple);
    }
    if is_release {
        env::set_var("CFORGE_RELEASE", "1");
    }

    if commands.get_command("compile").unwrap().is_valid {
        // A single `.mui` / `.crm` is a mocida UI: "compiling" it means lowering
        // the component AST to Rust (mui-codegen) rather than the Copper→Rust
        // transpile. Same `-c` entry point, file extension picks the backend.
        // `--release` additionally `cargo build`s the generated crate; `--bundle`
        // embeds the app.bundle (assets + config) into the binary.
        if input_dir.is_none() && files.len() == 1 && cforge::mui::is_mui_file(&files[0]) {
            let code =
                cforge::mui::compile(&files[0], output_dir.as_deref(), is_release, embed_bundle);
            println!();
            std::process::exit(code);
        }

        let detected_dependencies =
            cforge::compile(files.clone(), input_dir.clone(), output_dir.clone());
        cforge::generate_toml(detected_dependencies).await;

        // When --target is set, also cargo-build for that target and copy the
        // binary to dist/. Without --target, -c stops at the Rust source.
        if let Some(triple) = target_triple {
            cforge::build_for_target(triple, is_release);
        }
    }

    // Handle run subcommand
    if commands.get_command("run").is_some() && commands.get_command("run").unwrap().is_valid {
        // A single `.mui` / `.crm` file is a mocida UI: render it live via the
        // mui-dev host instead of going through the Copper→Rust→cargo pipeline.
        // (Transpiling `.crm` to a native binary is the M5 release path.)
        if input_dir.is_none() && files.len() == 1 && cforge::mui::is_mui_file(&files[0]) {
            let code = cforge::mui::run(&files[0]);
            println!();
            std::process::exit(code);
        }

        let detected_dependencies = cforge::compile(files, input_dir, output_dir.clone());
        cforge::generate_toml(detected_dependencies).await;
        cforge::run();
    }

    println!();
}
