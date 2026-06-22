//! Alloy — dual-mode binary.
//!
//! * `alloy run <file.crs>` / `alloy <file.crs>` → interpret in the terminal
//!   (reuses the portable `alloy-vm` lib; output goes straight to stdout).
//! * `alloy update` → run the self-updater headlessly.
//! * `alloy` (no args) / `alloy gui` → open the MUI window (playground + file
//!   runner + updater).
//!
//! The GUI half links mocida + mui-runtime; the CLI half does not touch them,
//! so the same binary serves both. Build with:
//!   cargo build --release --manifest-path alloy-gui/Cargo.toml
//! (needs MOCIDA_INCLUDE_DIR / MOCIDA_LIB_DIR — see mocida-sys).

// The backend is plain Rust sharing this crate's deps; pull it in directly
// (the `import { ... } from "./backend.rs"` in alloy.mui is for the parser/host
// wiring — here we call its functions ourselves).
#[path = "../backend.rs"]
mod backend;

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        None | Some("gui") => run_gui(),
        Some("update") => match backend::check_update() {
            info if info.available => match backend::apply_update(&info.asset_url) {
                Ok(()) => {
                    println!("Alloy updated to {}", info.latest);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("alloy: update failed: {e}");
                    ExitCode::FAILURE
                }
            },
            info => {
                println!("Alloy {} is already the latest version ({})", info.current, info.latest);
                ExitCode::SUCCESS
            }
        },
        Some("run") => match args.get(1) {
            Some(file) => cli_run(Path::new(file)),
            None => {
                eprintln!("usage: alloy run <file.crs>");
                ExitCode::FAILURE
            }
        },
        // Bare path: `alloy foo.crs`
        Some(path) if Path::new(path).exists() => cli_run(Path::new(path)),
        Some(other) => {
            eprintln!("alloy: unknown command `{other}` (use: run <file> | update | gui)");
            ExitCode::FAILURE
        }
    }
}

/// CLI interpretation — output to real stdout (not captured), like `alloy-vm`'s
/// own binary. Reuses the portable interpreter.
fn cli_run(file: &Path) -> ExitCode {
    use alloy_vm::interp::Interpreter;
    use copper_syntax::program::parse_program;

    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("alloy: could not read {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    let prog = parse_program(&src);
    if !prog.errors.is_empty() {
        for err in &prog.errors {
            eprintln!(
                "alloy: syntax error @ {}..{}: {}",
                err.span.start, err.span.end, err.message
            );
        }
        return ExitCode::FAILURE;
    }
    match Interpreter::new().run_program(&prog) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!(
                "alloy: runtime error @ {}..{}: {}",
                e.span.start, e.span.end, e.message
            );
            ExitCode::FAILURE
        }
    }
}

// ===========================================================================
// GUI host (mocida + mui-runtime)
// ===========================================================================

/// The `.mui` baked into the binary (used in release / when the source file
/// isn't on disk). In dev, the on-disk `alloy.mui` is preferred and watched for
/// changes so edits hot-reload live.
const MUI_BAKED: &str = include_str!("../alloy.mui");

/// On-disk path of `alloy.mui` (next to the crate), for dev hot-reload.
fn mui_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("alloy.mui")
}

/// Current `.mui` source: the on-disk file if present, else the baked copy.
fn read_mui_source() -> String {
    std::fs::read_to_string(mui_path()).unwrap_or_else(|_| MUI_BAKED.to_string())
}

fn run_gui() -> ExitCode {
    match try_run_gui() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("alloy: failed to open the GUI: {e}");
            ExitCode::FAILURE
        }
    }
}

