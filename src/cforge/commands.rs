//! `cforge init` and `cforge install` — project scaffolding and a
//! cargo-free way to add dependencies straight into `properties.kson`.
//!
//! Neither command touches the transpile pipeline. `init` writes a fresh
//! project skeleton; `install` resolves a crate version off crates.io
//! (reusing [`check_version_exists`]) and edits the `$dependencies` block
//! of `properties.kson` in place as text — the KSON parser is read-only
//! (it shells out to `lson`), so we never round-trip through it.

use colored::Colorize;
use std::collections::HashMap;
use std::{env, fs, path::Path};

use crate::cforge::fetch::check_version_exists;
use crate::cforge::pretty;

const PROPERTIES_FILE: &str = "properties.kson";
/// Lock file pinning every dependency to the exact resolved version, so
/// `cforge install` is reproducible (the `lson`/kson counterpart of Cargo.lock).
const LOCK_FILE: &str = "properties.lock";

/// `cforge init` — scaffold `properties.kson`, `main.crs` and `.gitignore`
/// in the current directory. Existing files are never overwritten.
pub fn init() {
    let project_name = env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "copper-app".to_string());

    // properties.kson — the heart of the project. Refuse to clobber it.
    if Path::new(PROPERTIES_FILE).exists() {
        println!(
            "⚠️  {} already exists — leaving it untouched.",
            PROPERTIES_FILE.yellow()
        );
    } else {
        let properties = format!(
            "name = \"{name}\"\nversion = \"0.1.0\"\nedition = 2021\n\n$dependencies\n",
            name = project_name
        );
        if let Err(e) = fs::write(PROPERTIES_FILE, properties) {
            eprintln!("❌ Failed to write {}: {}", PROPERTIES_FILE, e);
            std::process::exit(1);
        }
        println!("✅ Created {}", PROPERTIES_FILE.green());
    }

    // main.crs — entry point. Top-level statements are wrapped into
    // `fn main()` by the parser, so a bare `println!` is the whole program.
    if Path::new("main.crs").exists() {
        println!(
            "⚠️  {} already exists — leaving it untouched.",
            "main.crs".yellow()
        );
    } else {
        let main_src = "// Welcome to Copper! Run this with: cforge run main.crs\nprintln!(\"Hello, Copper!\")\n";
        if let Err(e) = fs::write("main.crs", main_src) {
            eprintln!("❌ Failed to write main.crs: {}", e);
            std::process::exit(1);
        }
        println!("✅ Created {}", "main.crs".green());
    }

    // .gitignore — keep the transpiler output and Rust build dir out of VCS.
    ensure_gitignore();

    println!(
        "\n🎉 Project {} ready. Try {} to build and run it.",
        project_name.bold(),
        "cforge run".cyan()
    );
}

/// Append `dist/` and `target/` to `.gitignore`, creating it if needed and
/// skipping entries that are already present.
fn ensure_gitignore() {
    let wanted = ["/dist", "/target"];
    let existing = fs::read_to_string(".gitignore").unwrap_or_default();
    let already: Vec<&str> = existing.lines().map(str::trim).collect();

    let mut to_add = String::new();
    for entry in wanted {
        if !already
            .iter()
            .any(|l| *l == entry || *l == entry.trim_start_matches('/'))
        {
            to_add.push_str(entry);
            to_add.push('\n');
        }
    }

    if to_add.is_empty() {
        return;
    }

    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&to_add);

    if let Err(e) = fs::write(".gitignore", content) {
        eprintln!("⚠️  Could not update .gitignore: {}", e);
    } else {
        println!("✅ Updated {}", ".gitignore".green());
    }
}

