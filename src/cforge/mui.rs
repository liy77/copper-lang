//! MUI integration for cforge.
//!
//! `.mui` / `.crm` files describe a mocida UI (see `mocida/mui/ARCHITECTURE.md`).
//! In **dev** they're rendered live by the `mui-dev` host (M1: static render;
//! mocida C library + bindgen into every cforge build and break the portable
//! Linux/macOS CI. Instead, mirroring how `run()` shells out to `cargo`, cforge
//! **invokes the `mui-dev` binary** in the neighbouring `mocida-rs` workspace.
//!
//! Resolution order for the renderer:
//!   1. `MUI_DEV_BIN` env var → an explicit path to a `mui-dev` executable.
//!   2. A pre-built `mui-dev[.exe]` under a discovered `mocida-rs/target/`
//!      (debug or release). Preferred: its mocida/SDL DLLs are already staged
//!      beside it, so it runs with no extra setup.
//!   3. `cargo run -p mui-dev -- <file>` inside a discovered `mocida-rs`
//!      workspace (needs the mocida C lib + clang; see `mocida-rs/EXAMPLES.md`).

use colored::Colorize;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::tokenizer::tokenizer::Tokenizer;

/// True if `file` is a MUI document cforge should render rather than transpile.
pub fn is_mui_file(file: &str) -> bool {
    matches!(
        Path::new(file)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("mui") | Some("crm")
    )
}

