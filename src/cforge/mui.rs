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
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

    // Host-backed `.mui`: when `App() { host: "..." }` names a native host crate,
    // its hand-written reducer/tick-loop is the real implementation. Static
    // codegen can't reproduce that (it renders `for`/`if` as one pass), so build
    // the host crate as the deliverable instead. Done before the `views` check so
    // a host-only file (no standalone `view`) still works.
    let host = doc.app.as_ref().and_then(|a| a.host.clone());
    if let Some(host) = host {
        // Resolve the host crate dir the same way run_native_host does.
        let dir = Path::new(file)
            .parent()
            .map(|p| p.join(&host))
            .unwrap_or_else(|| PathBuf::from(&host));
        if !dir.join("Cargo.toml").is_file() && !dir.join("properties.kson").is_file() {
            // Declared but missing: warn and fall through to static codegen.
            eprintln!(
                "{}",
                format!(
                    "⚠️  host crate '{host}' declared but not found at {}; falling back to static codegen",
                    dir.display()
                )
                .yellow()
            );
        } else {
            return compile_host(file, &dir, &host, output_dir, do_build);
        }
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
        // Without --bundle, the assets live alongside the crate root but the
        // binary is two levels deeper (target/release/). Copy every staged
        // file there so SDL_GetBasePath() finds them at runtime.
        if !embed {
            let bin_dir = out_root.join("target").join("release");
            for rel in &staged {
                let src = out_root.join(rel);
                let dst = bin_dir.join(rel);
                if let Some(p) = dst.parent() {
                    let _ = std::fs::create_dir_all(p);
                }
                let _ = std::fs::copy(&src, &dst);
            }
        }
        0
    } else {
        1
    }
}

/// Build a host-backed `.mui`'s native host crate as the codegen deliverable.
///
/// When a `.mui` declares `App() { host: "..." }`, the host crate (which already
/// embeds its own `.mui` + assets) *is* the working program — its reducer and
/// tick-loop can't be reproduced by static AST lowering. So `cforge -c` on such a
/// file builds that crate instead of generating Rust. With `-r`/`--release` it
/// compiles the host to release and stages a self-contained bundle (exe + the
/// native DLLs mocida needs) under `<output>/mui/`. Returns the exit code.
fn compile_host(
    file: &str,
    dir: &Path,
    host: &str,
    output_dir: Option<&str>,
    do_build: bool,
) -> i32 {
    crate::cforge::pretty::head("Embedded host build");
    println!(
        "📦 {} declares host {} — building that crate (it embeds the .mui + assets).",
        Path::new(file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(file)
            .bold(),
        host.bold()
    );

    if !do_build {
        // Plain `-c`: don't compile, just point at the host crate (mirrors how the
        // static path stops after codegen). `-r`/`--release` builds the binary.
        println!("   {}", dir.display());
        println!(
            "{}",
            "ℹ️  Add -r/--release to build it into a self-contained binary.".dimmed()
        );
        return 0;
    }

    // `-c -r`: build the host crate in release. If the host is a Copper package
    // (`properties.kson` without `Cargo.toml`), transpile it into `dist/rust` first.
    let cargo_dir = if !dir.join("Cargo.toml").is_file() && dir.join("properties.kson").is_file() {
        crate::cforge::ensure_copper_dep_built(dir);
        dir.join("dist").join("rust")
    } else {
        dir.to_path_buf()
    };
    println!();
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("--release").current_dir(&cargo_dir);
    if let Some(ws) = find_mocida_rs() {
        crate::cforge::apply_mocida_env(&mut cmd, &ws);
    }
    let (ok, _) = crate::cforge::cargo_with_spinner(
        cmd,
        &format!("building {host} (release)"),
        &format!("Built {host}"),
    );
    if !ok {
        return 1;
    }

    // Resolve the produced executable (crate's [[bin]]/package name, else host).
    let bin_name =
        crate_bin_name(&cargo_dir.join("Cargo.toml")).unwrap_or_else(|| host.to_string());
    let exe_name = if cfg!(windows) {
        format!("{bin_name}.exe")
    } else {
        bin_name.clone()
    };
    let release_dir = cargo_dir.join("target").join("release");
    let exe = release_dir.join(&exe_name);

    // Stage a self-contained bundle under <output>/mui/. The .mui + assets are
    // baked into the exe, but the native DLLs (mocida.dll/SDL3.dll, staged beside
    // the exe by mocida-sys) are not — copy them next to the deliverable too.
    let out_dir = PathBuf::from(output_dir.unwrap_or("./dist")).join("mui");
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("❌ cannot create {}: {e}", out_dir.display());
        return 1;
    }
    let dest_exe = out_dir.join(&exe_name);
    if let Err(e) = std::fs::copy(&exe, &dest_exe) {
        eprintln!(
            "❌ cannot copy {} → {}: {e}",
            exe.display(),
            dest_exe.display()
        );
        return 1;
    }
    // Copy every *.dll staged beside the host exe (case-insensitive), like
    // mocida-sys's stage_runtime_dlls does for the build output.
    if let Ok(entries) = std::fs::read_dir(&release_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let is_dll = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("dll"))
                .unwrap_or(false);
            if is_dll {
                if let Some(name) = p.file_name() {
                    let _ = std::fs::copy(&p, out_dir.join(name));
                }
            }
        }
    }

    crate::cforge::pretty::ok(&format!(
        "Self-contained host binary: {}",
        dest_exe.display()
    ));
    0
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