/// `cforge install <name>[@version]` — resolve a version off crates.io and
/// record it in the `$dependencies` block of `properties.kson`.
pub async fn install(spec: &str) {
    if !Path::new(PROPERTIES_FILE).exists() {
        eprintln!(
            "❌ No {} found. Run {} first.",
            PROPERTIES_FILE.yellow(),
            "cforge init".cyan()
        );
        std::process::exit(1);
    }

    // Split `name@version`; a bare name resolves to the latest version.
    let (name, requested) = match spec.split_once('@') {
        Some((n, v)) if !v.is_empty() => (n, v),
        _ => (spec, "latest"),
    };

    if name.is_empty() {
        eprintln!("❌ Usage: cforge install <name>[@version]");
        std::process::exit(1);
    }

    println!("🔎 Resolving {}...", name.green());

    // For an explicit version we still verify it exists (but fall back to
    // the requested string offline / on miss). For `latest` we pull the
    // newest published version.
    let query = if requested == "latest" {
        "*"
    } else {
        requested
    };
    let (valid, resolved) = check_version_exists(name, query, None)
        .await
        .unwrap_or((false, requested.to_string()));

    let version = if valid {
        resolved
    } else if requested == "latest" {
        eprintln!(
            "❌ Could not resolve a version for {} (crate not found or offline).",
            name.red()
        );
        std::process::exit(1);
    } else {
        // Explicit version that we couldn't verify — trust the user.
        println!(
            "⚠️  Could not verify {}@{} against crates.io; using it as given.",
            name, requested
        );
        requested.to_string()
    };

    match upsert_dependency(name, &version) {
        Ok(updated) => {
            let verb = if updated { "Updated" } else { "Added" };
            pretty::ok(&format!(
                "{} {} {} {} in {}",
                verb,
                name.green(),
                "=>".yellow(),
                version,
                PROPERTIES_FILE
            ));
            // Keep the lock in sync with this single change.
            let mut locked = read_lock();
            locked.insert(name.to_string(), version.clone());
            let pairs: Vec<(String, String)> = locked.into_iter().collect();
            if let Err(e) = write_lock(&pairs) {
                pretty::warn(&format!("could not update {LOCK_FILE}: {e}"));
            }
        }
        Err(e) => {
            pretty::fail(&format!("Failed to update {}: {}", PROPERTIES_FILE, e));
            std::process::exit(1);
        }
    }
}

/// `cforge install` with no package — resolve and download **every** dependency
/// listed in `properties.kson`, then write a `properties.lock` pinning the exact
/// versions. Already-locked deps are reused (reproducible); only new or unlocked
/// ones hit the network. Finishes by running `cargo fetch` to pull the crates
/// into the local registry cache.
pub async fn install_all() {
    if !Path::new(PROPERTIES_FILE).exists() {
        pretty::fail(&format!(
            "No {} found. Run `cforge init` first.",
            PROPERTIES_FILE
        ));
        std::process::exit(1);
    }

    let deps = read_dependencies();
    if deps.is_empty() {
        pretty::ok(&format!("No dependencies declared in {PROPERTIES_FILE}."));
        return;
    }

    pretty::head(&format!(
        "Installing {} dependenc{} from {}",
        deps.len(),
        if deps.len() == 1 { "y" } else { "ies" },
        PROPERTIES_FILE
    ));

    let lock = read_lock();
    let mut resolved: Vec<(String, String)> = Vec::new();
    let (mut reused, mut fetched) = (0u32, 0u32);

    let pb = pretty::ProgressBar::new(deps.len(), "Resolving dependencies");
    let mut done = 0usize;
    for (name, req) in &deps {
        pb.set(done, &format!("resolving {name} {req}"));
        // Reuse the locked version when present and the request still names it
        // (exact pin) — that's the reproducible fast path, no network.
        if let Some(v) = lock.get(name) {
            if req == v || req == "*" || req.is_empty() {
                resolved.push((name.clone(), v.clone()));
                reused += 1;
                done += 1;
                pb.set(done, &format!("{name} {v} (locked)"));
                continue;
            }
        }

        let query = if req.is_empty() || req == "*" {
            "*"
        } else {
            req.as_str()
        };
        let (valid, ver) = check_version_exists(name, query, None)
            .await
            .unwrap_or((false, req.clone()));
        if valid {
            resolved.push((name.clone(), ver.clone()));
            done += 1;
            pb.set(done, &format!("{name} => {ver}"));
        } else {
            resolved.push((name.clone(), req.clone()));
            done += 1;
            pb.set(done, &format!("{name} (unresolved)"));
        }
        fetched += 1;
    }
    pb.finish(&format!(
        "Resolved {} dependencies ({} reused, {} fetched)",
        deps.len(),
        reused,
        fetched
    ));

    match write_lock(&resolved) {
        Ok(_) => pretty::ok(&format!(
            "Locked {} dependencies → {} ({} reused, {} resolved)",
            resolved.len(),
            LOCK_FILE,
            reused,
            fetched
        )),
        Err(e) => pretty::warn(&format!("could not write {LOCK_FILE}: {e}")),
    }

    // Actually download the crates into the cargo registry cache.
    cargo_fetch_locked(&resolved).await;
}