/// **M5 — release codegen**, the `.mui`/`.crm` side of `cforge -c`. Lower the
/// component AST to a self-contained Rust project under `<output>/mui/` (a
/// `main.rs` that builds the mocida tree via `mocida-rs`, plus a `Cargo.toml`)
/// and print the generated source. When `do_build` (i.e. `-c -r`/`--release`),
/// also `cargo build` it into a native binary. Returns the exit code.
///
/// This mirrors `cforge -c foo.crs` (which transpiles Copper → Rust): same
/// `--compile` entry point, the file extension picks the backend. Unlike
/// `run()` (the dev runtime), this *produces code you can read and ship*. The
/// generated crate depends on `mocida` by path (discovered next to the
/// renderer), so it builds anywhere the dev host does.
pub fn compile(file: &str, output_dir: Option<&str>, do_build: bool, embed: bool) -> i32 {
    println!("🛠️  Generating Rust from {} (mui-codegen)...", file.bold());

    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("❌ cannot read {file}: {e}");
            return 1;
        }
    };

    // Parse → component AST, resolving `import { … } from "…"` relative to the
    // source file so imported components are inlined into the generated code.
    let loaded = mui_syntax::loader::load_from(&source, Path::new(file));
    for w in &loaded.warnings {
        eprintln!("⚠️  import: {w}");
    }
    let doc = &loaded.entry;
    for err in &doc.errors {
        eprintln!("⚠️  parse: {}", err.message);
    }
    if doc.views.is_empty() {
        eprintln!("❌ {file}: no `view` to generate");
        return 1;
    }

    let title = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("MUI");

    // Lay out a tiny cargo project under <output>/mui/ first, so the bundle can
    // be staged into it before codegen (the embed table include_bytes!'s it).
    let out_root = PathBuf::from(output_dir.unwrap_or("./dist")).join("mui");
    let src_dir = out_root.join("src");
    if let Err(e) = std::fs::create_dir_all(&src_dir) {
        eprintln!("❌ cannot create {}: {e}", src_dir.display());
        return 1;
    }

    // Stage a sibling app.bundle + the assets it references into the crate (so
    // the native build finds them). With `-b/--bundle`, also build an embed spec
    // that bakes them into the binary for a self-contained executable.
    let staged = stage_bundle(Path::new(file), &out_root);
    let embed_spec = if embed && !staged.is_empty() {
        let dir_name = doc
            .app
            .as_ref()
            .and_then(|a| a.id.clone().or_else(|| a.name.clone()))
            .map(|s| sanitize_dir(&s))
            .unwrap_or_else(|| sanitize_dir(title));
        println!(
            "📦 embedding {} file(s) into the binary (--bundle)",
            staged.len()
        );
        Some(mui_codegen::EmbedSpec {
            files: staged.clone(),
            dir_name,
        })
    } else {
        None
    };

    // Generate against the full registry (imported components + this file's own
    // sibling views), so views defined alongside the entry are inlined too.
    let components = loaded.registry();
    let code = mui_codegen::generate_program_with(doc, &components, title, embed_spec.as_ref());

    let main_rs = src_dir.join("main.rs");
    if let Err(e) = std::fs::write(&main_rs, &code) {
        eprintln!("❌ cannot write {}: {e}", main_rs.display());
        return 1;
    }

    // Materialize Copper (`.crs`) and Rust (`.rs`) imports as sibling modules of
    // main.rs, so the `mod`/`use` lines mui-codegen emitted resolve and the
    // imported functions/types are linked into the build.
    materialize_foreign_imports(file, doc, &src_dir);

    // The generated crate needs `mocida` (the safe wrapper). Point at the
    // workspace member by path so it builds without publishing anything.
    let mocida_path = match find_mocida_rs() {
        Some(ws) => ws.join("mocida"),
        None => {
            eprintln!(
                "{}",
                "❌ Could not locate the `mocida-rs` workspace for the `mocida` dependency.".red()
            );
            eprintln!("   Set MOCIDA_RS_DIR, or place copper-lang and mocida side by side.");
            return 1;
        }
    };
    let cargo_toml = render_cargo_toml(title, &mocida_path);
    if let Err(e) = std::fs::write(out_root.join("Cargo.toml"), cargo_toml) {
        eprintln!("❌ cannot write Cargo.toml: {e}");
        return 1;
    }

    // (The app.bundle + its assets were already staged above by stage_bundle.)

    // Report the generated file (the code itself stays on disk — printing the
    // whole thing to the terminal just buries the actual status). Use -V to
    // dump it for a quick look.
    crate::cforge::pretty::ok(&format!("Generated {}", main_rs.display()));
    {
        // Show the generated crate as a tree (paths relative to dist/mui/).
        let mut files: Vec<String> = vec!["Cargo.toml".to_string(), "src/main.rs".to_string()];
        for imp in &doc.imports {
            if imp.kind != mui_syntax::ast::ImportKind::Mui {
                files.push(format!("src/{}.rs", imp.module_name()));
            }
        }
        files.extend(staged.iter().cloned());
        files.sort();
        files.dedup();
        crate::cforge::pretty::tree("dist/mui", &files);
    }
    if std::env::var("CFORGE_VERBOSE").as_deref() == Ok("1") {
        println!("{}", "─".repeat(60).dimmed());
        print!("{code}");
        println!("{}", "─".repeat(60).dimmed());
    }

    if !do_build {
        // Plain `-c`: stop after generating, like `-c foo.crs` writes the .rs
        // without running it. Add `-r`/`--release` to also build a binary.
        println!(
            "{}",
            "ℹ️  Add -r/--release to build a native binary, or build it yourself:".dimmed()
        );
        println!("   cd {} && cargo build", out_root.display());
        return 0;
    }

    // `-c -r`: build the generated crate into an optimized native binary.
    println!();
    crate::cforge::pretty::head("Building the generated crate (release)");
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("--release").current_dir(&out_root);
    // Point mocida-sys at the C source headers + built lib so it doesn't bind
    // against a stale installed copy (the cause of "cannot find function in sys").
    if let Some(ws) = find_mocida_rs() {
        crate::cforge::apply_mocida_env(&mut cmd, &ws);
    }
    let (ok, _out) = crate::cforge::cargo_with_spinner(
        cmd,
        "compiling (release)",
        &format!("Built {}/target/release/", out_root.display()),
    );
    if ok {
        0
    } else {
        1
    }
}