/// Build a shippable web package for a `.mui` / `.crm`.
///
/// Writes `<output>/<stem>_web/` with the wasm host, generated `boot.js` /
/// `index.html`, the entry + import graph, and fonts/media referenced by the
/// document. `release` prefers the release wasm build when available.
///
/// Example: `cforge build app.mui --web --release` → `dist/app_web/`.
pub fn build_web(file: &str, output_dir: Option<&str>, release: bool) -> i32 {
    let stem = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app");
    let pkg_name = format!("{stem}_web");
    let out = PathBuf::from(output_dir.unwrap_or("./dist")).join(&pkg_name);

    crate::cforge::pretty::head(&format!(
        "Web package ({})",
        if release { "release" } else { "debug" }
    ));

    if out.exists() {
        if let Err(e) = std::fs::remove_dir_all(&out) {
            eprintln!("❌ cannot clean {}: {e}", out.display());
            return 1;
        }
    }
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("❌ cannot create {}: {e}", out.display());
        return 1;
    }

    match prepare_web_package(file, &out, release) {
        Ok(staged) => {
            let import_count = staged
                .files
                .iter()
                .filter(|f| f.as_str() != "/app.mui" && f.as_str() != staged.entry_vfs.as_str())
                .count();
            if import_count > 0 {
                println!(
                    "📦 Staged {} → {} (+{} import file(s))",
                    file.bold(),
                    staged.entry_vfs.cyan(),
                    import_count
                );
            }
            for w in &staged.warnings {
                eprintln!("{} {}", "⚠️ ".yellow(), w);
            }
            // Tiny local preview helper (not required for deployment).
            write_web_serve_helper(&out, &pkg_name);
            crate::cforge::pretty::ok(&format!("Web package: {}", out.display()));
            crate::cforge::pretty::tree(
                &pkg_name,
                &[
                    "index.html".into(),
                    "build-gate.js".into(),
                    "build-id.json".into(),
                    "boot.js".into(),
                    "sw.js".into(),
                    "mui-dev.js".into(),
                    "mui_dev.wasm".into(),
                    "app.mui".into(),
                    "manifest.json".into(),
                ],
            );
            0
        }
        Err(e) => {
            eprintln!("❌ Web build failed: {e}");
            let _ = std::fs::remove_dir_all(&out);
            1
        }
    }
}

/// Serve a `.mui` / `.crm` as a browser site via the prebuilt Emscripten
/// `mui-dev` host (`cforge run --listen[=PORT] app.mui`).
///
/// Fully automated: stages the wasm shell, the entry + `import` graph, fonts /
/// media referenced by the `.mui`, then **always** emits canonical
/// `boot.js` + `index.html` + `sw.js` (no hand-maintained host / JS / Python).
pub fn run_listen(file: &str, port: u16) -> i32 {
    let stage = std::env::temp_dir().join(format!("cforge-mui-listen-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&stage) {
        eprintln!("❌ Could not create listen stage dir: {e}");
        return 1;
    }

    let staged = match prepare_web_package(file, &stage, false) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("❌ {e}");
            let _ = std::fs::remove_dir_all(&stage);
            return 1;
        }
    };

    let import_count = staged
        .files
        .iter()
        .filter(|f| f.as_str() != "/app.mui" && f.as_str() != staged.entry_vfs.as_str())
        .count();
    if import_count > 0 {
        println!(
            "📦 Staged {} → {} (+{} import file(s))",
            file.bold(),
            staged.entry_vfs.cyan(),
            import_count
        );
    }
    for w in &staged.warnings {
        eprintln!("{} {}", "⚠️ ".yellow(), w);
    }

    println!(
        "🌐 Serving {} at {} (Ctrl-C to stop)",
        file.bold(),
        format!("http://127.0.0.1:{port}/").cyan()
    );
    vlog(&format!("stage = {}", stage.display()));
    vlog(&format!("entry vfs = {}", staged.entry_vfs));
    vlog(&format!("vfs files = {:?}", staged.files));

    let code = serve_mui_listen(&stage, port);
    let _ = std::fs::remove_dir_all(&stage);
    code
}

