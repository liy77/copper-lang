pub mod cache;
pub mod check;
pub mod commands;
pub mod fetch;
pub mod kson;
pub mod module_transpile;
pub mod mui;
pub mod mui_fmt;
pub mod pretty;
pub mod properties;
pub mod vprint;

use colored::Colorize;
use once_cell::sync::Lazy;
use std::{fs, path, process::Command};

use crate::{parser, tokenizer::tokenizer::Tokenizer, vprint};

pub const VERSION: &str = env!("CFORGE_VERSION");

/// Run a `cargo` command behind cforge's live progress bar (`BuildProgress`)
/// instead of cargo's raw stdout/stderr:
/// - Queries `cargo metadata --offline` in parallel to learn the package count.
/// - Passes `--message-format=json(-diagnostic-rendered-ansi)` so cargo emits
///   structured artifact completion, build-script execution, and diagnostics.
/// - Displays a progress bar (`done/total`), active parallel crates (including
///   `(build.rs)`), lock/error notices, and elapsed time.
/// - Surfaces captured compiler diagnostics and stderr cleanly on failure.
pub(crate) fn cargo_with_spinner(
    mut cmd: Command,
    building_label: &str,
    done_label: &str,
) -> (bool, String) {
    use std::collections::HashSet;
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};

    let animated = pretty::is_tty();
    let has_msg_fmt = cmd
        .get_args()
        .any(|a| a.to_string_lossy().starts_with("--message-format"));
    if !has_msg_fmt {
        cmd.arg(if animated {
            "--message-format=json-diagnostic-rendered-ansi"
        } else {
            "--message-format=json"
        });
    }

    let work_dir = cmd
        .get_current_dir()
        .map(path::PathBuf::from)
        .unwrap_or_else(|| path::PathBuf::from("."));
    let target_triple =
        extract_target_arg(&cmd).unwrap_or_else(|| env!("CFORGE_HOST_TARGET").to_string());

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            pretty::fail(&format!("failed to run cargo: {e}"));
            return (false, String::new());
        }
    };

    let progress = pretty::BuildProgress::start(building_label);
    let state = progress.state_handle();

    // Resolve total non-dev package count in the background without blocking cargo.
    let meta_state = state.clone();
    let meta_thread = std::thread::spawn(move || {
        if let Some(total) = query_package_count(&work_dir, &target_triple) {
            if let Ok(mut s) = meta_state.lock() {
                s.total = Some(total);
            }
        }
    });

    let diagnostics = Arc::new(Mutex::new(String::new()));
    let stderr_buf = Arc::new(Mutex::new(String::new()));

    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let state = state.clone();
        let diagnostics = diagnostics.clone();
        readers.push(std::thread::spawn(move || {
            let mut done_pkgs: HashSet<String> = HashSet::new();
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(t) {
                    handle_cargo_json(&val, &state, &diagnostics, &mut done_pkgs, animated);
                } else if let Ok(mut d) = diagnostics.lock() {
                    d.push_str(&line);
                    d.push('\n');
                }
            }
        }));
    }

    if let Some(stderr) = child.stderr.take() {
        let state = state.clone();
        let stderr_buf = stderr_buf.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                handle_cargo_stderr_line(&line, &state, animated);
                if !is_cargo_progress_noise(&line) {
                    if let Ok(mut c) = stderr_buf.lock() {
                        c.push_str(&line);
                        c.push('\n');
                    }
                }
            }
        }));
    }

    let status = child.wait();
    for r in readers {
        let _ = r.join();
    }
    let _ = meta_thread.join();

    let diag_out = diagnostics.lock().map(|c| c.clone()).unwrap_or_default();
    let err_out = stderr_buf.lock().map(|c| c.clone()).unwrap_or_default();
    let combined = format!("{diag_out}{err_out}");
    let ok = status.map(|s| s.success()).unwrap_or(false);

    if ok {
        progress.done(done_label);
    } else {
        progress.fail("Build failed");
        if !diag_out.is_empty() {
            print!("{diag_out}");
        }
        if !err_out.is_empty() {
            print!("{err_out}");
        }
    }
    (ok, combined)
}

fn extract_target_arg(cmd: &Command) -> Option<String> {
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    for (i, arg) in args.iter().enumerate() {
        if let Some(t) = arg.strip_prefix("--target=") {
            if !t.is_empty() {
                return Some(t.to_string());
            }
        } else if arg == "--target" {
            if let Some(next) = args.get(i + 1) {
                if !next.is_empty() {
                    return Some(next.clone());
                }
            }
        }
    }
    None
}

