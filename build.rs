use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Force build.rs to rerun if Cargo.toml changes
    println!("cargo:rerun-if-changed=Cargo.toml");

    // Get the build profile (debug or release)
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());

    // Define the output directory based on the profile
    let target_dir = Path::new("target").join(profile);

    // Destination path where Cargo.toml will be copied
    let dest = target_dir.join("Cargo.toml");

    // Source path for Cargo.toml
    let src = Path::new("Cargo.toml");

    // Copy the file
    match fs::copy(&src, &dest) {
        Ok(_) => println!("Cargo.toml successfully copied to {:?}", dest),
        Err(e) => eprintln!("Error copying Cargo.toml: {}", e),
    }
}