/// Write each Copper (`.crs`) / Rust (`.rs`) import as a `<module>.rs` file next
/// to the generated `main.rs`. Copper sources are transpiled to Rust via the
/// cforge pipeline (and their top-level items made `pub` so the `use module::*`
/// in main.rs can see them); Rust sources are copied verbatim. The module name
/// matches `Import::module_name`, which mui-codegen used for the `mod` lines.
fn materialize_foreign_imports(file: &str, doc: &mui_syntax::ast::Document, src_dir: &Path) {
    use mui_syntax::ast::ImportKind;
    let base = Path::new(file)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut written: Vec<String> = Vec::new();
    for imp in &doc.imports {
        if imp.kind == ImportKind::Mui {
            continue;
        }
        let module = imp.module_name();
        if written.contains(&module) {
            continue;
        }
        let resolved = if Path::new(&imp.path).is_absolute() {
            PathBuf::from(&imp.path)
        } else {
            base.join(&imp.path)
        };
        let content = match std::fs::read_to_string(&resolved) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("⚠️  import {}: {e}", imp.path);
                continue;
            }
        };
        let (rust, lang) = match imp.kind {
            ImportKind::Copper => (transpile_copper_module(&content), "Copper"),
            ImportKind::Rust => (content, "Rust"),
            ImportKind::Mui => unreachable!(),
        };
        let header = format!(
            "// Generated by cforge from the {lang} import `{}`.\n\
             // Edit the source file, not this — re-run `cforge -c` to regenerate.\n",
            imp.path
        );
        let out = src_dir.join(format!("{module}.rs"));
        if let Err(e) = std::fs::write(&out, format!("{header}{rust}")) {
            eprintln!("⚠️  could not write {}: {e}", out.display());
        } else {
            println!("🔗 linked {lang} import {} → src/{module}.rs", imp.path);
            written.push(module);
        }
    }
}

/// Transpile a Copper (`.crs`) library file to Rust for use as a module, reusing
/// the cforge pipeline. Top-level `fn`/`struct`/`enum`/`trait` are promoted to
/// `pub` so a `use module::*;` re-exports them (mirrors how `cstd` is injected);
/// the unconditional `fn main()` the parser emits is left private so it isn't
/// re-exported.
fn transpile_copper_module(source: &str) -> String {
    let mut tokenizer = Tokenizer::new(source.to_string());
    let tokens = tokenizer.tokenize();
    if !tokenizer.errors.is_empty() {
        for e in &tokenizer.errors {
            eprintln!("⚠️  copper import: {e}");
        }
    }
    let mut parser = crate::parser::Parser::new(tokens);
    let code = parser.parse();
    promote_top_level_pub(&code)
}

