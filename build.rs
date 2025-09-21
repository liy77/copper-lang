use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Force build.rs to rerun if Cargo.toml or lson changes
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=lson");

    // Get the build profile (debug or release)
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());

    // Define the output directory based on the profile
    let target_dir = Path::new("target").join(profile);

    // Destination path where Cargo.toml will be copied
    let dest_cargo = target_dir.join("Cargo.toml");

    // Source path for Cargo.toml
    let src_cargo = Path::new("Cargo.toml");

    // Copy Cargo.toml
    match fs::copy(&src_cargo, &dest_cargo) {
        Ok(_) => println!("Cargo.toml successfully copied to {:?}", dest_cargo),
        Err(e) => eprintln!("Error copying Cargo.toml: {}", e),
    }

    // Copy lson directory recursively
    let src_lson = Path::new("lson");
    let dest_lson = target_dir.join("lson");

    if src_lson.exists() {
        if let Err(e) = copy_dir_all(&src_lson, &dest_lson) {
            eprintln!("Error copying lson directory: {}", e);
        } else {
            println!("lson directory successfully copied to {:?}", dest_lson);
        }
    }
}

// Helper function to copy directories recursively
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
