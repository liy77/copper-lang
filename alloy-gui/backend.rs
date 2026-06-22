//! Backend logic for the Alloy GUI — std + the portable `alloy-vm` lib only
//! (no mocida). Imported by `alloy.mui` (`import { run_source, ... } from
//! "./backend.rs"`) and called directly by the native host (`src/main.rs`).
//!
//! Everything here is platform-agnostic and unit-testable without a window;
//! the host wires the results into the view's signals.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use alloy_vm::interp::Interpreter;
use copper_syntax::program::parse_program;

/// The result of running a Copper source through the interpreter.
pub struct RunResult {
    /// Captured stdout (everything `println`/`print` emitted).
    pub output: String,
    /// `Some(msg)` if parsing or evaluation failed; `None` on success.
    pub error: Option<String>,
}

/// Parse + interpret `src`, capturing its output instead of writing to stdout.
/// Never panics: syntax and runtime errors come back in `error`.
pub fn run_source(src: &str) -> RunResult {
    let prog = parse_program(src);
    if !prog.errors.is_empty() {
        let msg = prog
            .errors
            .iter()
            .map(|e| format!("sintaxe @ {}..{}: {}", e.span.start, e.span.end, e.message))
            .collect::<Vec<_>>()
            .join("\n");
        return RunResult {
            output: String::new(),
            error: Some(msg),
        };
    }
    let buf: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let result = Interpreter::with_output(Rc::clone(&buf)).run_program(&prog);
    let output = buf.borrow().clone();
    let error = result.err().map(|e| {
        format!("runtime @ {}..{}: {}", e.span.start, e.span.end, e.message)
    });
    RunResult { output, error }
}

/// Read a `.crs` file from disk for the file-runner. Returns the source or an
/// error message suitable for display.
pub fn read_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("não consegui ler {path}: {e}"))
}

/// Directory autocomplete: list child entries of the directory part of
/// `prefix` whose names start with the (partial) final component. Used to
/// suggest paths as the user types in the file-runner field.
pub fn list_dirs(prefix: &str) -> Vec<String> {
    let p = Path::new(prefix);
    let (dir, partial): (PathBuf, String) = if prefix.ends_with(std::path::MAIN_SEPARATOR) {
        (p.to_path_buf(), String::new())
    } else {
        (
            p.parent().map(Path::to_path_buf).unwrap_or_default(),
            p.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )
    };
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        &dir
    }) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if partial.is_empty() || name.starts_with(&partial) {
                out.push(entry.path().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out.truncate(20);
    out
}

// ===========================================================================
// Updater
// ===========================================================================
//
// Source of releases. Fill REPO_SLUG with the GitHub `owner/repo` that
// publishes Alloy release binaries (assets named `alloy-<target>(.exe)`).
// Downloads shell out to `curl` (present on Win10+/macOS/Linux) so the crate
// stays dependency-light, mirroring the installer's std-only backend.

const REPO_SLUG: &str = "liy77/copper-lang"; // TODO: confirme o slug de releases
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    /// True when `latest` is newer than `current`.
    pub available: bool,
    /// Download URL of the asset for this platform (empty if none matched).
    pub asset_url: String,
}

/// The version this binary was built as.
pub fn current_version() -> String {
    CURRENT_VERSION.to_string()
}

/// Query the latest GitHub release and compare to the running version.
/// Returns an `UpdateInfo`; on any network/parse failure, `available` is false
/// and `latest` carries the error so the UI can show it.
pub fn check_update() -> UpdateInfo {
    let api = format!("https://api.github.com/repos/{REPO_SLUG}/releases/latest");
    let body = match curl_get(&api) {
        Ok(b) => b,
        Err(e) => {
            return UpdateInfo {
                current: CURRENT_VERSION.to_string(),
                latest: format!("erro: {e}"),
                available: false,
                asset_url: String::new(),
            }
        }
    };
    let latest = json_str_field(&body, "tag_name").unwrap_or_default();
    let asset_url = find_asset_url(&body);
    let available = is_newer(&latest, CURRENT_VERSION);
    UpdateInfo {
        current: CURRENT_VERSION.to_string(),
        latest,
        available,
        asset_url,
    }
}