fn query_package_count(work_dir: &path::Path, target_triple: &str) -> Option<usize> {
    let out = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--filter-platform")
        .arg(target_triple)
        .arg("--offline")
        .current_dir(work_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let val: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    count_non_dev_packages(&val)
}

fn count_non_dev_packages(metadata: &serde_json::Value) -> Option<usize> {
    use std::collections::{HashMap, HashSet};
    let resolve = metadata.get("resolve")?;
    let root = resolve.get("root")?.as_str()?;
    let nodes_arr = resolve.get("nodes")?.as_array()?;
    let mut nodes: HashMap<&str, &serde_json::Value> = HashMap::with_capacity(nodes_arr.len());
    for n in nodes_arr {
        if let Some(id) = n.get("id").and_then(|v| v.as_str()) {
            nodes.insert(id, n);
        }
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = vec![root];
    while let Some(cur) = stack.pop() {
        if !seen.insert(cur) {
            continue;
        }
        let Some(node) = nodes.get(cur) else {
            continue;
        };
        let Some(deps) = node.get("deps").and_then(|v| v.as_array()) else {
            continue;
        };
        for dep in deps {
            let Some(pkg) = dep.get("pkg").and_then(|v| v.as_str()) else {
                continue;
            };
            let kinds = dep.get("dep_kinds").and_then(|v| v.as_array());
            let needed = match kinds {
                None => true,
                Some(arr) if arr.is_empty() => true,
                Some(arr) => arr.iter().any(|k| {
                    matches!(k.get("kind").and_then(|v| v.as_str()), None | Some("build"))
                }),
            };
            if needed {
                stack.push(pkg);
            }
        }
    }
    Some(seen.len())
}

fn pkg_name_from_id(id: &str) -> &str {
    if let Some((before_hash, after_hash)) = id.split_once('#') {
        if let Some((name, _ver)) = after_hash.split_once('@') {
            return name;
        }
        if let Some(seg) = before_hash.rsplit(['/', '\\']).next() {
            if !seg.is_empty() {
                return seg;
            }
        }
    }
    id.split_whitespace().next().unwrap_or(id)
}

fn crate_name_eq(a: &str, b: &str) -> bool {
    let a = a.strip_suffix(" (build.rs)").unwrap_or(a);
    let b = b.strip_suffix(" (build.rs)").unwrap_or(b);
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x == y || (matches!(x, b'-' | b'_') && matches!(y, b'-' | b'_')))
}

fn handle_cargo_json(
    val: &serde_json::Value,
    state: &std::sync::Mutex<pretty::BuildProgressState>,
    diagnostics: &std::sync::Mutex<String>,
    done_pkgs: &mut std::collections::HashSet<String>,
    animated: bool,
) {
    let Some(reason) = val.get("reason").and_then(|v| v.as_str()) else {
        return;
    };
    match reason {
        "compiler-artifact" => {
            let pkg_id = val.get("package_id").and_then(|v| v.as_str()).unwrap_or("");
            let fresh = val.get("fresh").and_then(|v| v.as_bool()).unwrap_or(false);
            let is_build_script = val
                .get("target")
                .and_then(|t| t.get("kind"))
                .and_then(|k| k.as_array())
                .map(|arr| arr.iter().any(|x| x.as_str() == Some("custom-build")))
                .unwrap_or(false);
            let pkg_name = pkg_name_from_id(pkg_id);
            let inserted =
                !is_build_script && !pkg_id.is_empty() && done_pkgs.insert(pkg_id.to_string());

            if let Ok(mut s) = state.lock() {
                if inserted {
                    s.done = done_pkgs.len();
                }
                if is_build_script && !fresh {
                    if let Some(slot) = s.active.iter_mut().find(|a| crate_name_eq(a, pkg_name)) {
                        *slot = format!("{pkg_name} (build.rs)");
                    } else if !pkg_name.is_empty() {
                        s.active.push(format!("{pkg_name} (build.rs)"));
                    }
                    if !animated && !pkg_name.is_empty() {
                        pretty::step(&format!("running {pkg_name} build.rs"));
                    }
                } else {
                    s.active.retain(|a| !crate_name_eq(a, pkg_name));
                }
            }
        }
        "build-script-executed" => {
            let pkg_id = val.get("package_id").and_then(|v| v.as_str()).unwrap_or("");
            let pkg_name = pkg_name_from_id(pkg_id);
            if let Ok(mut s) = state.lock() {
                // After build.rs completes, cargo compiles the crate itself.
                if let Some(slot) = s
                    .active
                    .iter_mut()
                    .find(|a| a.ends_with(" (build.rs)") && crate_name_eq(a, pkg_name))
                {
                    *slot = pkg_name.to_string();
                }
            }
        }
        "compiler-message" => {
            let Some(msg) = val.get("message") else {
                return;
            };
            let level = msg.get("level").and_then(|v| v.as_str()).unwrap_or("");
            if level == "error" {
                if let Ok(mut s) = state.lock() {
                    s.errors += 1;
                    if !animated && s.errors == 1 {
                        pretty::warn("compiler error detected; waiting for active jobs...");
                    }
                }
            }
            if let Some(rendered) = msg.get("rendered").and_then(|v| v.as_str()) {
                if let Ok(mut d) = diagnostics.lock() {
                    d.push_str(rendered);
                    if !rendered.ends_with('\n') {
                        d.push('\n');
                    }
                }
            }
        }
        _ => {}
    }
}

fn handle_cargo_stderr_line(
    line: &str,
    state: &std::sync::Mutex<pretty::BuildProgressState>,
    animated: bool,
) {
    let t = line.trim_start();
    if let Some(rest) = t
        .strip_prefix("Compiling ")
        .or_else(|| t.strip_prefix("Checking "))
    {
        if let Some(name) = rest.split_whitespace().next() {
            if let Ok(mut s) = state.lock() {
                s.started += 1;
                if !s.active.iter().any(|a| crate_name_eq(a, name)) {
                    s.active.push(name.to_string());
                }
                s.status.clear();
            }
            if !animated {
                pretty::step(&format!("compiling {name}"));
            }
        }
        return;
    }

    if t.starts_with("Blocking waiting for file lock") {
        if let Ok(mut s) = state.lock() {
            s.status = "waiting for cargo lock...".to_string();
        }
        if !animated {
            pretty::warn("waiting for cargo build directory lock...");
        }
    } else if t.starts_with("Updating ") {
        if let Ok(mut s) = state.lock() {
            s.status = "updating crates index".to_string();
        }
    } else if t.starts_with("Downloading ") || t.starts_with("Downloaded ") {
        if let Ok(mut s) = state.lock() {
            s.status = "downloading crates".to_string();
        }
    } else if t.starts_with("Locking ") {
        if let Ok(mut s) = state.lock() {
            s.status = "locking dependencies".to_string();
        }
    } else if t.starts_with("warning: build failed") {
        if let Ok(mut s) = state.lock() {
            s.build_failed = true;
        }
        if !animated {
            pretty::warn("build failed, waiting for other jobs...");
        }
    } else if t.starts_with("Finished ") {
        if let Ok(mut s) = state.lock() {
            s.active.clear();
            s.status = "finishing".to_string();
        }
    }
}

fn is_cargo_progress_noise(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("Compiling ")
        || t.starts_with("Checking ")
        || t.starts_with("Fresh ")
        || t.starts_with("Updating ")
        || t.starts_with("Downloading ")
        || t.starts_with("Downloaded ")
        || t.starts_with("Locking ")
        || t.starts_with("Finished ")
}

/// mocida build env for a spawned cargo build. The generated crate depends on
/// the mocida **source** wrapper (a path dep), so its headers + lib must come
/// from that same source tree — otherwise bindgen binds a mismatched (e.g.
/// stale *installed*) copy and the wrapper fails with "cannot find function in
/// sys". So when the C source is found next to mocida-rs we **override**
/// `MOCIDA_INCLUDE_DIR` / `MOCIDA_LIB_DIR` (even if the user set them to an
/// install); only when the source isn't present do we leave the user's env.
///
/// Cross-platform: the lib dir is discovered by scanning `mocida/build/` for a
/// library file (`.dll`/`.lib`/`.so`/`.dylib`/`.a`), not a fixed `win32` path,
/// and libclang uses per-OS install dirs (or bindgen's own discovery on Linux).
pub(crate) fn apply_mocida_env(cmd: &mut Command, mocida_rs: &path::Path) {
    // mocida-rs and the C lib `mocida/` sit side by side under the repo root.
    if let Some(root) = mocida_rs.parent() {
        let mocida_c = root.join("mocida");
        // Prefer a staged SDK (`mocida/release/stage`): its `lib/` ships the
        // *dynamic* libmocida (+ SDL3) which self-resolves system frameworks,
        // so the final link is clean. The raw `build/` tree often holds only a
        // static `libmocida.a` that pulls in unresolved Cocoa/SDL symbols.
        let stage = mocida_c.join("release").join("stage");
        let staged_inc = stage.join("include");
        let staged_lib = stage.join("lib");
        let source_inc = mocida_c.join("src").join("headers");
        // Use the staged SDK ONLY when it's at least as fresh as the C source
        // headers. A stale staged SDK (older than the wrapper's source) is
        // missing functions the `mocida` wrapper references, so bindgen binds an
        // incomplete `sys` and the wrapper fails with "cannot find function in
        // sys". When the source headers are newer, bind against them + the
        // source `build/` lib instead.
        let staged_ok = staged_inc.join("uikit").is_dir()
            && dir_has_mocida_lib(&staged_lib)
            && !headers_newer(&source_inc, &staged_inc);
        let (inc, lib) = if staged_ok {
            (staged_inc, Some(staged_lib))
        } else {
            (source_inc, find_mocida_lib_dir(&mocida_c.join("build")))
        };
        if inc.is_dir() {
            cmd.env("MOCIDA_INCLUDE_DIR", inc);
        }
        if let Some(lib) = lib {
            // On macOS/Linux the dynamic libmocida is loaded via `@rpath`/soname
            // but cargo bakes no rpath into the binary, so it fails at startup
            // with "Library not loaded: @rpath/libmocida.dylib". Inject an rpath
            // pointing at the lib dir via RUSTFLAGS (applies to every crate in
            // the build, so mui-dev and the codegen binary both get it). Windows
            // stages the DLLs beside the exe instead, so it's not needed there.
            #[cfg(not(target_os = "windows"))]
            if let Some(lib_str) = lib.to_str() {
                let flag = format!("-C link-arg=-Wl,-rpath,{lib_str}");
                let combined = match std::env::var("RUSTFLAGS") {
                    Ok(existing) if !existing.is_empty() => format!("{existing} {flag}"),
                    _ => flag,
                };
                cmd.env("RUSTFLAGS", combined);
            }
            cmd.env("MOCIDA_LIB_DIR", lib);
        }
    }
    if std::env::var_os("LIBCLANG_PATH").is_none() {
        if let Some(p) = default_libclang_dir() {
            cmd.env("LIBCLANG_PATH", p);
        }
    }
}

/// True if the `uikit/` headers under `a` are newer than those under `b`,
/// compared via a stable representative header (`uikit/app.h`). Used to avoid
/// binding against a stale staged SDK when the C source headers have moved on.
fn headers_newer(a: &path::Path, b: &path::Path) -> bool {
    let mtime = |p: &path::Path| {
        fs::metadata(p.join("uikit").join("app.h"))
            .and_then(|m| m.modified())
            .ok()
    };
    match (mtime(a), mtime(b)) {
        (Some(ta), Some(tb)) => ta > tb,
        // Can't compare → don't override the staged choice.
        _ => false,
    }
}

/// True if `dir` holds a mocida library for any platform.
fn dir_has_mocida_lib(dir: &path::Path) -> bool {
    [
        "mocida.dll",
        "mocida.lib",
        "libmocida.so",
        "libmocida.dylib",
        "libmocida.a",
    ]
    .iter()
    .any(|f| dir.join(f).exists())
}

/// Find a built mocida lib dir under `build/` on any OS: checks `build/` itself,
/// then `build/<platform>/<profile>` (two levels), preferring the newest lib.
fn find_mocida_lib_dir(build_root: &path::Path) -> Option<path::PathBuf> {
    // Track (path, mtime, is_dylib) so we can prefer a .dylib build over a
    // .a (static) build, even if the .a is more recent. The static
    // `libmocida.a` pulls in unresolved SDL3 / AVFoundation / curl / WebKit /
    // mimalloc symbols at the host link step (those dylibs aren't
    // transitively linked when mocida is consumed as a `.a`); the
    // `libmocida.dylib` self-resolves system frameworks.
    //
    // Note: we ALWAYS descend into subdirs (no early-return on a root
    // .a). The mocida build drops a `libmocida.a` in the build root
    // (via the standalone `setup.py` / `c:static` target) and a
    // `libmocida.dylib` under `darwin/debug-shared/` (via
    // `cmake --build build`). If we returned early on the root .a, the
    // linker would consume the .a and the host would fail to link
    // SDL3 / AVFoundation / etc.
    let mut best: Option<(path::PathBuf, std::time::SystemTime, bool)> = None;
    let mut consider = |dir: path::PathBuf| {
        if dir_has_mocida_lib(&dir) {
            let mtime = fs::metadata(&dir)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let is_dylib = dir.join("libmocida.dylib").exists();
            let is_better = match best.as_ref() {
                Some((_, prev_t, prev_dylib)) => {
                    // Always prefer dylib over .a (even if older); within
                    // the same lib kind, the most recent build wins.
                    is_dylib && !*prev_dylib || (is_dylib == *prev_dylib && mtime > *prev_t)
                }
                None => true,
            };
            if is_better {
                best = Some((dir, mtime, is_dylib));
            }
        }
    };
    // Scan the root AND every subdir (1-2 levels deep, matching the
    // standard mocida build layout: <root>/libmocida.{a,dylib},
    // <root>/<plat>/<prof>/libmocida.{a,dylib}, etc.).
    if build_root.is_dir() {
        consider(build_root.to_path_buf());
    }
    if let Ok(plats) = fs::read_dir(build_root) {
        for plat in plats.flatten() {
            let pdir = plat.path();
            if !pdir.is_dir() {
                continue;
            }
            consider(pdir.clone());
            if let Ok(profs) = fs::read_dir(&pdir) {
                for prof in profs.flatten() {
                    let d = prof.path();
                    if d.is_dir() {
                        consider(d);
                    }
                }
            }
        }
    }
    best.map(|(p, _, _)| p)
}

/// A directory likely holding libclang, by OS. Windows/macOS use the well-known
/// LLVM install dirs; on Linux bindgen's own discovery is reliable, so we return
/// `None` and let it find libclang on the system.
fn default_libclang_dir() -> Option<path::PathBuf> {
    #[cfg(target_os = "windows")]
    let candidates: &[&str] = &[
        r"C:\Program Files\LLVM\bin",
        r"C:\Program Files (x86)\LLVM\bin",
    ];
    #[cfg(target_os = "macos")]
    let candidates: &[&str] = &["/opt/homebrew/opt/llvm/lib", "/usr/local/opt/llvm/lib"];
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let candidates: &[&str] = &[];

    candidates
        .iter()
        .map(path::PathBuf::from)
        .find(|p| p.is_dir())
}
pub const COPPER_PATH: Lazy<String> = Lazy::new(|| std::env::var("COPPER_PATH").unwrap());

pub fn get_copper_version() -> String {
    // CalVer 0.YY.M — computed from the build date by build.rs at compile time.
    env!("COPPER_VERSION").to_string()
}

pub fn print() {
    vprint!("📂 Copper path: {}", (*COPPER_PATH).blue());
    let cargo_output = if cfg!(windows) {
        Command::new("where")
            .arg("cargo")
            .output()
            .unwrap_or_else(|_| panic!("Failed to execute 'where cargo'"))
    } else {
        Command::new("which")
            .arg("cargo")
            .output()
            .unwrap_or_else(|_| panic!("Failed to execute 'which cargo'"))
    };

    let cargo_path = String::from_utf8_lossy(&cargo_output.stdout);

    vprint!("📦 Cargo found at: {}", cargo_path.trim().blue());

    let rustc_output = if cfg!(windows) {
        Command::new("where")
            .arg("rustc")
            .output()
            .unwrap_or_else(|_| panic!("Failed to execute 'where rustc'"))
    } else {
        Command::new("which")
            .arg("rustc")
            .output()
            .unwrap_or_else(|_| panic!("Failed to execute 'which rustc'"))
    };

    let rustc_path = String::from_utf8_lossy(&rustc_output.stdout);

    vprint!("🦀 Rustc found at: {}", rustc_path.trim().blue());
    vprint!("Using CForge v{}", VERSION);
    vprint!("Using Copper v{}", get_copper_version());
}

pub fn compile(
    files: Vec<String>,
    input_dir: Option<String>,
    output_dir: Option<String>,
) -> Vec<String> {
    let copper_version = get_copper_version();
    let mut all_dependencies = Vec::new();

    // When the user passes a single `.crs` file (no input dir), treat it as
    // the program's entry point and emit it as `dist/rust/src/main.rs`. That
    // is the path Cargo treats as the default binary, so a subsequent
    // `cargo run` always picks up *this* compilation instead of falling back
    // to whatever `main.rs` was left over from an earlier build.
    let single_file_entry = input_dir.is_none() && files.len() == 1;

    // Output `src/` root, and the Rust modules copied in verbatim (so a Copper
    // project can mix hand-written `.rs` files next to its `.crs` sources).
    let src_root = format!(
        "{}/rust/src/",
        output_dir.clone().unwrap_or("./dist".to_string())
    );
    let mut rust_modules: Vec<String> = Vec::new();
    // Output files written, for the post-compile tree.
    let mut written: Vec<String> = Vec::new();
    // Stems of every local source file (`.rs`/`.crs`). An `import { … } from
    // <name>` that matches one of these is a sibling module, not a crate
    // dependency, so we drop it from the detected deps before generate_toml.
    let mut local_modules: Vec<String> = Vec::new();

    for mut file in files {
        file = file.replace(path::MAIN_SEPARATOR_STR, "/");
        if let Some(stem) = path::Path::new(&file).file_stem().and_then(|s| s.to_str()) {
            local_modules.push(stem.to_string());
        }
        let ext = path::Path::new(&file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        // Rust files are linked in verbatim, not transpiled. A lone `.rs` file
        // is the program's entry (emitted as `main.rs`); otherwise it's a module
        // copied alongside the transpiled Copper, wired in below so a Copper
        // `import { x } from <module>` (→ `use <module>::{x}`) resolves.
        if ext == "rs" {
            let content = match fs::read_to_string(&file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error reading {file}: {e}");
                    std::process::exit(1);
                }
            };
            fs::create_dir_all(&src_root).unwrap();
            let dest = if single_file_entry {
                format!("{src_root}main.rs")
            } else {
                let module = rust_module_name(&file);
                if !rust_modules.contains(&module) {
                    rust_modules.push(module.clone());
                }
                format!("{src_root}{module}.rs")
            };
            fs::write(&dest, content).unwrap();
            written.push(dest.clone());
            vprint!("  {} {} {}", file, "=>".yellow(), dest);
            continue;
        }

        // Skip anything that's neither Copper nor Rust — a project directory may
        // hold assets, kson, markdown, … which must not be tokenized as Copper.
        if ext != "crs" {
            vprint!("  {} {}", "skipping non-source".dimmed(), file);
            continue;
        }

        vprint!("🔨 Compiling {}", file);

        let c = fs::read(&file).unwrap();
        let c = String::from_utf8(c).unwrap();

        let mut tokenizer = Tokenizer::new(c);
        let tokens = tokenizer.tokenize();
        if !tokenizer.errors.is_empty() {
            // The tokenizer no longer aborts the process directly (so the
            // LSP can keep running across malformed edits). Mirror the
            // old CLI behavior here: print every collected error and exit.
            for e in &tokenizer.errors {
                eprintln!("Error: {}", e);
            }
            std::process::exit(1);
        }
        let source_path = path::PathBuf::from(&file);
        let project_root = input_dir
            .as_ref()
            .map(path::PathBuf::from)
            .or_else(|| source_path.parent().map(|p| p.to_path_buf()));
        let mut parser =
            parser::Parser::new(tokens).with_paths(Some(source_path.clone()), project_root);
        let mut result_code = parser.parse();
        // Non-entry modules must export their types/fns so
        // `use crate::math::vec2::{Vec2}` resolves.
        let is_entry = single_file_entry
            || path::Path::new(&file).file_stem().and_then(|s| s.to_str()) == Some("main");
        if !is_entry {
            result_code = promote_module_pub(&result_code);
        }

        // Collect detected dependencies
        let dependencies = parser.get_required_dependencies();
        for dep in dependencies {
            if !all_dependencies.contains(&dep) {
                all_dependencies.push(dep);
            }
        }

        // Each used std module is written as a SEPARATE local crate under
        // `__copper__/std/<name>/` (lib + its own Cargo.toml), and the main
        // project path-depends on `copper_<name>` instead of inlining the
        // module's code. The dep spec `copper_<name>=path:<rel>` is honoured by
        // generate_toml.
        let rust_root = format!(
            "{}/rust",
            output_dir.clone().unwrap_or("./dist".to_string())
        );
        for (name, lib_src) in parser.std_lib_crates() {
            let rel = format!("__copper__/std/{name}");
            let crate_dir = format!("{rust_root}/{rel}");
            fs::create_dir_all(format!("{crate_dir}/src")).unwrap();
            fs::write(format!("{crate_dir}/src/lib.rs"), lib_src).unwrap();
            // The module crate's own deps (pinned `name@version`).
            let mut deps_section = String::new();
            for spec in parser::Parser::std_module_crate_dependencies(name) {
                let (dn, dv) = spec.split_once('@').unwrap_or((spec, "*"));
                deps_section.push_str(&format!("{dn} = \"{dv}\"\n"));
            }
            let manifest = format!(
                "# Generated by cforge — Copper std module `{name}`.\n\
                 [package]\n\
                 name = \"copper_{name}\"\n\
                 version = \"0.1.0\"\n\
                 edition = \"2021\"\n\n\
                 [lib]\n\
                 path = \"src/lib.rs\"\n\n\
                 [dependencies]\n{deps_section}"
            );
            fs::write(format!("{crate_dir}/Cargo.toml"), manifest).unwrap();
            let spec = format!("copper_{name}=path:{rel}");
            if !all_dependencies.contains(&spec) {
                all_dependencies.push(spec);
            }
            vprint!("📦 std crate copper_{name} => {rel}");
        }

        let basepath = &format!(
            "{}/rust/src/",
            output_dir.clone().unwrap_or("./dist".to_string())
        );
        fs::create_dir_all(basepath).unwrap();

        // Map source path → `src/<relative>.rs`. Prefer stripping the project
        // root so `engine/math/vec2.crs` becomes `src/math/vec2.rs` (and
        // `crate::math::vec2` resolves). Never concatenate absolute paths into
        // the output root.
        let path = if single_file_entry {
            format!("{basepath}main.rs")
        } else if let Some(ref input_dir) = input_dir {
            let file_n = file.replace('\\', "/");
            let dir_n = {
                let d = input_dir.replace('\\', "/");
                let d = d.trim_end_matches('/').to_string();
                // Resolve `.` / relative roots against cwd for strip_prefix.
                path::PathBuf::from(&d)
                    .canonicalize()
                    .unwrap_or_else(|_| path::PathBuf::from(&d))
                    .to_string_lossy()
                    .replace('\\', "/")
            };
            let file_c = path::PathBuf::from(&file_n)
                .canonicalize()
                .unwrap_or_else(|_| path::PathBuf::from(&file_n))
                .to_string_lossy()
                .replace('\\', "/");
            let rel = file_c
                .strip_prefix(&(dir_n.clone() + "/"))
                .or_else(|| file_c.strip_prefix(&dir_n))
                .unwrap_or_else(|| {
                    // Fallback: file may already be relative to input_dir.
                    file_n.strip_prefix("./").unwrap_or(file_n.as_str())
                });
            let rel = rel.trim_start_matches('/');
            format!(
                "{}{}",
                basepath,
                rel.replace(".crs", ".rs").replace(".crm", ".rs")
            )
            .replace('\\', "/")
        } else {
            let name = path::Path::new(&file)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("module.crs");
            format!(
                "{}{}",
                basepath,
                name.replace(".crs", ".rs").replace(".crm", ".rs")
            )
            .replace('\\', "/")
        };

        let result = format!(
            "// Generated by CForge v{} with Copper v{}\n{}",
            VERSION, copper_version, result_code
        );
        // Create directories recursively if they don't exist
        let path_obj = std::path::Path::new(&path);

        if let Some(parent) = path_obj.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path.clone(), result).unwrap();
        written.push(path.clone());

        vprint!("  {} {} {}", file, "=>".yellow(), path);
    }

    // Wire any copied Rust modules into the crate root so cargo compiles them
    // and Copper `import { x } from <module>` resolves. Prepend `pub mod`
    // declarations to the generated `main.rs`.
    if !rust_modules.is_empty() {
        let main_path = format!("{src_root}main.rs");
        match fs::read_to_string(&main_path) {
            Ok(existing) => {
                let decls: String = rust_modules
                    .iter()
                    .map(|m| format!("pub mod {m};\n"))
                    .collect();
                if fs::write(&main_path, format!("{decls}{existing}")).is_ok() {
                    vprint!("🔗 linked Rust module(s): {}", rust_modules.join(", "));
                }
            }
            Err(_) => {
                eprintln!(
                    "⚠️  Linked Rust file(s) {} but found no main.rs entry to declare them in; \
                     add a `main.crs` or reference them by path.",
                    rust_modules.join(", ")
                );
            }
        }
    }

    // Nested Copper outputs (`math/vec2.rs`, `ecs/world.rs`) need `mod`
    // declarations so `use crate::math::vec2::{…}` resolves. Build the tree
    // under src/ and declare top-level modules from main.rs.
    wire_nested_copper_modules(&src_root, &written);

    // Drop deps that are really sibling modules (local `.rs`/`.crs` files
    // imported via `from <name>`), keeping only genuine external crates.
    all_dependencies.retain(|d| !local_modules.contains(d));

    pretty::ok("Successfully compiled");
    if !written.is_empty() {
        // Show the generated files as a tree (paths relative to the output root).
        let base = format!(
            "{}/rust/",
            output_dir.clone().unwrap_or("./dist".to_string())
        )
        .replace('\\', "/");
        let rel: Vec<String> = written
            .iter()
            .map(|p| {
                let p = p.replace('\\', "/");
                p.strip_prefix(&base).map(str::to_string).unwrap_or(p)
            })
            .collect();
        pretty::tree("dist/rust", &rel);
    }
    all_dependencies
}

