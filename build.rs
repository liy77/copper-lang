use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Force rerun when Cargo.toml changes.
    println!("cargo:rerun-if-changed=Cargo.toml");

    // Force this script to rerun on every cargo invocation so the build date
    // and derived version stay fresh. Pointing rerun-if-changed at a path that
    // does not exist makes cargo treat it as "always changed".
    println!("cargo:rerun-if-changed=.cforge-build-date-trigger");

    // Rerun when the current commit changes.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/");

    let (y, m, _d) = build_ymd();

    // CalVer: 0.YY.M  (e.g. 2026-06 → 0.26.6) — shared by both cforge and copper.
    let yy = y % 100;
    println!("cargo:rustc-env=CFORGE_VERSION=0.{}.{}", yy, m);
    println!("cargo:rustc-env=COPPER_VERSION=0.{}.{}", yy, m);
    println!(
        "cargo:rustc-env=COPPER_BUILD_DATE={:04}-{:02}-{:02}",
        y, m, _d
    );

    // Short git commit hash — falls back to "unknown" if git is unavailable.
    let hash = git_short_hash();
    println!("cargo:rustc-env=GIT_COMMIT_HASH={}", hash);

    // Copy Cargo.toml next to the binary so cforge can read project metadata.
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let target_dir = Path::new("target").join(profile);
    let dest_cargo = target_dir.join("Cargo.toml");
    if let Err(e) = fs::copy("Cargo.toml", &dest_cargo) {
        eprintln!(
            "Warning: could not copy Cargo.toml to {:?}: {}",
            dest_cargo, e
        );
    }
}

/// Returns the short git commit hash (7 chars), or `"unknown"` if git is unavailable.
fn git_short_hash() -> String {
    use std::process::Command;
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Returns the current UTC (year, month, day).
fn build_ymd() -> (i32, u32, u32) {
    let secs = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => return (1970, 1, 1),
    };
    civil_from_days(secs.div_euclid(86_400))
}

/// Algorithm from Howard Hinnant's public-domain date library:
/// http://howardhinnant.github.io/date_algorithms.html#civil_from_days
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    if m <= 2 {
        y += 1;
    }
    (y, m, d)
}
