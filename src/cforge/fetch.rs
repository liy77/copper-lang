use std::error::Error;
use colored::Colorize;
use serde_json::Value;

const CRATES_IO_URL: &str = "https://crates.io";

pub async fn check_version_exists(crate_name: &str, mut version: &str, registry: Option<&str>) -> Result<(bool, String), Box<dyn Error>> {
    let url = format!("{}/api/v1/crates/{}/versions", registry.unwrap_or(CRATES_IO_URL), crate_name);
    let client = reqwest::Client::new();
    let response = client.get(&url)
        .header("User-Agent", "CForge/0.1.0")
        .send().await?
        .text().await?;
    let json: Value = serde_json::from_str(&response)?;
    let modified_version = version.replace('"', "");
    version = &modified_version;

    if let Some(versions) = json["versions"].as_array() {
        if matches!(version, "latest" | "*") {
            if let Some(latest_version) = versions.iter().max_by_key(|v| v["num"].as_str().unwrap_or("")) {
                let latest = latest_version["num"].as_str();
                if latest.is_some() {
                    let is_deprecated = latest_version["yanked"].as_bool().unwrap_or(false);
                    let version = latest.unwrap().to_string();

                    if is_deprecated {
                        println!("💀 Yanked dependency: {} {} {}", crate_name.red(), "->".yellow(), version.black());
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
                        println!("💀 Yanked dependency: {} {} {}", crate_name.red(), "->".yellow(), v_num.black());
                    }

                    return Ok((true, v_num.to_string()));
                }
            }
        }
    }
    
    Ok((false, version.to_string()))
}
