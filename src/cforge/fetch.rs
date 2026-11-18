use colored::Colorize;
use once_cell::sync::Lazy;
use serde_json::Value;
use std::error::Error;
use std::time::Duration;

use crate::cforge::cache::RegistryCache;

const CRATES_IO_URL: &str = "https://crates.io";

static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent("CForge/0.1.0")
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
});

/// Build a sortable key for a crates.io version entry so `max_by` selects the
/// newest *usable* version. Ordering, highest-wins:
///   1. non-yanked over yanked
///   2. stable release over pre-release (`1.0.0` over `1.0.0-rc.1`)
///   3. numeric semver (major, minor, patch) — never string comparison
fn semver_sort_key(v: &Value) -> (bool, bool, u64, u64, u64) {
    let yanked = v["yanked"].as_bool().unwrap_or(false);
    let num = v["num"].as_str().unwrap_or("");

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

/// Check if a version exists in the registry (with persistent disk cache in
/// `~/.copper/cache/registry.json`).
pub async fn check_version_exists(
    crate_name: &str,
    version: &str,
    registry: Option<&str>,
) -> Result<(bool, String), Box<dyn Error>> {
    let clean_ver = version.trim().trim_matches('"');

    // Fast path: check persistent local registry cache first.
    let mut reg_cache = RegistryCache::load();
    if let Some(cached_ver) = reg_cache.get(crate_name, clean_ver) {
        return Ok((true, cached_ver));
    }

    let url = format!(
        "{}/api/v1/crates/{}/versions",
        registry.unwrap_or(CRATES_IO_URL),
        crate_name
    );
    let resp = match HTTP_CLIENT.get(&url).send().await {
        Ok(r) => r,
        Err(_) => {
            return Ok((false, clean_ver.to_string()));
        }
    };
    let response = resp.text().await?;
    let json: Value = serde_json::from_str(&response)?;

    if let Some(versions) = json["versions"].as_array() {
        if matches!(clean_ver, "latest" | "*") {
            if let Some(latest_version) = versions
                .iter()
                .max_by(|a, b| semver_sort_key(a).cmp(&semver_sort_key(b)))
            {
                if let Some(latest) = latest_version["num"].as_str() {
                    let is_deprecated = latest_version["yanked"].as_bool().unwrap_or(false);
                    let resolved = latest.to_string();

                    if is_deprecated {
                        println!(
                            "💀 Yanked dependency: {} {} {}",
                            crate_name.red(),
                            "=>".yellow(),
                            resolved.black()
                        );
                    }

                    reg_cache.put(crate_name, clean_ver, &resolved);
                    reg_cache.save();
                    return Ok((true, resolved));
                }
            }
        }

        // Exact or prefix match (e.g. "1" or "0.31" or "1.0.150")
        for v in versions {
            if let Some(v_num) = v["num"].as_str() {
                if v_num == clean_ver || v_num.starts_with(&format!("{clean_ver}.")) {
                    let is_deprecated = v["yanked"].as_bool().unwrap_or(false);
                    if is_deprecated {
                        println!(
                            "💀 Yanked dependency: {} {} {}",
                            crate_name.red(),
                            "=>".yellow(),
                            v_num.black()
                        );
                    }
                    reg_cache.put(crate_name, clean_ver, v_num);
                    reg_cache.save();
                    return Ok((true, v_num.to_string()));
                }
            }
        }
    }

    Ok((false, clean_ver.to_string()))
}