/// A valid Rust module name derived from a `.rs` file's stem (so a copied
/// `helpers.rs` is reachable as `mod helpers`). Non-identifier chars become `_`;
/// a leading digit is prefixed with `_`.
fn rust_module_name(file: &str) -> String {
    let stem = path::Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let mut name: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if name.is_empty() {
        name.push_str("module");
    }
    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name
}

/// Prefix top-level items with `pub` so sibling modules can `use` them.
fn promote_module_pub(code: &str) -> String {
    let mut out = String::with_capacity(code.len() + 64);
    for line in code.lines() {
        let trimmed = line.trim_start();
        let promote = (trimmed.starts_with("fn ")
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("enum ")
            || trimmed.starts_with("trait ")
            || trimmed.starts_with("type ")
            || trimmed.starts_with("const ")
            || trimmed.starts_with("static "))
            && !trimmed.starts_with("fn main(")
            && !trimmed.starts_with("pub ");
        // Also promote `#[derive…]`-preceded structs: the attribute line is
        // left alone; the next `struct` line gets `pub`.
        if promote && line.starts_with(trimmed) {
            out.push_str("pub ");
            out.push_str(line);
        } else if promote {
            // Indented top-level shouldn't happen; still promote carefully.
            let indent_len = line.len() - trimmed.len();
            out.push_str(&line[..indent_len]);
            out.push_str("pub ");
            out.push_str(trimmed);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Ensure nested transpiled Copper files are reachable via `crate::a::b`.
///
/// For each written path under `src/` like `math/vec2.rs`:
/// - create/update `math/mod.rs` with `pub mod vec2;`
/// - declare `pub mod math;` in `main.rs` (and promote items in leaf files)
fn wire_nested_copper_modules(src_root: &str, written: &[String]) {
    use std::collections::{BTreeMap, BTreeSet};

    let root = path::PathBuf::from(src_root.replace('\\', "/"));
    let root_s = root.to_string_lossy().replace('\\', "/");
    let root_s = root_s.trim_end_matches('/').to_string();
    let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut top_level: BTreeSet<String> = BTreeSet::new();

    for w in written {
        let p_s = w.replace('\\', "/");
        let rel_s = p_s
            .strip_prefix(&format!("{root_s}/"))
            .or_else(|| p_s.strip_prefix(&root_s))
            .unwrap_or("");
        if rel_s.is_empty() {
            continue;
        }
        let rel = path::Path::new(rel_s);
        let parts: Vec<&str> = rel
            .iter()
            .map(|s| s.to_str().unwrap_or(""))
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() || parts[0] == "main.rs" {
            continue;
        }
        // `math.rs` at src root — already a top-level module file.
        if parts.len() == 1 {
            if let Some(stem) = path::Path::new(parts[0])
                .file_stem()
                .and_then(|s| s.to_str())
            {
                if path::Path::new(parts[0])
                    .extension()
                    .and_then(|e| e.to_str())
                    == Some("rs")
                {
                    top_level.insert(stem.to_string());
                }
            }
            continue;
        }
        // `math/vec2.rs` → parent "math" declares child "vec2"
        if let Some(file) = parts.last() {
            if path::Path::new(file).extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Some(child) = path::Path::new(file).file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            // Walk parents: math/ecs/world.rs → math declares ecs, ecs declares world
            for i in 0..parts.len() - 1 {
                let parent = parts[..i + 1].join("/");
                let child_name = if i + 1 == parts.len() - 1 {
                    child.to_string()
                } else {
                    parts[i + 1].to_string()
                };
                // Skip if child_name looks like a file with extension mid-path
                if child_name.contains('.') {
                    continue;
                }
                children.entry(parent).or_default().insert(child_name);
            }
            top_level.insert(parts[0].to_string());
        }
    }

    for (parent, mods) in &children {
        let mod_rs = root.join(parent).join("mod.rs");
        if let Some(dir) = mod_rs.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let mut existing = fs::read_to_string(&mod_rs).unwrap_or_default();
        let mut added = false;
        for m in mods {
            let decl = format!("pub mod {m};");
            if !existing.contains(&decl) && !existing.contains(&format!("mod {m};")) {
                if !existing.is_empty() && !existing.ends_with('\n') {
                    existing.push('\n');
                }
                existing.push_str(&decl);
                existing.push('\n');
                added = true;
            }
        }
        if added || !mod_rs.exists() {
            if let Err(e) = fs::write(&mod_rs, &existing) {
                eprintln!("⚠️  could not write {}: {e}", mod_rs.display());
            } else {
                vprint!(
                    "🔗 nested mod {} → {}",
                    parent,
                    mods.iter().cloned().collect::<Vec<_>>().join(", ")
                );
            }
        }
    }

    // Declare top-level modules from main.rs (or lib.rs if there's no binary entry).
    if top_level.is_empty() {
        return;
    }
    let main_path = root.join("main.rs");
    let lib_path = root.join("lib.rs");
    let entry_path = if main_path.exists() {
        main_path
    } else {
        lib_path
    };
    let mut existing = fs::read_to_string(&entry_path).unwrap_or_default();
    let mut prefix = String::new();
    for m in &top_level {
        let decl = format!("pub mod {m};");
        if !existing.contains(&decl) && !existing.contains(&format!("mod {m};")) {
            prefix.push_str(&decl);
            prefix.push('\n');
        }
    }
    // Also declare root-level `.rs` modules like `prelude` / `scene_file`.
    if !prefix.is_empty() || !entry_path.exists() {
        existing = format!("{prefix}{existing}");
        if existing.trim().is_empty() {
            existing.push_str("// Generated module root for Copper crate paths.\n");
        }
        if fs::write(&entry_path, existing).is_ok() {
            vprint!(
                "🔗 crate mods ({}): {}",
                entry_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?"),
                top_level.iter().cloned().collect::<Vec<_>>().join(", ")
            );
        }
    }
}

pub fn get_toml_package_name() -> String {
    let toml = fs::read("./dist/rust/Cargo.toml").unwrap();
    let toml = String::from_utf8(toml).unwrap();

    let name = toml.split("name = \"").collect::<Vec<&str>>()[1]
        .split("\"")
        .collect::<Vec<&str>>()[0];

    name.to_string()
}

pub fn get_toml_package_version() -> String {
    let toml = fs::read("./dist/rust/Cargo.toml").unwrap();
    let toml = String::from_utf8(toml).unwrap();

    let version = toml.split("version = \"").collect::<Vec<&str>>()[1]
        .split("\"")
        .collect::<Vec<&str>>()[0];

    version.to_string()
}

/// Resolve a user-friendly target name to the canonical Rust triple.
/// Pass-through for anything that's already a triple (contains `-`).
pub fn resolve_target(name: &str) -> &'static str {
    match name {
        "windows" | "win" | "win64" => "x86_64-pc-windows-msvc",
        "windows-gnu" | "win-gnu" => "x86_64-pc-windows-gnu",
        "linux" | "linux64" => "x86_64-unknown-linux-gnu",
        "linux-musl" => "x86_64-unknown-linux-musl",
        "macos" | "mac" | "darwin" | "osx" => "x86_64-apple-darwin",
        "macos-arm" | "mac-arm" | "m1" | "m2" => "aarch64-apple-darwin",
        "linux-arm" | "linux-aarch64" => "aarch64-unknown-linux-gnu",
        "wasm" | "wasm32" => "wasm32-unknown-unknown",
        // Already a full triple or unknown alias — pass through (Cargo will validate).
        _ => Box::leak(name.to_string().into_boxed_str()),
    }
}

fn target_os_label(triple: &str) -> &'static str {
    if triple.contains("windows") {
        "windows"
    } else if triple.contains("apple") {
        "macos"
    } else if triple.contains("wasm") {
        "wasm"
    } else if triple.contains("linux") {
        "linux"
    } else {
        "native"
    }
}

fn target_exe_ext(triple: &str) -> &'static str {
    if triple.contains("windows") {
        ".exe"
    } else {
        ""
    }
}