/// Prefix top-level item lines with `pub` so a module's items are visible
/// through `use module::*`. Only column-0 `fn`/`struct`/`enum`/`trait` lines are
/// touched (nested/indented items are left alone), and `fn main(` is skipped so
/// the synthesized entry point stays private.
fn promote_top_level_pub(code: &str) -> String {
    let mut out = String::with_capacity(code.len() + 64);
    for line in code.lines() {
        let promote = (line.starts_with("fn ")
            || line.starts_with("struct ")
            || line.starts_with("enum ")
            || line.starts_with("trait "))
            && !line.starts_with("fn main(");
        if promote {
            out.push_str("pub ");
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Copy a sibling `app.bundle` and every asset it references into `out_root`,
/// preserving each asset's relative path. Returns the staged paths (relative to
/// `out_root`, forward-slashed) — `app.bundle` first — for the embed table.
fn stage_bundle(src_file: &Path, out_root: &Path) -> Vec<String> {
    let Some(dir) = src_file.parent() else {
        return Vec::new();
    };
    let bundle = dir.join("app.bundle");
    if !bundle.is_file() {
        return Vec::new();
    }

    let mut staged = Vec::new();
    match std::fs::copy(&bundle, out_root.join("app.bundle")) {
        Ok(_) => {
            staged.push("app.bundle".to_string());
            println!("📦 bundled {}", bundle.display());
        }
        Err(e) => {
            eprintln!("⚠️  could not copy app.bundle: {e}");
            return staged;
        }
    }

    // Copy each asset listed in the bundle's "assets" map (value = real path
    // relative to the bundle dir), preserving its relative layout.
    if let Ok(text) = std::fs::read_to_string(&bundle) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(assets) = json.get("assets").and_then(|a| a.as_object()) {
                for val in assets.values() {
                    let Some(rel) = val.as_str() else { continue };
                    let src = dir.join(rel);
                    let dst = out_root.join(rel);
                    if let Some(parent) = dst.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::copy(&src, &dst) {
                        Ok(_) => staged.push(rel.replace('\\', "/")),
                        Err(e) => eprintln!("⚠️  asset {}: {e}", src.display()),
                    }
                }
            }
        }
    }
    staged
}

/// Sanitize a string into a safe temp-dir name (keeps alnum / `.` / `-` / `_`).
fn sanitize_dir(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "mui_app".to_string()
    } else {
        out
    }
}

/// Cargo.toml for the generated crate: a binary that depends on `mocida` by
/// path. The crate name is derived from the source file stem.
fn render_cargo_toml(title: &str, mocida_path: &Path) -> String {
    let pkg = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .to_lowercase();
    let pkg = if pkg.is_empty() {
        "mui_app".to_string()
    } else {
        pkg
    };
    let mocida = mocida_path.to_string_lossy().replace('\\', "/");
    format!(
        "# Generated by cforge for a MUI release build.\n\
         # The empty [workspace] table detaches this crate from any parent\n\
         # workspace (e.g. copper-lang's), so `cargo build` here is standalone.\n\
         [workspace]\n\n\
         [package]\n\
         name = \"{pkg}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\n\
         [[bin]]\n\
         name = \"{pkg}\"\n\
         path = \"src/main.rs\"\n\n\
         [dependencies]\n\
         mocida = {{ path = \"{mocida}\" }}\n"
    )
}

/// Render a `.mui` / `.crm` file by handing it to the `mui-dev` host. Returns
/// the child's exit code (0 = ok). Prints actionable guidance if the renderer
/// can't be located or built.
pub fn run(file: &str) -> i32 {
    let abs = std::fs::canonicalize(file)
        .map(|p| strip_unc(&p))
        .unwrap_or_else(|_| file.to_string());

    // A `.mui` can declare a native host crate (`App() { host: "copper-installer" }`)
    // — a sibling crate that links its real backend (foreign imports, effects,
    // threads). Run that instead of the generic mui-dev, so `cforge run` shows
    // the fully-working app (backend + reactive UI).
    if let Some(host) = mui_app_host(file) {
        if let Some(code) = run_native_host(&abs, &host) {
            return code;
        }
        eprintln!(
            "{}",
            format!("⚠️  host crate '{host}' not found/built; falling back to mui-dev").yellow()
        );
    }

    println!("🎨 Rendering {} with mui-dev...", file.bold());

    // 1) Explicit override.
    if let Ok(bin) = std::env::var("MUI_DEV_BIN") {
        if Path::new(&bin).is_file() {
            return spawn_binary(&bin, &abs);
        }
        eprintln!(
            "⚠️  MUI_DEV_BIN points at '{}', which is not a file — falling back.",
            bin
        );
    }

    // 2) Pre-built binary next to its staged DLLs.
    if let Some(ws) = find_mocida_rs() {
        if let Some(bin) = prebuilt_mui_dev(&ws) {
            return spawn_binary(&bin.to_string_lossy(), &abs);
        }
        // 3) Build + run the `mui-dev` host via cargo — but only if the
        //    workspace actually ships it. When it doesn't (the common case:
        //    the live M1 host was never built), fall back to the codegen
        //    path (M5): generate Rust, native-build it, and run the binary.
        if workspace_has_mui_dev(&ws) {
            println!(
                "{}",
                "ℹ️  No pre-built mui-dev found; building it with cargo (first run only)..."
                    .dimmed()
            );
            return cargo_run_mui_dev(&ws, &abs);
        }
        println!(
            "{}",
            "ℹ️  No mui-dev host in the workspace; using the codegen build+run path instead."
                .dimmed()
        );
        return codegen_build_and_run(file, &ws);
    }

    eprintln!(
        "{}",
        "❌ Could not locate the `mocida-rs` workspace that provides `mui-dev`.".red()
    );
    eprintln!(
        "   Set MUI_DEV_BIN to a built mui-dev executable, or place the\n   \
         copper-lang and mocida repos side by side (…/copper-lang, …/mocida)."
    );
    1
}