/// Generate a Cargo manifest from the resolved set and run `cargo fetch` so the
/// crates are downloaded ahead of the first build. Best-effort: a failure (e.g.
/// offline) is a warning, not fatal — the lock is already written.
async fn cargo_fetch_locked(_resolved: &[(String, String)]) {
    use std::process::Command;

    // Build ./dist/rust/Cargo.toml from properties.kson + a stub entry so the
    // manifest is valid for `cargo fetch` (which reads deps but doesn't build).
    crate::cforge::generate_toml(Vec::new()).await;
    let src_dir = Path::new("./dist/rust/src");
    let _ = fs::create_dir_all(src_dir);
    let main_rs = src_dir.join("main.rs");
    if !main_rs.exists() {
        let _ = fs::write(&main_rs, "fn main() {}\n");
    }

    let sp = pretty::Spinner::start("downloading crates (cargo fetch)");
    let status = Command::new("cargo")
        .arg("fetch")
        .current_dir("./dist/rust")
        .output();
    match status {
        Ok(o) if o.status.success() => sp.done("Crates downloaded into the cargo cache."),
        Ok(o) => {
            sp.fail("cargo fetch failed");
            let err = String::from_utf8_lossy(&o.stderr);
            for line in err.lines().take(4) {
                pretty::step(line);
            }
        }
        Err(e) => sp.fail(&format!("could not run cargo fetch: {e}")),
    }
}

/// Parse the `name = "value"` entries under a top-level `$section` in a KSON
/// file's text. Nested sub-sections and non-string values are skipped.
fn read_kson_section(text: &str, section: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // A top-level (non-indented) token starts/ends a section.
        if !line.starts_with(char::is_whitespace) {
            in_section = trimmed == section;
            continue;
        }
        if !in_section {
            continue;
        }
        // `name = "value"` (string values only — crate versions).
        if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim();
            let val = v.trim().trim_matches('"');
            if !key.is_empty() && !key.starts_with('$') && !val.is_empty() {
                out.push((key.to_string(), val.to_string()));
            }
        }
    }
    out
}

/// Dependencies declared under `$dependencies` in properties.kson.
fn read_dependencies() -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(PROPERTIES_FILE) else {
        return Vec::new();
    };
    read_kson_section(&text, "$dependencies")
}

/// The pinned versions from properties.lock, keyed by name.
fn read_lock() -> HashMap<String, String> {
    let Ok(text) = fs::read_to_string(LOCK_FILE) else {
        return HashMap::new();
    };
    read_kson_section(&text, "$locked").into_iter().collect()
}

/// Write properties.lock from the resolved (name, version) pairs, sorted for a
/// stable diff.
fn write_lock(pairs: &[(String, String)]) -> std::io::Result<()> {
    let mut sorted: Vec<(String, String)> = pairs.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::new();
    out.push_str("# Auto-generated by cforge — pinned dependency versions.\n");
    out.push_str("# Do not edit by hand; run `cforge install` to regenerate.\n\n");
    out.push_str("$locked\n");
    for (name, version) in &sorted {
        out.push_str(&format!("    {} = \"{}\"\n", name, version));
    }
    fs::write(LOCK_FILE, out)
}

/// Insert or update `name = "version"` inside the `$dependencies` section of
/// `properties.kson`, creating the section if it is missing. Returns `true`
/// when an existing entry was replaced.
fn upsert_dependency(name: &str, version: &str) -> std::io::Result<bool> {
    let text = fs::read_to_string(PROPERTIES_FILE)?;
    let dep_line = format!("    {} = \"{}\"", name, version);
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();

    // Locate the `$dependencies` section header.
    let header_idx = lines.iter().position(|l| l.trim() == "$dependencies");

    let Some(header_idx) = header_idx else {
        // No section yet — append one at the end of the file.
        let mut out = text.trim_end().to_string();
        out.push_str("\n\n$dependencies\n");
        out.push_str(&dep_line);
        out.push('\n');
        fs::write(PROPERTIES_FILE, out)?;
        return Ok(false);
    };

    // The block runs until the next top-level key/section (a non-empty line
    // that doesn't start with whitespace) or EOF.
    let mut block_end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(header_idx + 1) {
        if !line.trim().is_empty() && !line.starts_with(char::is_whitespace) {
            block_end = i;
            break;
        }
    }

    // Already present? Replace its version in place.
    let key_prefix = format!("{} ", name);
    for line in lines.iter_mut().take(block_end).skip(header_idx + 1) {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&key_prefix) || trimmed.starts_with(&format!("{}=", name)) {
            *line = dep_line.clone();
            fs::write(PROPERTIES_FILE, join_lines(&lines, &text))?;
            return Ok(true);
        }
    }

    // Not present — insert at the end of the block (after the last
    // non-empty dependency line, so we don't strand it past blank lines).
    let mut insert_at = header_idx + 1;
    for (i, line) in lines
        .iter()
        .enumerate()
        .take(block_end)
        .skip(header_idx + 1)
    {
        if !line.trim().is_empty() {
            insert_at = i + 1;
        }
    }
    lines.insert(insert_at, dep_line);
    fs::write(PROPERTIES_FILE, join_lines(&lines, &text))?;
    Ok(false)
}

/// Re-join lines, preserving the original file's trailing-newline style.
fn join_lines(lines: &[String], original: &str) -> String {
    let mut out = lines.join("\n");
    if original.ends_with('\n') {
        out.push('\n');
    }
    out
}