/// Ensure the rustup target toolchain is installed, silently skipping if
/// rustup is not available (the user may be using a custom toolchain).
fn ensure_target_installed(triple: &str) {
    let already_installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout).to_string();
            out.lines().any(|l| l.trim() == triple)
        })
        .unwrap_or(false);

    if already_installed {
        return;
    }

    pretty::step(&format!("rustup target add {triple}"));
    let ok = Command::new("rustup")
        .args(["target", "add", triple])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !ok {
        pretty::warn(&format!(
            "Could not add {triple} via rustup. \
             If you need a cross-linker, try `cargo install cross` \
             and run `cross build --target {triple}` in dist/rust/ manually."
        ));
    }
}

/// Cargo-build the dist project for a specific Rust triple and copy the
/// resulting binary to `dist/<name>-<os>[.exe]`.
///
/// Returns the path to the output binary, or `None` on build failure.
pub fn build_for_target(triple: &str, release: bool) -> Option<path::PathBuf> {
    ensure_target_installed(triple);

    let label = target_os_label(triple);
    let profile = if release { "release" } else { "debug" };

    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("--target").arg(triple);
    if release {
        cmd.arg("--release");
    }
    cmd.current_dir("./dist/rust");

    let (ok, _) = cargo_with_spinner(
        cmd,
        &format!("compiling for {label}"),
        &format!("Built for {label}"),
    );
    if !ok {
        pretty::fail(&format!(
            "Build failed for target {triple}. \
             Cross-compiling requires a compatible linker on this host. \
             Alternative: `cargo install cross` then `cross build --target {triple}` in dist/rust/."
        ));
        return None;
    }

    let name = get_toml_package_name();
    let ext = target_exe_ext(triple);
    let src = path::PathBuf::from(format!("./dist/rust/target/{triple}/{profile}/{name}{ext}"));

    fs::create_dir_all("./dist").ok();
    let dest_name = format!("{name}-{label}{ext}");
    let dest = path::PathBuf::from(format!("./dist/{dest_name}"));

    match fs::copy(&src, &dest) {
        Ok(_) => {
            pretty::ok(&format!("→  dist/{dest_name}"));
            Some(dest)
        }
        Err(e) => {
            pretty::warn(&format!(
                "Compiled OK but could not copy to dist/: {e}\n  Binary at: {}",
                src.display()
            ));
            Some(src)
        }
    }
}

