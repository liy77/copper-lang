// Backend logic for the Copper installer — no Tauri dependency.
// Imported by installer.mui via `import { ... } from "./backend.rs"`.

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct QuickInfo {
    pub detected_source: String,
    pub default_local_dir: String,
    pub default_global_dir: String,
}

pub struct CheckInfo {
    pub has_cargo: bool,
    pub cargo_version: String,
    pub is_admin: bool,
}

/// Fast probe: detects source tree location + default install dirs.
/// Returns immediately; safe to call on the UI thread.
pub fn prereqs_quick() -> QuickInfo {
    let detected_source = detect_copper_source()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let (default_local_dir, default_global_dir) = default_install_dirs();

    QuickInfo { detected_source, default_local_dir, default_global_dir }
}

/// Slower probe (~100–500 ms): checks `cargo` presence + admin rights.
/// Runs both checks in parallel threads.
pub fn prereqs_check() -> CheckInfo {
    use std::thread;

    let cargo_handle = thread::spawn(|| {
        Command::new("cargo")
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    });

    let admin_handle = thread::spawn(|| is_elevated());

    let cargo_ver = cargo_handle.join().ok().flatten();
    let is_admin  = admin_handle.join().unwrap_or(false);

    CheckInfo {
        has_cargo:     cargo_ver.is_some(),
        cargo_version: cargo_ver.unwrap_or_else(|| "not found".into()),
        is_admin,
    }
}

/// Full install pipeline.  `log` is called with (level, text) pairs where
/// level ∈ { "step" | "ok" | "info" | "warn" | "error" }.
pub fn install_copper(
    source_dir: &str,
    install_dir: &str,
    global: bool,
    log: impl Fn(&str, &str),
) -> Result<(), String> {
    let src  = Path::new(source_dir);
    let dest = Path::new(install_dir);

    // ── Validar fonte ─────────────────────────────────────────────────────
    if !src.join("Cargo.toml").exists() {
        return Err(format!("No Cargo.toml found in {source_dir}"));
    }
    log("step", &format!("Source: {source_dir}"));
    log("step", &format!("Target: {install_dir}"));

    // ── Verificar cargo ───────────────────────────────────────────────────
    let ver = Command::new("cargo")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .ok_or_else(|| "cargo not found — install Rust first".to_owned())?;
    log("ok", &format!("Cargo: {ver}"));

    // ── Criar diretórios ──────────────────────────────────────────────────
    let bin_dir = dest.join("bin");
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("Cannot create {}: {e}", bin_dir.display()))?;
    log("ok", "Created install directories");

    // ── Build ─────────────────────────────────────────────────────────────
    log("step", "Building cforge in release mode (this can take a while)…");
    let exe_name = if cfg!(windows) { "cforge.exe" } else { "cforge" };
    let _ = std::fs::remove_file(src.join("target/release").join(exe_name));

    let build = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(src)
        .output()
        .map_err(|e| format!("Failed to run cargo build: {e}"))?;

    if !build.status.success() {
        let stderr = String::from_utf8_lossy(&build.stderr);
        return Err(format!("Build failed:\n{stderr}"));
    }
    log("ok", "Build successful");

    // ── Copiar arquivos ───────────────────────────────────────────────────
    let built_exe = src.join("target/release").join(exe_name);
    std::fs::copy(&built_exe, bin_dir.join(exe_name))
        .map_err(|e| format!("Failed to copy {exe_name}: {e}"))?;
    log("ok", &format!("Installed {exe_name}"));

    std::fs::copy(src.join("Cargo.toml"), dest.join("Cargo.toml"))
        .map_err(|e| format!("Failed to copy Cargo.toml: {e}"))?;
    log("ok", "Copied Cargo.toml");

    for dir in ["std", "lson"] {
        copy_dir_all(src.join(dir), dest.join(dir))
            .map_err(|e| format!("Failed to copy {dir}/: {e}"))?;
        log("ok", &format!("Copied {dir}/"));
    }

    for file in ["uninstall.py", "uninstall.bat"] {
        let p = src.join("scripts").join(file);
        if p.exists() {
            std::fs::copy(&p, dest.join(file))
                .map_err(|e| format!("Failed to copy {file}: {e}"))?;
        }
    }
    log("ok", "Installed uninstaller");

    // ── PATH / Registro (Windows) ─────────────────────────────────────────
    #[cfg(windows)]
    setup_windows_path(dest, global, &log)?;

    log("step", "Installation complete!");
    log("info", "Restart your terminal so the new PATH/COPPER_PATH take effect.");
    Ok(())
}

/// Directory autocomplete: up to 8 existing subdirectories whose full path
/// continues `partial`. If `partial` is itself a directory (or ends with a
/// separator) its children are listed; otherwise its parent is scanned for
/// names starting with the typed leaf (case-insensitive).
pub fn list_dirs(partial: &str) -> Vec<String> {
    let partial = partial.trim();
    if partial.is_empty() {
        return Vec::new();
    }
    let p = Path::new(partial);
    let (dir, prefix) = if partial.ends_with('\\') || partial.ends_with('/') || p.is_dir() {
        (p.to_path_buf(), String::new())
    } else {
        (
            p.parent().map(Path::to_path_buf).unwrap_or_default(),
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
        )
    };
    let mut out: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                if prefix.is_empty() || name.to_lowercase().starts_with(&prefix) {
                    out.push(e.path().to_string_lossy().into_owned());
                }
            }
        }
    }
    out.sort();
    out.truncate(8);
    out
}

// ── Helpers internos ───────────────────────────────────────────────────────

fn detect_copper_source() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    for _ in 0..6 {
        if dir.join("Cargo.toml").exists() && dir.join("src/cforge").exists() {
            return Some(dir);
        }
        dir = dir.parent()?.to_owned();
    }
    None
}