/// Stage a complete browser package into `out` (wasm host + generated boot +
/// project sources + fonts/media). Shared by `build_web` and `run_listen`.
fn prepare_web_package(
    file: &str,
    out: &Path,
    prefer_release: bool,
) -> Result<StagedMui, String> {
    let abs = std::fs::canonicalize(file)
        .map(|p| strip_unc(&p))
        .unwrap_or_else(|_| file.to_string());
    let web = find_mui_web_dir(prefer_release).ok_or_else(|| {
        "Could not find the MUI WASM site host (mui-dev.js + mui_dev.wasm).\n\
         Set MUI_WEB_DIR, or place OndaEngine/site/www next to copper-lang / mocida."
            .to_string()
    })?;

    for name in ["mui-dev.js", "mui_dev.wasm", "mui_dev.data", "logo.png"] {
        let src = web.join(name);
        if src.is_file() {
            std::fs::copy(&src, out.join(name))
                .map_err(|e| format!("Failed to stage {}: {e}", src.display()))?;
            // Ship precompressed siblings when present (mobile Safari).
            let br = PathBuf::from(format!("{}.br", src.display()));
            if br.is_file() {
                let _ = std::fs::copy(&br, out.join(format!("{name}.br")));
            }
        } else if matches!(name, "mui-dev.js" | "mui_dev.wasm") {
            return Err(format!("{name} is missing from {}", web.display()));
        }
    }

    let staged = stage_mui_import_graph(Path::new(&abs), out)?;

    let disk_sources: Vec<PathBuf> = match mui_syntax::loader::load(Path::new(&abs)) {
        Ok(loaded) => loaded.sources,
        Err(_) => vec![canon_path(Path::new(&abs))],
    };
    let entry_canon = canon_path(Path::new(&abs));
    let project_root = mui_project_root(&entry_canon, &disk_sources);
    let (font_vfs, media_vfs) = collect_mui_asset_refs(&disk_sources);

    stage_listen_fonts(&web, out, &project_root, &font_vfs)?;
    stage_listen_media(&web, out, &project_root, &media_vfs)?;

    // Scene3D / Sprite3D read textures from MEMFS — put media paths on the
    // manifest so listen_boot.js can fetchIntoVfs before first paint.
    {
        let mut files = staged.files.clone();
        for m in &media_vfs {
            push_unique(&mut files, m.clone());
        }
        let manifest = serde_json::json!({
            "entry": staged.entry_vfs,
            "files": files,
            "media": media_vfs,
        });
        std::fs::write(
            out.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }

    let stamp = compute_build_stamp(out)?;
    let mut early: Vec<String> = Vec::new();
    for f in &font_vfs {
        push_unique(&mut early, f.clone());
    }
    early.push("/manifest.json".to_string());
    early.push("/build-id.json".to_string());
    for f in &staged.files {
        push_unique(&mut early, f.clone());
    }
    for m in &media_vfs {
        push_unique(&mut early, m.clone());
    }
    let title = mui_app_title(&abs);
    write_listen_host(out, &stamp, &title, &font_vfs, &early)?;

    vlog(&format!("web host = {}", web.display()));
    vlog(&format!("build_id = {}  built_at = {}", stamp.id, stamp.built_at_iso));
    Ok(staged)
}

fn write_web_serve_helper(out: &Path, pkg_name: &str) {
    // Auto-emitted by cforge — never hand-edited. Prefer `cforge run --listen`
    // for day-to-day; this is only a static preview of the built package.
    let script = format!(
        "#!/usr/bin/env python3\n\
         # Generated by cforge build --web — do not edit.\n\
         import os\n\
         from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler\n\
         os.chdir(os.path.dirname(os.path.abspath(__file__)) or '.')\n\
         print('Serving {pkg} at http://127.0.0.1:3000/ (Ctrl-C to stop)')\n\
         print('Tip: day-to-day use `cforge run --listen your.mui` instead.')\n\
         ThreadingHTTPServer(('0.0.0.0', 3000), SimpleHTTPRequestHandler).serve_forever()\n",
        pkg = pkg_name
    );
    let path = out.join("serve.py");
    if std::fs::write(&path, script).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
    }
}

struct StagedMui {
    entry_vfs: String,
    files: Vec<String>,
    warnings: Vec<String>,
}

fn canon_path(p: &Path) -> PathBuf {
    std::fs::canonicalize(p)
        .map(|p| PathBuf::from(strip_unc(&p)))
        .unwrap_or_else(|_| p.to_path_buf())
}

/// Smallest directory that contains the entry and every successfully-loaded
/// import — so `import { X } from "../card/card.mui"` keeps the same relative
/// layout under the VFS as on disk.
fn mui_project_root(entry: &Path, sources: &[PathBuf]) -> PathBuf {
    let mut root = entry
        .parent()
        .map(canon_path)
        .unwrap_or_else(|| PathBuf::from("."));
    for src in sources {
        let src = canon_path(src);
        while !src.starts_with(&root) {
            match root.parent() {
                Some(parent) if parent != root => root = parent.to_path_buf(),
                _ => break,
            }
        }
    }
    root
}

fn vfs_path(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.is_empty() || s.contains("..") {
        return None;
    }
    Some(format!("/{s}"))
}

