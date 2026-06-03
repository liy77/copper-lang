use colored::Colorize;
use serde_json::Value;
use std::{error::Error, process::Command};

const CRATES_IO_URL: &str = "https://crates.io";

/// Build a sortable key for a crates.io version entry so `max_by` selects the
/// newest *usable* version. Ordering, highest-wins:
///   1. non-yanked over yanked
///   2. stable release over pre-release (`1.0.0` over `1.0.0-rc.1`)
///   3. numeric semver (major, minor, patch) — never string comparison
///
/// Unparseable versions sort to the bottom of their yanked/pre tier.
fn semver_sort_key(v: &Value) -> (bool, bool, u64, u64, u64) {
    let yanked = v["yanked"].as_bool().unwrap_or(false);
    let num = v["num"].as_str().unwrap_or("");

    // Strip build metadata (`+...`) and detect/strip a pre-release (`-...`).
    let core = num.split('+').next().unwrap_or(num);
    let (main, is_release) = match core.split_once('-') {
        Some((m, _)) => (m, false),
        None => (core, true),
    };

    let mut parts = main.split('.');
    let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);

    (!yanked, is_release, major, minor, patch)
}

fn is_local_network_connected() -> bool {
    let mut output = Command::new("ping");

    if cfg!(target_os = "windows") {
        output.arg("-n").arg("1");
    } else {
        output.arg("-c").arg("1");
    }

    output.arg("8.8.8.8");

    let output = output.output();

    match output {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

// Check if a version exists in the registry and if is yanked
pub async fn check_version_exists(
    crate_name: &str,
    mut version: &str,
    registry: Option<&str>,
) -> Result<(bool, String), Box<dyn Error>> {
    if !is_local_network_connected() {
        println!("🛜 Could not connect to the internet. Skipping version check.");
        return Ok((false, version.to_string()));
    }

    let url = format!(
        "{}/api/v1/crates/{}/versions",
        registry.unwrap_or(CRATES_IO_URL),
        crate_name
    );
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("User-Agent", "CForge/0.1.0")
        .send()
        .await?
        .text()
        .await?;
    let json: Value = serde_json::from_str(&response)?;
    let modified_version = version.replace('"', "");
    version = &modified_version;

    if let Some(versions) = json["versions"].as_array() {
        if matches!(version, "latest" | "*") {
            // Pick the highest version by semver order, NOT lexicographically:
            // a string `max` ranks "1.0.99" above "1.0.200" because '9' > '2'.
            // Prefer non-yanked releases over pre-releases; only fall back to a
            // pre-release when nothing stable is published.
            if let Some(latest_version) = versions
                .iter()
                .max_by(|a, b| semver_sort_key(a).cmp(&semver_sort_key(b)))
            {
                let latest = latest_version["num"].as_str();
                if latest.is_some() {
                    let is_deprecated = latest_version["yanked"].as_bool().unwrap_or(false);
                    let version = latest.unwrap().to_string();

                    if is_deprecated {
                        println!(
                            "💀 Yanked dependency: {} {} {}",
                            crate_name.red(),
                            "=>".yellow(),
                            version.black()
                        );
                    }

                    return Ok((true, version));
                }
            }
        }

        for v in versions {
            if let Some(v_num) = v["num"].as_str() {
                if v_num == version {
                    let is_deprecated = v["yanked"].as_bool().unwrap_or(false);

                    if is_deprecated {
                        println!(
                            "💀 Yanked dependency: {} {} {}",
                            crate_name.red(),
                            "=>".yellow(),
                            v_num.black()
                        );
                    }

                    return Ok((true, v_num.to_string()));
                }
            }
        }
    }

    Ok((false, version.to_string()))
}