pub fn run() {
    pretty::head("Building and running project");

    let target = std::env::var("CFORGE_TARGET").ok();
    let release = std::env::var("CFORGE_RELEASE")
        .map(|v| v == "1")
        .unwrap_or(false);

    // Cross-compile: produce the binary for the target OS but don't try to
    // execute it — you can't run a foreign binary on this host.
    if let Some(ref triple) = target {
        build_for_target(triple, release);
        return;
    }

    // Native build.
    let mut build_cmd = Command::new("cargo");
    build_cmd.arg("build");
    if release {
        build_cmd.arg("--release");
    }
    build_cmd.current_dir("./dist/rust");
    let (ok, _) = cargo_with_spinner(build_cmd, "compiling (cargo build)", "Build successful");
    if !ok {
        return;
    }

    println!(
        "{} {} v{}:\n",
        pretty::arrow(),
        get_toml_package_name().bold(),
        get_toml_package_version().bold(),
    );

    // Run. Inherit stdio so interactive programs (input(), prompts, ...) work.
    let mut run_cmd = Command::new("cargo");
    run_cmd.arg("run").arg("--quiet");
    if release {
        run_cmd.arg("--release");
    }
    run_cmd.current_dir("./dist/rust");
    run_cmd.stdin(std::process::Stdio::inherit());
    run_cmd.stdout(std::process::Stdio::inherit());
    run_cmd.stderr(std::process::Stdio::inherit());
    let status = run_cmd.status().expect("Failed to execute cargo run");

    if !status.success() {
        if let Some(code) = status.code() {
            pretty::fail(&format!("Program exited with code {}", code));
        } else {
            pretty::fail("Program terminated by signal");
        }
    }
}