/// Stage the entry + import graph under `stage/` with the same relative layout
/// as on disk. Writes `manifest.json` so the WASM host opens the real entry
/// path (not a flattened `/app.mui`), matching desktop `import` resolution.
fn stage_mui_import_graph(entry: &Path, stage: &Path) -> Result<StagedMui, String> {
    let loaded = mui_syntax::loader::load(entry).map_err(|e| e.to_string())?;
    let entry_canon = canon_path(entry);
    let root = mui_project_root(&entry_canon, &loaded.sources);
    let entry_vfs = vfs_path(&root, &entry_canon)
        .ok_or_else(|| format!("entry {} is outside project root", entry.display()))?;

    let mut files: Vec<String> = Vec::new();
    for src in &loaded.sources {
        let src_canon = canon_path(src);
        let Some(vfs) = vfs_path(&root, &src_canon) else {
            continue;
        };
        // Only ship MUI/CRM sources into the browser VFS (Copper/Rust imports
        // are native-host concerns and are skipped with a warning).
        let ext = src_canon
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "mui" | "crm") {
            continue;
        }
        let rel = src_canon
            .strip_prefix(&root)
            .map_err(|_| format!("{} escapes project root", src.display()))?;
        let dest = stage.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(&src_canon, &dest).map_err(|e| format!("{}: {e}", src.display()))?;
        if !files.iter().any(|f| f == &vfs) {
            files.push(vfs);
        }
    }
    if !files.iter().any(|f| f == &entry_vfs) {
        let rel = entry_canon
            .strip_prefix(&root)
            .map_err(|_| "entry escapes project root".to_string())?;
        let dest = stage.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(&entry_canon, &dest).map_err(|e| e.to_string())?;
        files.insert(0, entry_vfs.clone());
    }

    // Compatibility alias for older boots that only fetch `/app.mui`.
    std::fs::copy(&entry_canon, stage.join("app.mui")).map_err(|e| e.to_string())?;
    if !files.iter().any(|f| f == "/app.mui") {
        files.push("/app.mui".to_string());
    }

    let manifest = serde_json::json!({
        "entry": entry_vfs,
        "files": files,
    });
    std::fs::write(
        stage.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    Ok(StagedMui {
        entry_vfs,
        files,
        warnings: loaded.warnings,
    })
}

/// Canonical browser host — always regenerated by `run_listen` / `build --web`
/// (never hand-edited). Placeholders: `__ASSET_VER__` / `__BUILD_ID__`,
/// `__BUILT_AT__`, `__APP_TITLE__`, `__EARLY_FILES__`, `__FONT_FILES__`.
const LISTEN_INDEX_HTML: &str = include_str!("listen_index.html");
const LISTEN_BOOT_JS: &str = include_str!("listen_boot.js");
const LISTEN_SW_JS: &str = include_str!("listen_sw.js");
const LISTEN_BUILD_GATE_JS: &str = include_str!("listen_build_gate.js");

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.iter().any(|v| v == &value) {
        list.push(value);
    }
}

/// Content fingerprint of the staged web app. Changes only when wasm / JS /
/// `.mui` bytes change — used as Cache Storage key + `?v=` bust token.
struct BuildStamp {
    id: String,
    built_at: u64,
    built_at_iso: String,
}

fn hash_file(hasher: &mut Sha256, path: &Path) {
    if let Ok(bytes) = std::fs::read(path) {
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
}

fn compute_build_stamp(stage: &Path) -> Result<BuildStamp, String> {
    let mut hasher = Sha256::new();
    // Runtime binary + glue — the shipped "app build".
    hash_file(&mut hasher, &stage.join("mui_dev.wasm"));
    hash_file(&mut hasher, &stage.join("mui-dev.js"));
    // Project sources staged beside them (entry + imports), any depth.
    let mut mui_files: Vec<PathBuf> = Vec::new();
    for ent in walkdir::WalkDir::new(stage)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = ent.path();
        if !p.is_file() {
            continue;
        }
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "mui" || ext == "crm" {
            mui_files.push(p.to_path_buf());
        }
    }
    mui_files.sort();
    for p in &mui_files {
        hash_file(&mut hasher, p);
    }
    let digest = hasher.finalize();
    // 16 hex chars = 64 bits — enough to detect code changes, short for URLs.
    let id = digest
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let built_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let built_at_iso = unix_secs_to_rfc3339(built_at);
    Ok(BuildStamp {
        id,
        built_at,
        built_at_iso,
    })
}

