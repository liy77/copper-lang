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

const MUI_SOURCE: &str = include_str!("../alloy.mui");

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

    // 1. Parse the .mui to extract the App{} config + entry view.
    let doc = mui_syntax::parse(MUI_SOURCE);
    let view = mui_runtime::entry_view(&doc).ok_or("no view found in alloy.mui")?;
    let view_name = view.name.clone();
    let cfg = mui_runtime::window_config(&doc, &view_name);

    // 2. Pre-creation flags (must precede App::new).
    mui_runtime::prefer_custom_titlebar(&cfg);
    mui_runtime::prefer_renderer(&cfg);

    // 3. Load app.bundle (registers mocida://alloy-icon.png, name, id).
    let bundle = Path::new(env!("CARGO_MANIFEST_DIR")).join("app.bundle");
    if bundle.is_file() {
        mocida::bundle::load_manifest(&bundle.to_string_lossy());
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

    // 7. Build the initial tree, seed signals.
    let loaded = mui_syntax::loader::load_from(MUI_SOURCE, Path::new("alloy.mui"));
    let components = loaded.registry();
    let (children, reactive) =
        mui_runtime::build_view_seeded(view, &components, &HashMap::new())?;
    reactive.set_str("version", &backend::current_version());
    reactive.set_str("output", "");
    reactive.set_str("source", "func main() {\n    println(\"hello from Alloy\")\n}\n");

    let slot: Rc<RefCell<Option<Reactive>>> = Rc::new(RefCell::new(Some(reactive)));
    app.set_children(children);

    // 8. On-tick: handle Run/Open/Update requests + rebuild on structural change.
    let app_ptr = app.as_ptr();
    let tick_slot = Rc::clone(&slot);
    // Remember the last-seen trigger counters so we only act on a change.
    let last_run = Rc::new(RefCell::new(0i32));
    let last_update = Rc::new(RefCell::new(0i32));

    app.on_tick(move || {
        if let Some(r) = tick_slot.borrow().as_ref() {
            // --- Run trigger (incremented by the Run button in the .mui) ---
            let run_n = r.get("run_trigger").unwrap_or(0);
            if run_n != *last_run.borrow() {
                *last_run.borrow_mut() = run_n;
                let src = r.get_str("source").unwrap_or_default();
                let res = backend::run_source(&src);
                let shown = match res.error {
                    Some(err) => format!("{}{}", res.output, err),
                    None => res.output,
                };
                r.set_str("output", &shown);
            }

            // --- Open file trigger: `open_path` holds a path to load ---
            let path = r.get_str("open_path").unwrap_or_default();
            if !path.is_empty() {
                r.set_str("open_path", "");
                match backend::read_file(&path) {
                    Ok(src) => r.set_str("source", &src),
                    Err(e) => r.set_str("output", &e),
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

        // --- Structural rebuild when an if/for signal changed ---
        let dirty = tick_slot
            .borrow()
            .as_ref()
            .map(|r| r.take_dirty())
            .unwrap_or(false);
        if dirty {
            let seed = tick_slot
                .borrow()
                .as_ref()
                .map(|r| r.values())
                .unwrap_or_default();
            let loaded = mui_syntax::loader::load_from(MUI_SOURCE, Path::new("alloy.mui"));
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
    });

    // 9. Run (blocks until the window closes).
    app.show().run();
    drop(slot);
    Ok(())
}
