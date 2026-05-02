use std::env;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Force build.rs to rerun if Cargo.toml or lson changes
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=lson");

    // Capture the build date as a UTC `YYYY-MM-DD` string so the binary can
    // surface it in `--version` output for pre-release builds.
    //
    // To keep the date fresh, force this script to rerun on every cargo
    // invocation: pointing `rerun-if-changed` at a path that does not exist
    // makes cargo unable to stat it, which it treats as "always changed".
    // Without this, the explicit `rerun-if-changed` lines above would limit
    // reruns to Cargo.toml / lson and the date could go stale across builds.
    println!("cargo:rerun-if-changed=.cforge-build-date-trigger");
    println!("cargo:rustc-env=COPPER_BUILD_DATE={}", build_date());

    // Get the build profile (debug or release)
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());

    // Define the output directory based on the profile
    let target_dir = Path::new("target").join(profile);

    // Destination path where Cargo.toml will be copied
    let dest_cargo = target_dir.join("Cargo.toml");

    // Source path for Cargo.toml
    let src_cargo = Path::new("Cargo.toml");

    // Copy Cargo.toml
    match fs::copy(src_cargo, &dest_cargo) {
        Ok(_) => println!("Cargo.toml successfully copied to {:?}", dest_cargo),
        Err(e) => eprintln!("Error copying Cargo.toml: {}", e),
    }

    // Copy lson directory recursively
    let src_lson = Path::new("lson");
    let dest_lson = target_dir.join("lson");

    if src_lson.exists() {
        if let Err(e) = copy_dir_all(src_lson, &dest_lson) {
            eprintln!("Error copying lson directory: {}", e);
        } else {
            println!("lson directory successfully copied to {:?}", dest_lson);
        }
    }
}

/// Returns the current UTC date as `YYYY-MM-DD`, computed without any
/// external date crate. Falls back to `unknown` if the system clock is
/// before the UNIX epoch.
fn build_date() -> String {
    let secs = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => return "unknown".to_string(),
    };
    let days = secs.div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Convert a count of days since 1970-01-01 (UTC) into a (year, month, day)
/// tuple. Algorithm from Howard Hinnant's public-domain date library:
/// http://howardhinnant.github.io/date_algorithms.html#civil_from_days
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let mut y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    if m <= 2 {
        y += 1;
    }
    (y, m, d)
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