/// Minimal UTC RFC3339 (`YYYY-MM-DDTHH:MM:SSZ`) without pulling chrono.
fn unix_secs_to_rfc3339(mut secs: u64) -> String {
    const DAY: u64 = 86400;
    let days = secs / DAY;
    secs %= DAY;
    let hh = secs / 3600;
    let mm = (secs % 3600) / 60;
    let ss = secs % 60;
    // Civil date from days since 1970-01-01 (Howard Hinnant algorithm).
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn js_string_array(paths: &[String]) -> String {
    let parts: Vec<String> = paths
        .iter()
        .map(|p| format!("\"{}\"", p.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!("[{}]", parts.join(","))
}

fn write_listen_host(
    stage: &Path,
    stamp: &BuildStamp,
    app_title: &str,
    fonts: &[String],
    early: &[String],
) -> Result<(), String> {
    let title = if app_title.trim().is_empty() {
        "MUI"
    } else {
        app_title.trim()
    };
    let title_esc = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    let asset_ver = stamp.id.as_str();
    let boot = LISTEN_BOOT_JS
        .replace("__ASSET_VER__", asset_ver)
        .replace("__BUILD_ID__", asset_ver)
        .replace("__BUILT_AT__", &stamp.built_at_iso)
        .replace("__EARLY_FILES__", &js_string_array(early))
        .replace("__FONT_FILES__", &js_string_array(fonts));
    let gate = LISTEN_BUILD_GATE_JS
        .replace("__BUILD_ID__", asset_ver)
        .replace("__BUILT_AT__", &stamp.built_at_iso);
    let index = LISTEN_INDEX_HTML
        .replace("__ASSET_VER__", asset_ver)
        .replace("__BUILD_ID__", asset_ver)
        .replace("__BUILT_AT__", &stamp.built_at_iso)
        .replace("__APP_TITLE__", &title_esc);
    let sw = LISTEN_SW_JS
        .replace("__ASSET_VER__", asset_ver)
        .replace("__BUILD_ID__", asset_ver)
        .replace("__BUILT_AT__", &stamp.built_at_iso);
    let build_id_json = format!(
        "{{\n  \"id\": \"{}\",\n  \"builtAt\": {},\n  \"builtAtIso\": \"{}\"\n}}\n",
        stamp.id, stamp.built_at, stamp.built_at_iso
    );
    std::fs::write(stage.join("boot.js"), boot).map_err(|e| e.to_string())?;
    std::fs::write(stage.join("build-gate.js"), gate).map_err(|e| e.to_string())?;
    std::fs::write(stage.join("index.html"), index).map_err(|e| e.to_string())?;
    std::fs::write(stage.join("sw.js"), sw).map_err(|e| e.to_string())?;
    std::fs::write(stage.join("build-id.json"), build_id_json).map_err(|e| e.to_string())?;
    Ok(())
}

/// `App() { title: "..." }` → browser `<title>` / a11y label.
fn mui_app_title(file: &str) -> String {
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return "MUI".into(),
    };
    mui_syntax::parse(&src)
        .app
        .and_then(|a| a.title)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "MUI".into())
}

/// Scan `.mui` sources for `FontFace(..., "path")`, `Image("path"...)`,
/// and `Sprite3D("path"...)` (Scene3D loads from MEMFS, not HTTP fetch).
fn collect_mui_asset_refs(sources: &[PathBuf]) -> (Vec<String>, Vec<String>) {
    let font_re = regex::Regex::new(
        r#"FontFace\s*\(\s*"[^"]*"\s*,\s*"([^"]+\.(?:ttf|otf|woff2?))"\s*\)"#,
    )
    .expect("font regex");
    let image_re =
        regex::Regex::new(r#"Image\s*\(\s*"([^"]+\.(?:png|jpe?g|webp|gif|svg))"\s*"#)
            .expect("image regex");
    let sprite_re =
        regex::Regex::new(r#"Sprite3D\s*\(\s*"([^"]+\.(?:png|jpe?g|webp|gif|svg))"\s*"#)
            .expect("sprite regex");

    let mut fonts: Vec<String> = Vec::new();
    let mut media: Vec<String> = Vec::new();
    for src in sources {
        let Ok(text) = std::fs::read_to_string(src) else {
            continue;
        };
        for cap in font_re.captures_iter(&text) {
            let path = normalize_asset_vfs(&cap[1]);
            push_unique(&mut fonts, path);
        }
        for cap in image_re.captures_iter(&text) {
            let path = normalize_asset_vfs(&cap[1]);
            push_unique(&mut media, path);
        }
        for cap in sprite_re.captures_iter(&text) {
            let path = normalize_asset_vfs(&cap[1]);
            push_unique(&mut media, path);
        }
    }
    if fonts.is_empty() {
        fonts.push("/fonts/Inter-Regular.ttf".to_string());
        fonts.push("/fonts/Inter-Bold.ttf".to_string());
    }
    (fonts, media)
}

fn normalize_asset_vfs(raw: &str) -> String {
    let s = raw.trim().replace('\\', "/");
    if s.starts_with('/') {
        s
    } else {
        format!("/{s}")
    }
}

fn mocida_fonts_dir() -> Option<PathBuf> {
    let ws = find_mocida_rs()?;
    // mocida-rs lives beside `mocida/` → `../mocida/fonts`
    let cand = ws.parent()?.join("mocida").join("fonts");
    if cand.is_dir() {
        return Some(cand);
    }
    let cand = ws.join("..").join("mocida").join("fonts");
    cand.canonicalize().ok().filter(|p| p.is_dir())
}

fn resolve_staged_asset(
    vfs: &str,
    web: &Path,
    stage: &Path,
    project_root: &Path,
) -> Option<PathBuf> {
    let rel = vfs.trim_start_matches('/');
    if rel.is_empty() || rel.contains("..") {
        return None;
    }
    let candidates = [
        stage.join(rel),
        web.join(rel),
        project_root.join(rel),
        project_root.join(Path::new(rel).file_name().unwrap_or_default()),
    ];
    for c in &candidates {
        if c.is_file() {
            return Some(c.clone());
        }
    }
    if let Some(name) = Path::new(rel).file_name() {
        if let Some(fonts) = mocida_fonts_dir() {
            let c = fonts.join(name);
            if c.is_file() {
                return Some(c);
            }
        }
        let web_font = web.join("fonts").join(name);
        if web_font.is_file() {
            return Some(web_font);
        }
    }
    None
}

fn stage_listen_fonts(
    web: &Path,
    stage: &Path,
    project_root: &Path,
    fonts: &[String],
) -> Result<(), String> {
    // Copy whole web/fonts tree when present (covers defaults).
    let fonts_src = web.join("fonts");
    if fonts_src.is_dir() {
        copy_dir_recursive(&fonts_src, &stage.join("fonts")).map_err(|e| e.to_string())?;
    }
    for vfs in fonts {
        let dest = stage.join(vfs.trim_start_matches('/'));
        if dest.is_file() {
            continue;
        }
        let Some(src) = resolve_staged_asset(vfs, web, stage, project_root) else {
            eprintln!(
                "{} font {} not found — FontFace may fail in browser",
                "⚠️ ".yellow(),
                vfs
            );
            continue;
        };
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(&src, &dest).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    Ok(())
}

fn stage_listen_media(
    web: &Path,
    stage: &Path,
    project_root: &Path,
    media: &[String],
) -> Result<(), String> {
    // Optional media/ from the web host (screenshots, logo).
    let media_src = web.join("media");
    if media_src.is_dir() {
        let _ = copy_dir_recursive(&media_src, &stage.join("media"));
    }
    for vfs in media {
        let dest = stage.join(vfs.trim_start_matches('/'));
        if dest.is_file() {
            continue;
        }
        let Some(src) = resolve_staged_asset(vfs, web, stage, project_root) else {
            eprintln!(
                "{} image {} not found — Image() may fail in browser",
                "⚠️ ".yellow(),
                vfs
            );
            continue;
        };
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(&src, &dest).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    Ok(())
}

fn find_mui_web_dir(prefer_release: bool) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("MUI_WEB_DIR") {
        let p = PathBuf::from(dir);
        if web_dir_ok(&p) {
            return Some(p);
        }
    }

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
        }
    }
    if let Some(ws) = find_mocida_rs() {
        roots.push(ws.clone());
        if let Some(parent) = ws.parent() {
            roots.push(parent.to_path_buf());
            if let Some(gp) = parent.parent() {
                roots.push(gp.to_path_buf());
            }
        }
    }

    // Prefer the matching emscripten profile for release/debug builds.
    if let Some(ws) = find_mocida_rs() {
        let profiles = if prefer_release {
            ["release", "debug"]
        } else {
            ["debug", "release"]
        };
        for profile in profiles {
            let cand = ws
                .join("target")
                .join("wasm32-unknown-emscripten")
                .join(profile);
            if web_dir_ok(&cand) {
                return Some(cand);
            }
        }
    }

    // Fall back to a full site shell (fonts/media often live here).
    let rels = [
        "site/www",
        "OndaEngine/site/www",
        "../OndaEngine/site/www",
        "../../OndaEngine/site/www",
        "www",
    ];
    for root in &roots {
        for rel in rels {
            let cand = root.join(rel);
            if web_dir_ok(&cand) {
                return std::fs::canonicalize(&cand)
                    .ok()
                    .map(|p| PathBuf::from(strip_unc(&p)))
                    .or(Some(cand));
            }
        }
    }
    None
}

fn web_dir_ok(dir: &Path) -> bool {
    // `.data` preload pack is optional — fonts/images ship over HTTP.
    dir.join("mui-dev.js").is_file() && dir.join("mui_dev.wasm").is_file()
}

/// Map a request URL to a path relative to the listen stage. Allows the wasm
/// shell, `manifest.json`, `/app.mui`, and any nested `.mui` / `.crm` from the
/// staged import graph. Rejects `..` and unknown extensions.
fn listen_rel_path(url_path: &str) -> Option<String> {
    let path = url_path.split('?').next().unwrap_or("/");
    if path.contains('\0') || path.contains('\\') || path.split('/').any(|s| s == "..") {
        return None;
    }
    let rel = match path {
        "/" | "/index.html" => "index.html".to_string(),
        "/boot.js" => "boot.js".to_string(),
        "/mui-dev.js" => "mui-dev.js".to_string(),
        "/mui_dev.wasm" => "mui_dev.wasm".to_string(),
        "/mui_dev.data" => "mui_dev.data".to_string(),
        "/manifest.json" => "manifest.json".to_string(),
        "/build-id.json" => "build-id.json".to_string(),
        "/build-gate.js" => "build-gate.js".to_string(),
        "/app.mui" => "app.mui".to_string(),
        "/logo.png" | "/favicon.ico" => "logo.png".to_string(),
        "/sw.js" => "sw.js".to_string(),
        other if other.starts_with('/') => {
            let trimmed = other.trim_start_matches('/');
            if trimmed.is_empty() {
                return None;
            }
            let ext = Path::new(trimmed)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(
                ext.as_str(),
                "mui" | "crm" | "json" | "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg"
                    | "ttf" | "otf" | "woff" | "woff2" | "js" | "wasm" | "data" | "html"
            ) {
                trimmed.to_string()
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(rel)
}

fn serve_mui_listen(root: &Path, port: u16) -> i32 {
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::Arc;

    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("❌ Could not bind 0.0.0.0:{port}: {e}");
            return 1;
        }
    };
    let root = Arc::new(root.to_path_buf());
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let root = Arc::clone(&root);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let n = match stream.read(&mut buf) {
                Ok(n) if n > 0 => n,
                _ => return,
            };
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/");
            let path = path.split('?').next().unwrap_or("/");
            let rel = match listen_rel_path(path) {
                Some(r) => r,
                None => {
                    let _ = write_listen_response(&mut stream, 404, "text/plain", "no-store", b"");
                    vlog(&format!("404 {path}"));
                    return;
                }
            };
            let file = root.join(&rel);
            // Refuse anything that escapes the stage directory.
            let Ok(canon_root) = std::fs::canonicalize(root.as_path()) else {
                let _ = write_listen_response(&mut stream, 404, "text/plain", "no-store", b"");
                return;
            };
            let Ok(canon_file) = std::fs::canonicalize(&file) else {
                let _ = write_listen_response(&mut stream, 404, "text/plain", "no-store", b"");
                return;
            };
            if !canon_file.starts_with(&canon_root) || !canon_file.is_file() {
                let _ = write_listen_response(&mut stream, 404, "text/plain", "no-store", b"");
                return;
            }
            let data = match std::fs::read(&canon_file) {
                Ok(d) => d,
                Err(_) => {
                    let _ = write_listen_response(&mut stream, 404, "text/plain", "no-store", b"");
                    return;
                }
            };
            let ctype = match Path::new(&rel)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
            {
                "html" => "text/html; charset=utf-8",
                "js" => "text/javascript; charset=utf-8",
                "json" => "application/json; charset=utf-8",
                "wasm" => "application/wasm",
                "data" => "application/octet-stream",
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "gif" => "image/gif",
                "svg" => "image/svg+xml",
                "ttf" => "font/ttf",
                "otf" => "font/otf",
                "woff" => "font/woff",
                "woff2" => "font/woff2",
                "mui" | "crm" => "text/plain; charset=utf-8",
                _ => "application/octet-stream",
            };
            let cache = listen_cache_control(&rel);
            let _ = write_listen_response(&mut stream, 200, ctype, cache, &data);
        });
    }
    0
}