pub fn ensure_copper_dep_built(pkg_dir: &path::Path) {
    if pkg_dir.join("crs").is_dir() {
        if let Err(e) = module_transpile::transpile_crs_project(pkg_dir) {
            eprintln!(
                "⚠️  Failed to transpile Copper package {}: {e}",
                pkg_dir.display()
            );
        }
        return;
    }
    let dist_cargo = pkg_dir.join("dist").join("rust").join("Cargo.toml");
    if dist_cargo.exists() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe)
            .arg("-c")
            .arg("-i")
            .arg(pkg_dir)
            .current_dir(pkg_dir)
            .status();
    }
}

pub async fn generate_toml(extra_dependencies: Vec<String>) {
    let properties = kson::read_properties(
        std::env::current_dir()
            .unwrap()
            .join("properties.kson")
            .to_str()
            .unwrap(),
    );
    let toml: String;

    vprint!("🔍 Debug: properties.0 (is_toml): {}", properties.0);
    vprint!(
        "🔍 Debug: properties.1 JSON: {}",
        serde_json::to_string_pretty(&properties.1)
            .unwrap_or_else(|_| "Failed to serialize".to_string())
    );

    if properties.0 {
        println!("⚠️  Warning: Detected Cargo.toml file. CForge now uses properties.kson as the main configuration file. Please migrate your configuration to properties.kson. See https://copper-lang.org/docs/cforge/properties for more information.");
        let mut properties_obj = properties::Properties::from_toml(&properties.1).await;

        // Add extra detected dependencies. A `name@version` spec pins the
        // version (std modules target a specific crate API, e.g. `ureq@2`);
        // a bare name resolves to the latest.
        for dep in &extra_dependencies {
            // `name=path:<rel>` is a local path dependency (a per-module std
            // crate under `__copper__/std/<name>/`).
            if let Some((name, path)) = dep.split_once("=path:") {
                properties_obj.add_path_dependency(name, path);
                continue;
            }
            let (name, version) = match dep.split_once('@') {
                Some((n, v)) => (n, v),
                None => (dep.as_str(), "latest"),
            };
            properties_obj.add_dependency(name, version).await;
        }

        toml = properties_obj.to_toml();
        vprint!("Using Cargo.toml for configuration");
    } else {
        let mut properties_obj = properties::Properties::from_kson(&properties.1).await;

        // Add extra detected dependencies. A `name@version` spec pins the
        // version (std modules target a specific crate API, e.g. `ureq@2`);
        // a bare name resolves to the latest.
        for dep in &extra_dependencies {
            // `name=path:<rel>` is a local path dependency (a per-module std
            // crate under `__copper__/std/<name>/`).
            if let Some((name, path)) = dep.split_once("=path:") {
                properties_obj.add_path_dependency(name, path);
                continue;
            }
            let (name, version) = match dep.split_once('@') {
                Some((n, v)) => (n, v),
                None => (dep.as_str(), "latest"),
            };
            properties_obj.add_dependency(name, version).await;
        }

        toml = properties_obj.to_toml();
        vprint!("Using properties.kson for configuration");
    }

    fs::write("./dist/rust/Cargo.toml", toml).unwrap();
    vprint!("📦 Cargo.toml generated")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;

    #[test]
    fn parses_cargo_package_ids() {
        assert_eq!(
            pkg_name_from_id("registry+https://github.com/rust-lang/crates.io-index#serde@1.0.228"),
            "serde"
        );
        assert_eq!(
            pkg_name_from_id("path+file:///C:/Users/dev/mocida/mocida-rs/mocida-sys#0.1.0"),
            "mocida-sys"
        );
        assert_eq!(
            pkg_name_from_id("mocida 0.1.0 (path+file:///...)"),
            "mocida"
        );
    }

    #[test]
    fn tracks_parallel_crates_build_scripts_and_errors() {
        let state = Mutex::new(pretty::BuildProgressState::default());
        let diagnostics = Mutex::new(String::new());
        let mut done = HashSet::new();

        handle_cargo_stderr_line("   Compiling mocida-sys v0.1.0", &state, true);
        handle_cargo_stderr_line("   Compiling mocidad-launcher v0.1.0", &state, true);
        assert_eq!(state.lock().unwrap().started, 2);
        assert_eq!(
            state.lock().unwrap().active,
            vec!["mocida-sys", "mocidad-launcher"]
        );

        let build_script_built = serde_json::json!({
            "reason": "compiler-artifact",
            "package_id": "path+file:///repo/mocidad-launcher#0.1.0",
            "target": { "kind": ["custom-build"], "name": "build-script-build" },
            "fresh": false
        });
        handle_cargo_json(&build_script_built, &state, &diagnostics, &mut done, true);
        assert_eq!(done.len(), 0);
        assert_eq!(
            state.lock().unwrap().active,
            vec!["mocida-sys", "mocidad-launcher (build.rs)"]
        );

        let build_script_done = serde_json::json!({
            "reason": "build-script-executed",
            "package_id": "path+file:///repo/mocidad-launcher#0.1.0"
        });
        handle_cargo_json(&build_script_done, &state, &diagnostics, &mut done, true);
        assert_eq!(
            state.lock().unwrap().active,
            vec!["mocida-sys", "mocidad-launcher"]
        );

        let lib_done = serde_json::json!({
            "reason": "compiler-artifact",
            "package_id": "path+file:///repo/mocida-sys#0.1.0",
            "target": { "kind": ["lib"], "name": "mocida_sys" },
            "fresh": false
        });
        handle_cargo_json(&lib_done, &state, &diagnostics, &mut done, true);
        assert_eq!(done.len(), 1);
        assert_eq!(state.lock().unwrap().active, vec!["mocidad-launcher"]);

        let err_msg = serde_json::json!({
            "reason": "compiler-message",
            "message": { "level": "error", "rendered": "error[E0308]: mismatched types\n" }
        });
        handle_cargo_json(&err_msg, &state, &diagnostics, &mut done, true);
        assert_eq!(state.lock().unwrap().errors, 1);
        assert!(diagnostics.lock().unwrap().contains("E0308"));

        handle_cargo_stderr_line(
            "warning: build failed, waiting for other jobs to finish...",
            &state,
            true,
        );
        assert!(state.lock().unwrap().build_failed);
    }

    #[test]
    fn counts_only_non_dev_packages_from_metadata() {
        let meta = serde_json::json!({
            "resolve": {
                "root": "root#0.1.0",
                "nodes": [
                    {
                        "id": "root#0.1.0",
                        "deps": [
                            { "pkg": "dep#1.0.0", "dep_kinds": [{ "kind": null }] },
                            { "pkg": "build_dep#1.0.0", "dep_kinds": [{ "kind": "build" }] },
                            { "pkg": "dev_only#1.0.0", "dep_kinds": [{ "kind": "dev" }] }
                        ]
                    },
                    { "id": "dep#1.0.0", "deps": [] },
                    { "id": "build_dep#1.0.0", "deps": [] },
                    { "id": "dev_only#1.0.0", "deps": [] }
                ]
            }
        });
        assert_eq!(count_non_dev_packages(&meta), Some(3));
    }
}
