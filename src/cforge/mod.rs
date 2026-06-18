pub mod commands;
pub mod fetch;
pub mod kson;
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

/// Run a `cargo` command behind cforge's own spinner instead of cargo's output:
/// stdout+stderr are captured (so cargo's progress bar / `Compiling …` spam is
/// hidden), the spinner label tracks the crate currently compiling, and the
/// captured output is only printed on failure. Returns `(success, output)`.
pub(crate) fn cargo_with_spinner(
    mut cmd: Command,
    building_label: &str,
    done_label: &str,
) -> (bool, String) {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};

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

    let sp = pretty::Spinner::start(building_label);
    let label = sp.label_handle();
    let collected = Arc::new(Mutex::new(String::new()));

    // stderr carries cargo's `Compiling <crate>` progress → drive the spinner.
    let mut readers = Vec::new();
    if let Some(stderr) = child.stderr.take() {
        let label = label.clone();
        let collected = collected.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let t = line.trim_start();
                if let Some(rest) = t.strip_prefix("Compiling ") {
                    if let Some(name) = rest.split_whitespace().next() {
                        if let Ok(mut l) = label.lock() {
                            *l = format!("compiling {name}");
                        }
                    }
                }
                if let Ok(mut c) = collected.lock() {
                    c.push_str(&line);
                    c.push('\n');
                }
            }
        }));
    }
    if let Some(stdout) = child.stdout.take() {
        let collected = collected.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(mut c) = collected.lock() {
                    c.push_str(&line);
                    c.push('\n');
                }
            }
        }));
    }

    let status = child.wait();
    for r in readers {
        let _ = r.join();
    }
    let output = collected.lock().map(|c| c.clone()).unwrap_or_default();
    let ok = status.map(|s| s.success()).unwrap_or(false);

    if ok {
        sp.done(done_label);
    } else {
        sp.fail("Build failed");
        // Surface cargo's captured output so the error is visible.
        print!("{output}");
    }
    (ok, output)
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
                    is_dylib && !*prev_dylib
                        || (is_dylib == *prev_dylib && mtime > *prev_t)
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
        let mut parser = parser::Parser::new(tokens);
        let result_code = parser.parse();

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
        let rust_root = format!("{}/rust", output_dir.clone().unwrap_or("./dist".to_string()));
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

        let mut path = format!("{}{}", basepath, file);
        path = path.replace(".crs", ".rs").replace("\\", "/");

        if let Some(ref input_dir) = input_dir {
            let input_dir = input_dir.clone().replace("\\", "/").replace(".crs", ".rs");
            path = path.replacen(&(input_dir + "/"), "", 1);
        } else if single_file_entry {
            path = format!("{}main.rs", basepath);
        }

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
