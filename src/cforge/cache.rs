//! Multi-layer cache system for CForge:
//!   1. Global package registry cache (`~/.copper/cache/registry.json`) — caches
//!      resolved crates.io versions so `cforge install` / `generate_toml` never
//!      re-queries the network for recently resolved packages.
//!   2. Incremental `.crs` transpilation cache (`<dist>/.cforge_cache.json`) —
//!      hashes `.crs` source + compiler version and skips tokenization, parsing,
//!      and `rustfmt` when a file hasn't changed, while `write_if_changed`
//!      preserves `.rs` modification timestamps so Cargo's incremental build
//!      cache stays warm.

use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

use crate::cforge::pretty;

/// TTL for cached crates.io version lookups (24 hours).
const REGISTRY_TTL_SECS: u64 = 86_400;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranspileEntry {
    pub hash: String,
    pub output_path: String,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranspileCache {
    pub compiler_version: String,
    pub entries: HashMap<String, TranspileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub version: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistryCache {
    pub entries: HashMap<String, RegistryEntry>,
}

/// Fast 64-bit FNV-1a content hash formatted as 16-char hex.
pub fn hash_str(s: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in s.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Compute a composite hash of compiler version + source content.
pub fn hash_source(source: &str, compiler_tag: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in compiler_tag.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= 0xff;
    hash = hash.wrapping_mul(0x100000001b3);
    for &b in source.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Write `content` to `path` only if the file does not exist or its current
/// contents differ. Preserves file mtime when unchanged so Cargo does not
/// recompile untouched modules.
pub fn write_if_changed<P: AsRef<Path>>(path: P, content: &str) -> std::io::Result<bool> {
    let path = path.as_ref();
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == content {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(true)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Global cache directory (`~/.copper/cache`).
pub fn global_cache_dir() -> PathBuf {
    let home = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".copper").join("cache")
}

pub fn registry_cache_path() -> PathBuf {
    global_cache_dir().join("registry.json")
}

pub fn transpile_cache_path(output_dir: &str) -> PathBuf {
    Path::new(output_dir).join(".cforge_cache.json")
}

impl TranspileCache {
    pub fn load(output_dir: &str, compiler_version: &str) -> Self {
        let path = transpile_cache_path(output_dir);
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(cache) = serde_json::from_str::<TranspileCache>(&text) {
                if cache.compiler_version == compiler_version {
                    return cache;
                }
            }
        }
        Self {
            compiler_version: compiler_version.to_string(),
            entries: HashMap::new(),
        }
    }

    pub fn save(&self, output_dir: &str) {
        let path = transpile_cache_path(output_dir);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = write_if_changed(path, &json);
        }
    }

    pub fn get_valid(&self, file_key: &str, source_hash: &str) -> Option<&TranspileEntry> {
        let entry = self.entries.get(file_key)?;
        if entry.hash == source_hash && Path::new(&entry.output_path).exists() {
            Some(entry)
        } else {
            None
        }
    }

    pub fn insert(&mut self, file_key: String, entry: TranspileEntry) {
        self.entries.insert(file_key, entry);
    }
}

impl RegistryCache {
    pub fn load() -> Self {
        let path = registry_cache_path();
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(cache) = serde_json::from_str::<RegistryCache>(&text) {
                return cache;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = registry_cache_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = write_if_changed(path, &json);
        }
    }

    pub fn get(&self, crate_name: &str, req: &str) -> Option<String> {
        let key = format!("{crate_name}@{req}");
        let entry = self.entries.get(&key)?;
        if now_secs().saturating_sub(entry.timestamp) <= REGISTRY_TTL_SECS {
            Some(entry.version.clone())
        } else {
            None
        }
    }

    pub fn put(&mut self, crate_name: &str, req: &str, version: &str) {
        let key = format!("{crate_name}@{req}");
        self.entries.insert(
            key,
            RegistryEntry {
                version: version.to_string(),
                timestamp: now_secs(),
            },
        );
    }
}

/// CLI handler for `cforge cache [status|clean]`.
pub fn handle_cache_command(action: Option<&str>, output_dir: Option<&str>) {
    let out = output_dir.unwrap_or("./dist");
    match action.unwrap_or("status") {
        "clean" | "clear" => {
            let reg_path = registry_cache_path();
            let tr_path = transpile_cache_path(out);
            let mut removed = 0;
            if reg_path.exists() && fs::remove_file(&reg_path).is_ok() {
                removed += 1;
            }
            if tr_path.exists() && fs::remove_file(&tr_path).is_ok() {
                removed += 1;
            }
            pretty::ok(&format!(
                "Cleared CForge caches ({} cache file(s) removed)",
                removed
            ));
        }
        _ => {
            let reg = RegistryCache::load();
            let tr_path = transpile_cache_path(out);
            let tr_count = fs::read_to_string(&tr_path)
                .ok()
                .and_then(|s| serde_json::from_str::<TranspileCache>(&s).ok())
                .map(|c| c.entries.len())
                .unwrap_or(0);
            pretty::head("CForge Cache Status");
            println!(
                "  {} {} ({} cached packages)",
                "Registry cache:".bold(),
                registry_cache_path().display().to_string().cyan(),
                reg.entries.len().to_string().green()
            );
            println!(
                "  {} {} ({} cached modules)",
                "Transpile cache:".bold(),
                tr_path.display().to_string().cyan(),
                tr_count.to_string().green()
            );
        }
    }
}