fn try_run_gui() -> Result<(), Box<dyn std::error::Error>> {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use mocida::{App, Color};
    use mui_runtime::Reactive;

    // 1. Parse the .mui to extract the App{} config + entry view. Read from
    //    disk when available (dev hot-reload), else the baked copy.
    let mui_src = read_mui_source();
    let doc = mui_syntax::parse(&mui_src);
    let view = mui_runtime::entry_view(&doc).ok_or("no view found in alloy.mui")?;
    let view_name = view.name.clone();
    let cfg = mui_runtime::window_config(&doc, &view_name);

    // 2. Pre-creation flags (must precede App::new).
    mui_runtime::prefer_custom_titlebar(&cfg);
    mui_runtime::prefer_renderer(&cfg);

    // 3. Load app.bundle (name, id) and register the icon with an ABSOLUTE
    //    path, so `mocida://alloy-icon.png` resolves regardless of the CWD the
    //    binary is launched from (the manifest's relative path would not).
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bundle = manifest_dir.join("app.bundle");
    if bundle.is_file() {
        mocida::bundle::load_manifest(&bundle.to_string_lossy());
    }
    let icon = manifest_dir.join("assets").join("alloy-icon.png");
    if icon.is_file() {
        mocida::bundle::set("mocida://alloy-icon.png", &icon.to_string_lossy());
    }
    let logo = manifest_dir.join("assets").join("alloy-logo.png");
    if logo.is_file() {
        mocida::bundle::set("mocida://alloy-logo.png", &logo.to_string_lossy());
    }

    // 4. Create the window.
    let (br, bg, bb, _) = cfg.background;
    let mut app = App::new(&cfg.title, cfg.width, cfg.height)?;
    app.set_background_color(Color::rgb(br as i32, bg as i32, bb as i32));
    if let Some(name) = &cfg.name {
        mocida::bundle::set_name(name);
        let _ = app.set_name(name);
    }
    if let Some(id) = &cfg.id {
        let _ = app.set_app_id(id);
    }
    if cfg.min_width > 0 || cfg.min_height > 0 {
        app.set_min_size(cfg.min_width, cfg.min_height);
    }

    // 5. Render tuning + backdrop + window icon.
    mui_runtime::apply_render_config(&mut app, &cfg);
    mui_runtime::apply_backdrop(&cfg);
    if app.set_window_icon("mocida://alloy-icon.png").unwrap_or(false) {
        // custom icon set
    } else {
        mui_runtime::set_default_window_icon(&mut app);
    }

    // 6. Fonts.
    mocida::text::search_fonts();
    let _ = mocida::text::get_font("Arial");

    // 7. Build the initial tree, seed signals. `source_cell` holds the live
    //    `.mui` text so the on-tick hot-reload can swap it without a restart.
    let source_cell = Rc::new(RefCell::new(mui_src.clone()));
    let loaded = mui_syntax::loader::load_from(&mui_src, Path::new("alloy.mui"));
    let components = loaded.registry();
    let (children, reactive) =
        mui_runtime::build_view_seeded(view, &components, &HashMap::new())?;
    reactive.set_str("version", &backend::current_version());
    reactive.set_str("output", "");
    // TextArea renders real newlines, so a multi-line default is fine.
    reactive.set_str(
        "source",
        "func main() {\n    println(\"hello from Alloy\")\n}\n",
    );

    let slot: Rc<RefCell<Option<Reactive>>> = Rc::new(RefCell::new(Some(reactive)));
    app.set_children(children);

    // 8. On-tick: handle Run/Open/Update requests + rebuild on resize / structural change.
    let app_ptr = app.as_ptr();
    let tick_slot = Rc::clone(&slot);
    // Remember the last-seen trigger counters so we only act on a change.
    let last_run = Rc::new(RefCell::new(0i32));
    let last_update = Rc::new(RefCell::new(0i32));
    let last_open = Rc::new(RefCell::new(0i32));
    // Last (win w, win h, screen w, screen h) we built against — seeded zeros so
    // the first tick rebuilds against the realized size, then again on any resize
    // so `Window.width`/`Window.height` (and `width: Window.width - N`) re-resolve.
    let last_size = std::rc::Rc::new(std::cell::Cell::new((0i32, 0i32, 0i32, 0i32)));
    // Hot-reload watch: the live `.mui` source cell + the last-seen mtime.
    let watch_source = Rc::clone(&source_cell);
    let last_mtime: Rc<RefCell<Option<std::time::SystemTime>>> = Rc::new(RefCell::new(None));

    // Shared rebuild closure — re-runs build against the current window size,
    // seeding from the live signal values so user state survives the swap.
    let rebuild = {
        let tick_slot = Rc::clone(&tick_slot);
        let source_cell = Rc::clone(&source_cell);
        move || {
            let seed = tick_slot
                .borrow()
                .as_ref()
                .map(|r| r.values())
                .unwrap_or_default();
            let src = source_cell.borrow().clone();
            let loaded = mui_syntax::loader::load_from(&src, Path::new("alloy.mui"));
            let comps = loaded.registry();
            if let Some(v) = mui_runtime::entry_view(&loaded.entry) {
                if let Ok((c, r)) = mui_runtime::build_view_seeded(v, &comps, &seed) {
                    let raw = c.into_raw();
                    unsafe {
                        mocida::sys::UIApp_SetChildren(app_ptr, raw);
                    }
                    *tick_slot.borrow_mut() = Some(r);
                }
            }
        }
    };

    app.on_tick(move || {
        // (-1) Hot-reload: if alloy.mui changed on disk, reload + rebuild live.
        let cur_m = std::fs::metadata(mui_path())
            .ok()
            .and_then(|md| md.modified().ok());
        if let Some(m) = cur_m {
            let prev = *last_mtime.borrow();
            if prev != Some(m) {
                *last_mtime.borrow_mut() = Some(m);
                if prev.is_some() {
                    *watch_source.borrow_mut() = read_mui_source();
                    rebuild();
                }
            }
        }

        // (0) Window resize / monitor change → rebuild so `Window.*` re-resolve.
        let cur_size = unsafe {
            (
                mocida::sys::UIApp_GetWidthG(),
                mocida::sys::UIApp_GetHeightG(),
                mocida::sys::UIScreen_GetWidth(),
                mocida::sys::UIScreen_GetHeight(),
            )
        };
        if cur_size.0 > 0 && cur_size.1 > 0 && cur_size != last_size.get() {
            last_size.set(cur_size);
            rebuild();
        }

        if let Some(r) = tick_slot.borrow().as_ref() {
            // --- Run trigger (incremented by the Run button in the .mui) ---
            let run_n = r.get("run_trigger").unwrap_or(0);
            if run_n != *last_run.borrow() {
                *last_run.borrow_mut() = run_n;
                // Prefer the path field (File runner) when it points at a real
                // file; otherwise run the editor `source` (Playground).
                let path = r.get_str("path_field").unwrap_or_default();
                let src = if !path.is_empty() && std::path::Path::new(&path).is_file() {
                    backend::read_file(&path).unwrap_or_default()
                } else {
                    r.get_str("source").unwrap_or_default()
                };
                let res = backend::run_source(&src);
                let shown = match res.error {
                    Some(err) => format!("{}{}", res.output, err),
                    None => res.output,
                };
                r.set_str("output", &shown);
            }

            // --- Open file trigger: native OS file picker (rfd) ---
            let open_n = r.get("open_trigger").unwrap_or(0);
            if open_n != *last_open.borrow() {
                *last_open.borrow_mut() = open_n;
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Copper", &["crs"])
                    .pick_file()
                {
                    let p = path.to_string_lossy().into_owned();
                    match backend::read_file(&p) {
                        Ok(src) => {
                            r.set_str("source", &src);
                            r.set_str("path_field", &p);
                            r.set_str("output", "");
                        }
                        Err(e) => r.set_str("output", &e),
                    }
                }
            }

            // --- Update trigger ---
            let upd_n = r.get("update_trigger").unwrap_or(0);
            if upd_n != *last_update.borrow() {
                *last_update.borrow_mut() = upd_n;
                let info = backend::check_update();
                let msg = if info.available {
                    match backend::apply_update(&info.asset_url) {
                        Ok(()) => format!("Updated to {} — restart Alloy.", info.latest),
                        Err(e) => format!("Update failed: {e}"),
                    }
                } else {
                    format!("Already up to date ({}).", info.latest)
                };
                r.set_str("update_status", &msg);
            }
        }

        // --- Structural rebuild when an if/for signal changed (e.g. tab) ---
        let dirty = tick_slot
            .borrow()
            .as_ref()
            .map(|r| r.take_dirty())
            .unwrap_or(false);
        if dirty {
            rebuild();
        }
    });

    // 9. Run (blocks until the window closes).
    app.show().run();
    drop(slot);
    Ok(())
}