/// Read a `.mui`'s `App() { host: "..." }` declaration, if any.
fn mui_app_host(file: &str) -> Option<String> {
    let src = std::fs::read_to_string(file).ok()?;
    mui_syntax::parse(&src).app.and_then(|a| a.host)
}

/// Run a `.mui`'s declared native host crate (`<file-dir>/<host>/`). Prefers a
/// pre-built executable (its mocida/SDL DLLs are already staged beside it, so it
/// needs no clang/env); else `cargo run` it. `None` if the crate isn't there.
fn run_native_host(file: &str, host: &str) -> Option<i32> {
    let dir = Path::new(file).parent()?.join(host);
    if !dir.join("Cargo.toml").is_file() {
        return None;
    }
    // Pre-built binary first (no toolchain needed).
    for profile in ["release", "debug"] {
        let exe = dir.join("target").join(profile).join(format!("{host}.exe"));
        if exe.is_file() {
            println!("🎨 Running {} (native host with backend)...", host.bold());
            return Some(spawn_host(&exe.to_string_lossy(), file));
        }
    }
    // Else build + run via cargo (needs the mocida C lib + clang).
    println!(
        "{}",
        format!("ℹ️  Building native host '{host}' with cargo (first run)...").dimmed()
    );
    Some(cargo_run_host(&dir, file))
}

/// Spawn a native host executable, passing the `.mui` path so it renders the
/// right file. Inherits stdio.
fn spawn_host(bin: &str, file: &str) -> i32 {
    vlog(&format!("exec {bin} {file}"));
    let status = Command::new(bin)
        .arg(file)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    match status {
        Ok(s) => s.code().unwrap_or(0),
        Err(e) => {
            eprintln!("❌ Failed to launch native host ('{bin}'): {e}");
            1
        }
    }
}

/// `cargo run --release -- <file>` in the host crate dir, with the mocida build
/// env applied (so `mocida-sys`'s bindgen + link find the C headers/lib).
fn cargo_run_host(dir: &Path, file: &str) -> i32 {
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--release")
        .arg("--")
        .arg(file)
        .current_dir(dir)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    if let Some(ws) = find_mocida_rs() {
        crate::cforge::apply_mocida_env(&mut cmd, &ws);
    }
    match cmd.status() {
        Ok(s) => s.code().unwrap_or(0),
        Err(e) => {
            eprintln!("❌ Failed to build/run native host: {e}");
            1
        }
    }
}

/// Spawn a `mui-dev` executable on `file`, inheriting stdio so its logs and
/// the mocida window come through. Returns the exit code.
fn spawn_binary(bin: &str, file: &str) -> i32 {
    vlog(&format!("exec {bin} {file}"));
    let status = Command::new(bin)
        .arg(file)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    match status {
        Ok(s) => s.code().unwrap_or(0),
        Err(e) => {
            eprintln!("❌ Failed to launch mui-dev ('{bin}'): {e}");
            1
        }
    }
}