fn default_install_dirs() -> (String, String) {
    #[cfg(windows)]
    {
        let local = std::env::var("USERPROFILE")
            .map(|h| format!("{h}\\.copper"))
            .unwrap_or_else(|_| "C:\\copper".into());
        (local, "C:\\Program Files\\Copper".into())
    }
    #[cfg(not(windows))]
    {
        let local = std::env::var("HOME")
            .map(|h| format!("{h}/.copper"))
            .unwrap_or_else(|_| "/opt/copper".into());
        (local, "/usr/local/lib/copper".into())
    }
}

fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "net session"])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u32>().ok())
            .map(|uid| uid == 0)
            .unwrap_or(false)
    }
}

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    let (src, dst) = (src.as_ref(), dst.as_ref());
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn setup_windows_path(dest: &Path, global: bool, log: &impl Fn(&str, &str)) -> Result<(), String> {
    // The per-user env vars live under HKCU\Environment; the system-wide ones
    // under HKLM\...\Session Manager\Environment (NOT HKLM\Environment, which is
    // not the system PATH — writing there silently does nothing useful).
    let env_key = if global {
        r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment".to_string()
    } else {
        r"HKCU\Environment".to_string()
    };
    let dest_str = dest.to_string_lossy();

    // COPPER_PATH — a plain absolute path, so REG_SZ (matches scripts/install.py).
    Command::new("reg")
        .args([
            "add", &env_key,
            "/v", "COPPER_PATH",
            "/t", "REG_SZ",
            "/d", &dest_str,
            "/f",
        ])
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| format!("reg add COPPER_PATH failed: {e}"))?;
    log("ok", &format!("COPPER_PATH = {dest_str}"));

    // PATH — read the current value robustly, then append our marker. We NEVER
    // overwrite a PATH we couldn't read (that would wipe the user's PATH), so a
    // read error aborts the PATH edit instead of guessing.
    let marker = r"%COPPER_PATH%\bin";
    let current = reg_query_path(&env_key)?.unwrap_or_default();
    // Keep every non-empty entry except the marker itself, then append exactly
    // ONE marker. This both adds us and *cleans up* the duplicate markers /
    // empty segments a buggy older installer may have piled on.
    let kept: Vec<&str> = current
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case(marker))
        .collect();
    let marker_count = current
        .split(';')
        .filter(|p| p.trim().eq_ignore_ascii_case(marker))
        .count();
    let new_path = if kept.is_empty() {
        marker.to_owned()
    } else {
        format!("{};{}", kept.join(";"), marker)
    };
    // Only write when something actually changes (a single trailing marker and
    // no empties already => nothing to do).
    let already_clean = marker_count == 1
        && current.trim_end_matches(';').ends_with(marker)
        && !current.split(';').any(|p| p.trim().is_empty());
    if already_clean {
        log("info", &format!("{marker} already in PATH"));
    } else {
        Command::new("reg")
            .args([
                "add", &env_key,
                "/v", "Path",
                "/t", "REG_EXPAND_SZ",
                "/d", &new_path,
                "/f",
            ])
            .creation_flags(0x08000000)
            .output()
            .map_err(|e| format!("reg add Path failed: {e}"))?;
        if marker_count > 1 {
            log("ok", &format!("Added {marker} to PATH (removed {} duplicate(s))", marker_count - 1));
        } else {
            log("ok", &format!("Added {marker} to PATH"));
        }
    }

    // Tell already-running shells the environment changed (best effort), so a
    // freshly-opened terminal in the SAME explorer session sees it without a
    // full re-login.
    broadcast_env_change();
    Ok(())
}

/// Read the current `Path` value from a registry environment key via `reg query`.
/// Returns `Ok(None)` when the value simply doesn't exist yet, `Ok(Some(value))`
/// with the bare value (type token stripped), or `Err` if the value exists but
/// couldn't be parsed — the caller must then NOT overwrite PATH.
#[cfg(windows)]
fn reg_query_path(key: &str) -> Result<Option<String>, String> {
    let out = Command::new("reg")
        .args(["query", key, "/v", "Path"])
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| format!("reg query Path failed: {e}"))?;
    // Non-zero exit = the value is absent (fresh user with no PATH). Safe to
    // treat as empty; we'll create it.
    if !out.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let l = line.trim_start();
        if !l.starts_with("Path") {
            continue;
        }
        // `reg query` prints:  "Path    REG_EXPAND_SZ    <value>" (or REG_SZ).
        // Take everything AFTER the type token — robust against the value
        // itself containing runs of spaces.
        for ty in ["REG_EXPAND_SZ", "REG_SZ"] {
            if let Some(i) = l.find(ty) {
                return Ok(Some(l[i + ty.len()..].trim().to_string()));
            }
        }
    }
    // Reported success but no parseable Path line — refuse to guess.
    Err("could not parse the existing PATH from the registry".into())
}

/// Broadcast `WM_SETTINGCHANGE("Environment")` so running processes refresh
/// their environment. Raw FFI (no crate) — best effort, failures ignored.
#[cfg(windows)]
fn broadcast_env_change() {
    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: *const u16,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }
    const HWND_BROADCAST: isize = 0xFFFF;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    let env: Vec<u16> = "Environment".encode_utf16().chain(std::iter::once(0)).collect();
    let mut result: usize = 0;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            env.as_ptr(),
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}

#[cfg(windows)]
trait CommandExtWindows {
    fn creation_flags(&mut self, flags: u32) -> &mut Self;
}

#[cfg(windows)]
impl CommandExtWindows for Command {
    fn creation_flags(&mut self, flags: u32) -> &mut Self {
        use std::os::windows::process::CommandExt;
        CommandExt::creation_flags(self, flags);
        self
    }
}