/// Versioned heavy assets can live in the browser / SW cache forever; shell
/// files stay revalidating so listen/rebuild picks up new boot/html/.mui.
fn listen_cache_control(rel: &str) -> &'static str {
    let name = Path::new(rel)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(rel);
    if name == "sw.js"
        || name == "index.html"
        || name == "build-id.json"
        || name == "build-gate.js"
        || rel == "index.html"
    {
        return "no-cache";
    }
    match Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
    {
        // Versioned boot.js / mui-dev.js stay immutable via ?v=BUILD_ID.
        "wasm" | "js" | "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg" | "ttf" | "otf"
        | "woff" | "woff2" | "data" => "public, max-age=31536000, immutable",
        "mui" | "crm" | "json" | "html" => "no-cache",
        _ => "no-cache",
    }
}

fn write_listen_response(
    stream: &mut impl std::io::Write,
    status: u16,
    content_type: &str,
    cache_control: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: {cache_control}\r\n\
         Content-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-eval' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; font-src 'self'; connect-src 'self'; worker-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    )?;
    stream.write_all(body)
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
    if !dir.join("Cargo.toml").is_file() && !dir.join("properties.kson").is_file() {
        return None;
    }
    if !dir.join("Cargo.toml").is_file() && dir.join("properties.kson").is_file() {
        return Some(cargo_run_host(&dir, file));
    }
    // Pre-built binary first (no toolchain needed), named by the crate's
    // [[bin]] (e.g. MocidadEngine), else the host dir.
    let bin = crate_bin_name(&dir.join("Cargo.toml")).unwrap_or_else(|| host.to_string());
    for profile in ["release", "debug"] {
        let exe = dir.join("target").join(profile).join(format!("{bin}.exe"));
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

/// Build + run the host crate in `dir`. Build is run through `cargo_with_spinner`
/// so cargo's verbose output is hidden; only the actual app gets inherited stdio.
fn cargo_run_host(dir: &Path, file: &str) -> i32 {
    let cargo_dir = if !dir.join("Cargo.toml").is_file() && dir.join("properties.kson").is_file() {
        crate::cforge::ensure_copper_dep_built(dir);
        dir.join("dist").join("rust")
    } else {
        dir.to_path_buf()
    };
    let host_name = crate_bin_name(&cargo_dir.join("Cargo.toml")).unwrap_or_else(|| {
        dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("host")
            .to_string()
    });

    let mut build_cmd = Command::new("cargo");
    build_cmd
        .arg("build")
        .arg("--release")
        .current_dir(&cargo_dir);
    if let Some(ws) = find_mocida_rs() {
        crate::cforge::apply_mocida_env(&mut build_cmd, &ws);
    }
    let (ok, _) = crate::cforge::cargo_with_spinner(
        build_cmd,
        &format!("building {host_name}"),
        &format!("Built {host_name}"),
    );
    if !ok {
        return 1;
    }

    let exe_name = if cfg!(windows) {
        format!("{host_name}.exe")
    } else {
        host_name.clone()
    };
    let binary = cargo_dir.join("target").join("release").join(&exe_name);
    spawn_host(&binary.to_string_lossy(), file)
}

/// Spawn a `mui-dev` executable on `file`, inheriting stdio so its logs and
/// the mocida window come through. Returns the exit code.
///
/// Hot-reload depends on this process staying alive after `cforge run`
/// returns from its `Command::status()` call (the mui-dev child runs the
/// per-frame tick callback that swaps the tree on file changes). On
/// macOS/Linux we also have to put the mocida lib dir on the loader
/// search path BEFORE spawn — `mui-dev` was built with
/// `-Wl,-rpath,<lib>`, but only when the most recent build saw a non-empty
/// `MOCIDA_LIB_DIR` env (see `apply_mocida_env`); a pre-built binary with a
/// stale or missing rpath crashes at startup with
/// "Library not loaded: @rpath/libmocida.dylib" and the user just sees a
/// broken window — they conclude hot-reload is "not working" because
/// nothing ever renders. Inject the same env var here so the loader finds
/// the dynamic lib even when the rpath is wrong.
fn spawn_binary(bin: &str, file: &str) -> i32 {
    vlog(&format!("exec {bin} {file}"));
    let mut cmd = Command::new(bin);
    cmd.arg(file)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    if let Some(ws) = find_mocida_rs() {
        if let Some(lib) = mocida_runtime_lib_dir(&ws) {
            let (var, sep) = runtime_lib_var();
            let prev = std::env::var(var).unwrap_or_default();
            let val = if prev.is_empty() {
                lib.to_string_lossy().into_owned()
            } else {
                format!("{}{sep}{prev}", lib.display())
            };
            cmd.env(var, val);
        }
    }
    let status = cmd.status();
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
    let mut section = "";
    let mut pkg_name = None;
    for line in s.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            section = t;
            continue;
        }
        if let Some(rest) = t.strip_prefix("name") {
            let rest = rest.trim_start();
            if let Some(val) = rest.strip_prefix('=') {
                let name = val.trim().trim_matches('"').to_string();
                if section == "[[bin]]" {
                    return Some(name);
                }
                if (section == "[package]" || section.is_empty()) && pkg_name.is_none() {
                    pkg_name = Some(name);
                }
            }
        }
    }
    pkg_name
}