/// `cargo run -p mui-dev -- <file>` inside the mocida-rs workspace.
fn cargo_run_mui_dev(workspace: &Path, file: &str) -> i32 {
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--quiet")
        .arg("-p")
        .arg("mui-dev")
        .arg("--")
        .arg(file)
        .current_dir(workspace)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    // Point mocida-sys at the C source headers + built lib (cross-platform), so
    // a first-time mui-dev build doesn't bind against a stale installed mocida.
    crate::cforge::apply_mocida_env(&mut cmd, workspace);
    // On macOS/Linux the mui-dev binary links the *dynamic* libmocida via
    // `@rpath`/soname but has no rpath baked in, so put the mocida lib dir on
    // the OS loader path for the run (Windows stages DLLs beside the exe).
    if let Some(lib) = mocida_runtime_lib_dir(workspace) {
        let (var, sep) = runtime_lib_var();
        let prev = std::env::var(var).unwrap_or_default();
        let val = if prev.is_empty() {
            lib.to_string_lossy().into_owned()
        } else {
            format!("{}{sep}{prev}", lib.display())
        };
        cmd.env(var, val);
    }
    let status = cmd.status();
    match status {
        Ok(s) => s.code().unwrap_or(0),
        Err(e) => {
            eprintln!("❌ Failed to run `cargo run -p mui-dev`: {e}");
            1
        }
    }
}

/// True if the workspace actually contains a buildable `mui-dev` host crate
/// (a `mui-dev/Cargo.toml` member). When false, `cforge run` can't render live
/// and falls back to the codegen build+run path.
fn workspace_has_mui_dev(workspace: &Path) -> bool {
    workspace.join("mui-dev").join("Cargo.toml").is_file()
}

/// Fallback for `cforge run <file>.mui` when no live `mui-dev` host exists:
/// run the codegen path (generate Rust → native release build) and then launch
/// the produced binary, with the mocida shared lib on the OS loader path so the
/// dynamic `libmocida` (+ SDL3) resolve at startup. Returns the binary's exit
/// code (or the build's, on failure).
fn codegen_build_and_run(file: &str, workspace: &Path) -> i32 {
    let rc = compile(file, None, /*do_build=*/ true, /*embed=*/ false);
    if rc != 0 {
        return rc;
    }
    let out_crate = PathBuf::from("./dist").join("mui");
    let name = crate_bin_name(&out_crate.join("Cargo.toml")).unwrap_or_else(|| "mui_app".into());
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.clone()
    };
    let binary = out_crate.join("target").join("release").join(&exe);
    if !binary.is_file() {
        eprintln!("❌ built binary not found: {}", binary.display());
        return 1;
    }
    // Absolute paths: `current_dir` below changes the cwd, so a relative
    // program path would be re-resolved against it and not be found.
    let binary = std::fs::canonicalize(&binary).unwrap_or(binary);
    let run_dir = std::fs::canonicalize(&out_crate).unwrap_or(out_crate);

    let mut cmd = Command::new(&binary);
    cmd.current_dir(&run_dir)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    if let Some(lib) = mocida_runtime_lib_dir(workspace) {
        let (var, sep) = runtime_lib_var();
        let prev = std::env::var(var).unwrap_or_default();
        let val = if prev.is_empty() {
            lib.to_string_lossy().into_owned()
        } else {
            format!("{}{sep}{prev}", lib.display())
        };
        cmd.env(var, val);
    }
    println!("🚀 Running {}...", name.bold());
    match cmd.status() {
        Ok(s) => s.code().unwrap_or(0),
        Err(e) => {
            eprintln!("❌ failed to launch {}: {e}", binary.display());
            1
        }
    }
}