/// Download `asset_url` and replace the running executable. Downloads to a temp
/// file beside the current exe, then swaps it in. On Windows the running `.exe`
/// can't be deleted, so we rename the old one aside first.
pub fn apply_update(asset_url: &str) -> Result<(), String> {
    if asset_url.is_empty() {
        return Err("nenhum asset de release para esta plataforma".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let tmp = exe.with_extension("new");
    curl_download(asset_url, &tmp).map_err(|e| format!("download falhou: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
        std::fs::rename(&tmp, &exe).map_err(|e| format!("troca falhou: {e}"))?;
    }
    #[cfg(windows)]
    {
        let old = exe.with_extension("old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old).map_err(|e| format!("não pude mover o exe atual: {e}"))?;
        std::fs::rename(&tmp, &exe).map_err(|e| format!("troca falhou: {e}"))?;
    }
    Ok(())
}

// --- helpers (std + curl) -------------------------------------------------

fn curl_get(url: &str) -> Result<String, String> {
    let out = Command::new("curl")
        .args(["-sSL", "-H", "User-Agent: alloy-updater", url])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn curl_download(url: &str, dest: &Path) -> Result<(), String> {
    let out = Command::new("curl")
        .args(["-sSL", "-H", "User-Agent: alloy-updater", "-o"])
        .arg(dest)
        .arg(url)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(())
}

/// Pick the release asset whose name matches this platform's target triple
/// family. Minimal: matches on OS substring (`windows`/`darwin`|`apple`/`linux`).
fn find_asset_url(release_json: &str) -> String {
    let os_key = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "apple"
    } else {
        "linux"
    };
    for url in json_all_field(release_json, "browser_download_url") {
        if url.contains(os_key) || (os_key == "apple" && url.contains("darwin")) {
            return url;
        }
    }
    String::new()
}

/// Naive `"field": "value"` extractor (first match). Good enough for the few
/// fields we read from the GitHub release JSON without pulling a JSON crate.
fn json_str_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let i = json.find(&needle)?;
    let rest = &json[i + needle.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

/// All string values for a repeated field (e.g. each asset's download URL).
fn json_all_field(json: &str, field: &str) -> Vec<String> {
    let needle = format!("\"{field}\"");
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = json[from..].find(&needle) {
        let i = from + rel + needle.len();
        if let Some(colon) = json[i..].find(':') {
            let after = json[i + colon + 1..].trim_start();
            if let Some(after) = after.strip_prefix('"') {
                if let Some(end) = after.find('"') {
                    out.push(after[..end].to_string());
                }
            }
        }
        from = i;
    }
    out
}

/// Compare two `vX.Y.Z[-pre]` tags numerically; returns true if `latest` is
/// strictly newer than `current`. Pre-release suffixes are ignored in the
/// numeric compare (a tie on numbers → not newer).
fn is_newer(latest: &str, current: &str) -> bool {
    fn nums(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split(|c: char| c == '.' || c == '-' || c == '+')
            .take_while(|s| s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect()
    }
    let (a, b) = (nums(latest), nums(current));
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_source_captures_and_reports() {
        let r = run_source("func main() {\n  println(\"oi\")\n}");
        assert_eq!(r.output, "oi\n");
        assert!(r.error.is_none());

        let bad = run_source("func main() { 1 / 0 }");
        assert!(bad.error.is_some(), "div-by-zero deve reportar erro");
    }

    #[test]
    fn version_compare() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn json_field_extraction() {
        let j = r#"{"tag_name": "v1.2.3", "assets":[{"browser_download_url":"http://x/alloy-linux"}]}"#;
        assert_eq!(json_str_field(j, "tag_name").as_deref(), Some("v1.2.3"));
        assert_eq!(json_all_field(j, "browser_download_url").len(), 1);
    }
}