/// The mocida shared-lib directory to put on the OS loader path at run-time.
///
/// Two candidates:
///   1. `mocida/release/stage/lib` — the staged SDK layout (matches the dylib
///      the codegen build linked against on most setups).
///   2. `mocida/build/`           — the developer's fresh `python build.py`
///      output, where a `libmocida.dylib` was produced without going through
///      the staging step.
///
/// Hot-reload runs `mui-dev`, which was linked against whichever dylib the
/// most recent `MOCIDA_LIB_DIR` pointed at. If the stage is stale (older
/// than the build/ tree — e.g. someone added a C export after the last
/// `python build.py --release --stage`), the runtime would load the stale
/// lib and crash with SEGV on the first new-symbol FFI call. Pick the newer
/// of the two so the loader finds the symbols the binary was linked with.
fn mocida_runtime_lib_dir(workspace: &Path) -> Option<PathBuf> {
    let mocida_c = workspace.parent()?.join("mocida");
    let stage = mocida_c.join("release").join("stage").join("lib");
    let build = mocida_c.join("build");
    let stage_newer = newer_libmocida(&stage, &build);
    let dir = if stage_newer == Some(true) {
        stage
    } else {
        build
    };
    if dir.join(mocida_lib_filename()).is_file() {
        Some(dir)
    } else {
        None
    }
}

/// Filename of the dynamic mocida lib on this OS.
fn mocida_lib_filename() -> &'static str {
    if cfg!(windows) {
        "mocida.dll"
    } else if cfg!(target_os = "macos") {
        "libmocida.dylib"
    } else {
        "libmocida.so"
    }
}

/// True when `a` is a directory containing a mocida lib whose mtime is
/// strictly newer than the mocida lib in `b` (or `b` has none). `None` when
/// neither dir carries a lib — caller falls back to whichever is non-empty.
fn newer_libmocida(a: &Path, b: &Path) -> Option<bool> {
    let a_mtime = std::fs::metadata(a.join(mocida_lib_filename()))
        .and_then(|m| m.modified())
        .ok();
    let b_mtime = std::fs::metadata(b.join(mocida_lib_filename()))
        .and_then(|m| m.modified())
        .ok();
    match (a_mtime, b_mtime) {
        (Some(am), Some(bm)) => Some(am > bm),
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    }
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

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
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