/// Read the package `name = "…"` from a generated crate's Cargo.toml.
fn crate_bin_name(cargo_toml: &Path) -> Option<String> {
    let s = std::fs::read_to_string(cargo_toml).ok()?;
    for line in s.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("name =") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// The mocida shared-lib directory to put on the OS loader path at run-time.
/// Prefers the staged SDK (`mocida/release/stage/lib`), matching the dylib the
/// codegen build linked against.
fn mocida_runtime_lib_dir(workspace: &Path) -> Option<PathBuf> {
    let lib = workspace
        .parent()?
        .join("mocida")
        .join("release")
        .join("stage")
        .join("lib");
    lib.is_dir().then_some(lib)
}

/// Env var + path separator the OS uses to find shared libs at run-time.
fn runtime_lib_var() -> (&'static str, char) {
    if cfg!(windows) {
        ("PATH", ';')
    } else if cfg!(target_os = "macos") {
        ("DYLD_FALLBACK_LIBRARY_PATH", ':')
    } else {
        ("LD_LIBRARY_PATH", ':')
    }
}

/// Find a pre-built `mui-dev` executable under `<workspace>/target/{debug,
/// release}`. Picks whichever is newer so a fresh `--release` build wins.
fn prebuilt_mui_dev(workspace: &Path) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "mui-dev.exe"
    } else {
        "mui-dev"
    };
    let candidates = [
        workspace.join("target/release").join(exe),
        workspace.join("target/debug").join(exe),
    ];
    candidates
        .into_iter()
        .filter(|p| p.is_file())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// Locate the `mocida-rs` Cargo workspace. Checks `MOCIDA_RS_DIR`, then a few
/// layouts relative to cforge's install dir and the current directory.
fn find_mocida_rs() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("MOCIDA_RS_DIR") {
        let p = PathBuf::from(dir);
        if is_mocida_rs(&p) {
            return Some(p);
        }
    }

    let mut roots: Vec<PathBuf> = Vec::new();
    // The cforge install / build dir (COPPER_PATH) and its ancestors.
    if let Ok(cp) = std::env::var("COPPER_PATH") {
        roots.push(PathBuf::from(cp));
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
        }
    }

    // For each root, try `../mocida/mocida-rs` (repos side by side) and a few
    // ancestor combinations.
    for root in roots {
        let mut cur: Option<&Path> = Some(root.as_path());
        let mut hops = 0;
        while let Some(dir) = cur {
            for rel in [
                "mocida-rs",
                "mocida/mocida-rs",
                "../mocida/mocida-rs",
                "../../mocida/mocida-rs",
            ] {
                let cand = dir.join(rel);
                if is_mocida_rs(&cand) {
                    return std::fs::canonicalize(&cand).ok().map(|p| {
                        // Keep a clean path for display / cargo cwd.
                        PathBuf::from(strip_unc(&p))
                    });
                }
            }
            cur = dir.parent();
            hops += 1;
            if hops > 6 {
                break;
            }
        }
    }
    None
}

/// A directory looks like the mocida-rs workspace if it has a Cargo.toml that
/// declares the `mocida-sys` member. (The codegen path only needs the `mocida`
/// crate; the live-render path additionally wants `mui-dev`, but that member is
/// not always present — validating on `mui-dev` wrongly rejected real
/// workspaces and blocked `cforge -c` too.)
fn is_mocida_rs(dir: &Path) -> bool {
    let manifest = dir.join("Cargo.toml");
    match std::fs::read_to_string(&manifest) {
        Ok(s) => s.contains("mocida-sys"),
        Err(_) => false,
    }
}

/// Strip the Windows `\\?\` UNC prefix that `canonicalize` adds, so paths
/// display cleanly and work as a process cwd.
fn strip_unc(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
}

fn vlog(msg: &str) {
    if std::env::var("CFORGE_VERBOSE").as_deref() == Ok("1") {
        println!("   {} {}", "mui:".dimmed(), msg.dimmed());
    }
}
