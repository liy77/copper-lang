//! Native host for the Copper installer.
//!
//! The UI/design lives entirely in `installer.mui` and is rendered by the MUI
//! runtime (`mui-runtime`) — the same engine `cforge run` uses, so the dev
//! preview and this binary draw the identical tree. What the runtime can't do
//! (call the real `backend.rs`, run a multi-minute `cargo build` off the UI
//! thread) this host supplies:
//!
//!   * at startup it runs the prerequisite probes and **seeds** the view's
//!     signals (`source_dir`, `has_cargo`, `cargo_ver`, …) so the form opens
//!     pre-filled;
//!   * the view's `Install` button only flips `phase = "installing"`; this host
//!     sees that transition, runs `install_copper` on a background thread, and
//!     streams each log line into the `log_text` signal (which the `installing`
//!     screen shows live);
//!   * when the thread finishes it sets `phase = "done"` (and `err` on failure);
//!   * the `Close` button flips `phase = "exit"`, which quits.
//!
//! Reactive screen switches happen exactly like in `cforge run`: a structural
//! signal (`phase`) changing flags the tree dirty, and the tick rebuilds it,
//! seeding from the live values so nothing the user typed is lost.

// Release builds are a GUI app — no console window pops up behind the installer
// (the mocida C log goes nowhere). Debug keeps the console so the logs are
// visible while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[path = "../../backend.rs"]
mod backend;

/// The installer's own icon (a copper download glyph), shown in the title bar /
/// taskbar at runtime. The .exe *file* icon is embedded separately by build.rs.
const ICON_PNG: &[u8] = include_bytes!("../assets/installer.png");

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Instant;

use mocida::{App, Color};
use mui_runtime::Reactive;

/// The installer UI, baked in so the binary is self-contained.
const SRC: &str = include_str!("../../installer.mui");
/// Absolute path to the source on this machine — only used to resolve relative
/// imports / the bundle when present; a missing path just degrades gracefully.
const SRC_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../installer.mui");

/// A message from the install worker thread to the UI tick.
enum Msg {
    /// One formatted log line.
    Line(String),
    /// The install finished (Ok, or Err with a message).
    Done(Result<(), String>),
}

/// The `.mui` to render: the first non-flag CLI arg (cforge passes the file when
/// it launches us as a `host:`), else the sibling source path.
fn mui_path() -> String {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| SRC_PATH.to_string())
}

/// The current installer source: the on-disk `.mui` if present (so edits show on
/// the next rebuild — handy while iterating), else the embedded copy (so a
/// distributed binary is self-contained).
fn read_source() -> String {
    std::fs::read_to_string(mui_path()).unwrap_or_else(|_| SRC.to_string())
}

/// Last-modified time of `path` (for hot-reload), `None` if it can't be read.
fn file_mtime(path: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Build the widget tree from `installer.mui`, seeding signal values from `seed`
/// (so a rebuild preserves live state).
fn build_tree(
    seed: &HashMap<String, String>,
) -> Result<(mocida::Children, Reactive), Box<dyn std::error::Error>> {
    let src = read_source();
    let entry_path = std::path::PathBuf::from(mui_path());
    let loaded = mui_syntax::loader::load_from(&src, &entry_path);
    let view = mui_runtime::entry_view(&loaded.entry).ok_or("no `view` to render")?;
    let registry = loaded.registry();
    let (children, reactive) = mui_runtime::build_view_seeded(view, &registry, seed)?;
    Ok((children, reactive))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Headless install (`--install [--global]` [--source DIR] [--dir DIR]): run
    // the real pipeline with no window, streaming the log to stdout. Same code
    // path the GUI's Install button drives — handy for CI / a no-UI install.
    if std::env::args().any(|a| a == "--install") {
        return run_headless_install();
    }

    // 1) Run the real prerequisite probes and seed the view's signals.
    let q = backend::prereqs_quick();
    let c = backend::prereqs_check();
    let mut seed: HashMap<String, String> = HashMap::new();
    seed.insert("source_dir".into(), q.detected_source.clone());
    seed.insert("default_loc".into(), q.default_local_dir.clone());
    seed.insert("default_glob".into(), q.default_global_dir.clone());
    seed.insert("install_dir".into(), q.default_local_dir.clone());
    seed.insert("cargo_ver".into(), c.cargo_version.clone());
    seed.insert("has_cargo".into(), bit(c.has_cargo));
    seed.insert("is_admin".into(), bit(c.is_admin));
    // `--dry-run` jumps straight to the installing screen (mock log + progress)
    // so the install UI can be exercised without a real build/registry write.
    if dry_run() {
        seed.insert("phase".into(), "installing".into());
    }
    // `--demo-error` opens straight on the failure screen with a sample multi-line
    // reason, so the error UI (scrollable, wrapped) can be checked without a real
    // failing install.
    if std::env::args().any(|a| a == "--demo-error") {
        let sample = "Build failed:\nerror[E0425]: cannot find value `foo` in this scope\n  --> src/main.rs:12:5\n   |\n12 |     foo();\n   |     ^^^ not found in this scope\n\nerror: could not compile `demo` (bin \"demo\") due to 1 previous error\nThis is a long reason to verify wrapping works for the full cargo stderr output rather than being clipped off the edge of the window.";
        seed.insert("phase".into(), "done".into());
        seed.insert("err".into(), sample.into());
        // List signals seed from a '\n'-joined string (split back into items).
        seed.insert("err_lines".into(), sample.into());
    }

    // 2) Window config from the `App() { }` block.
    let doc = mui_syntax::parse(&read_source());
    let view_name = mui_runtime::entry_view(&doc)
        .map(|v| v.name.clone())
        .unwrap_or_else(|| "Installer".into());
    let cfg = mui_runtime::window_config(&doc, &view_name);

    // Renderer backend must be chosen BEFORE *any* SDL call (the env-var hint is
    // read when SDL first initialises — loading a bundle or creating the window
    // both touch SDL, so this has to come first).
    mui_runtime::prefer_renderer(&cfg);

    // Load the asset bundle (the Copper logo) sitting next to installer.mui.
    let bundle = format!("{}/../app.bundle", env!("CARGO_MANIFEST_DIR"));
    let _ = mocida::bundle::load_manifest(&bundle);

    let mut app = App::new(&cfg.title, cfg.width, cfg.height)?;
    let (br, bg, bb, _) = cfg.background;
    app.set_background_color(Color::rgb(br as i32, bg as i32, bb as i32));
    if let Some(name) = &cfg.name {
        mocida::bundle::set_name(name);
        let _ = app.set_name(name);
    }
    if let Some(id) = &cfg.id {
        let _ = app.set_app_id(id);
    }
    // The installer's OWN window icon (NOT the generic mui-dev lightning block):
    // extract the embedded PNG to a temp file and apply it.
    {
        let p = std::env::temp_dir().join("copper-installer-icon.png");
        let _ = std::fs::write(&p, ICON_PNG);
        if let Some(s) = p.to_str() {
            let _ = app.set_window_icon(s);
        }
    }
    // Min/max window size from the App block (desktop; 0 = unconstrained).
    if cfg.min_width > 0 || cfg.min_height > 0 {
        app.set_min_size(cfg.min_width, cfg.min_height);
    }
    if cfg.max_width > 0 || cfg.max_height > 0 {
        app.set_max_size(cfg.max_width, cfg.max_height);
    }
    // Renderer backend + AA/MSAA tuning from the App block.
    mui_runtime::apply_render_config(&mut app, &cfg);
    mocida::text::search_fonts();
    let _ = mocida::text::get_font("Arial");

    // 3) Initial (prereq-seeded) build.
    let (children, reactive) = build_tree(&seed)?;
    let slot: Rc<RefCell<Option<Reactive>>> = Rc::new(RefCell::new(Some(reactive)));
    app.set_children(children);

    // 4) Controller state shared with the tick.
    let app_ptr = app.as_ptr();
    let tick_slot = slot.clone();
    let rx_slot: Rc<RefCell<Option<mpsc::Receiver<Msg>>>> = Rc::new(RefCell::new(None));
    let install_started = Rc::new(Cell::new(false));
    // Progress 0..100. `target` is set by install milestones; during the long
    // `cargo build` (which reports nothing) it creeps up so the bar keeps moving.
    let target = Rc::new(Cell::new(0.0f32));
    let building = Rc::new(Cell::new(false));
    // Hot-reload: watch the .mui's mtime and rebuild on edit (preserving live
    // state), so editing installer.mui updates the window in place.
    let watch_path = mui_path();
    let last_mtime = Rc::new(RefCell::new(file_mtime(&watch_path)));
    // Folder autocomplete state: (last value seen, value already suggested for,
    // when it last changed) — debounced so we don't rebuild on every keystroke.
    // Seed "already suggested" with the initial value so the form doesn't open
    // cluttered with suggestions.
    let get0 = |k: &str| {
        slot.borrow()
            .as_ref()
            .and_then(|r| r.get_str(k))
            .unwrap_or_default()
    };
    let ac_src = Rc::new(RefCell::new((get0("source_dir"), get0("source_dir"), Instant::now())));
    let ac_dst = Rc::new(RefCell::new((get0("install_dir"), get0("install_dir"), Instant::now())));
    // Last (window w, window h, screen w, screen h) we built against — seeded
    // zeros so the first tick rebuilds once against the realized size (DPI
    // settle), then on every window resize or monitor change.
    let last_size = Cell::new((0i32, 0i32, 0i32, 0i32));

    app.on_tick(move || {
        // (r) Window resize / monitor change → reactive rebuild so `Window.*`/
        // `Screen.*` and any `width: Window.width - N` re-resolve.
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
            let seed = tick_slot
                .borrow()
                .as_ref()
                .map(|r| r.values())
                .unwrap_or_default();
            if let Ok((children, reactive)) = build_tree(&seed) {
                let raw = children.into_raw();
                unsafe { mocida::sys::UIApp_SetChildren(app_ptr, raw) };
                *tick_slot.borrow_mut() = Some(reactive);
            }
        }

        // (h) Hot-reload on a `.mui` edit → rebuild seeded from the live values.
        let now_mtime = file_mtime(&watch_path);
        if now_mtime != *last_mtime.borrow() && now_mtime.is_some() {
            *last_mtime.borrow_mut() = now_mtime;
            let seed = tick_slot
                .borrow()
                .as_ref()
                .map(|r| r.values())
                .unwrap_or_default();
            match build_tree(&seed) {
                Ok((children, reactive)) => {
                    let raw = children.into_raw();
                    unsafe { mocida::sys::UIApp_SetChildren(app_ptr, raw) };
                    *tick_slot.borrow_mut() = Some(reactive);
                    println!("copper-installer: reloaded {watch_path}");
                }
                Err(e) => eprintln!("copper-installer: reload failed: {e}"),
            }
        }

        // (g) Folder autocomplete: ~400ms after the user stops typing a path,
        // fill its suggestion list from backend::list_dirs (one rebuild, not one
        // per keystroke).
        for (field, sig, st) in [
            ("source_dir", "src_suggest", &ac_src),
            ("install_dir", "dst_suggest", &ac_dst),
        ] {
            let cur = tick_slot
                .borrow()
                .as_ref()
                .and_then(|r| r.get_str(field))
                .unwrap_or_default();
            let mut s = st.borrow_mut();
            if cur != s.0 {
                s.0 = cur.clone();
                s.2 = Instant::now();
            }
            if cur != s.1 && s.2.elapsed().as_millis() > 400 {
                s.1 = cur.clone();
                drop(s);
                let sug = backend::list_dirs(&cur);
                if let Some(r) = tick_slot.borrow().as_ref() {
                    r.set_list(sig, sug);
                }
            }
        }

        // (a) Drain the install worker → append log lines + advance progress.
        let mut finished: Option<Result<(), String>> = None;
        let mut new_lines: Vec<String> = Vec::new();
        if let Some(rx) = rx_slot.borrow().as_ref() {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    Msg::Line(l) => new_lines.push(l),
                    Msg::Done(r) => finished = Some(r),
                }
            }
        }
        if !new_lines.is_empty() {
            if let Some(r) = tick_slot.borrow().as_ref() {
                for l in &new_lines {
                    r.push_list("log_lines", l);
                    if let Some(t) = milestone_progress(l) {
                        target.set(t);
                    }
                    if l.contains("Building cforge") {
                        building.set(true);
                    }
                    if l.contains("Build successful") {
                        building.set(false);
                    }
                }
            }
        }
        if let Some(res) = finished {
            if let Some(r) = tick_slot.borrow().as_ref() {
                if let Err(e) = &res {
                    r.set_str("err", e);
                    // Split into lines for the failure screen (Text can't render
                    // a literal `\n`), so the whole cargo stderr is readable.
                    r.set_list("err_lines", e.lines().map(|l| l.to_string()).collect());
                }
                r.set_str("phase", "done"); // structural → rebuild below
            }
            target.set(100.0);
            building.set(false);
            *rx_slot.borrow_mut() = None;
        }
        // Creep the bar during the silent build gap, then push it to the signal
        // (the int equality-guard means it only notifies on a real change).
        if building.get() && target.get() < 68.0 {
            target.set(target.get() + 0.06);
        }
        if let Some(r) = tick_slot.borrow().as_ref() {
            r.set_int("progress", target.get().round() as i32);
        }

        // (b) Rebuild + swap on a structural change (phase switch / new log
        //     line), preserving live state via the value snapshot seed.
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
            match build_tree(&seed) {
                Ok((children, reactive)) => {
                    let raw = children.into_raw();
                    unsafe { mocida::sys::UIApp_SetChildren(app_ptr, raw) };
                    *tick_slot.borrow_mut() = Some(reactive);
                }
                Err(e) => eprintln!("copper-installer: rebuild failed: {e}"),
            }
        }

        // (c) React to the current phase.
        let phase = tick_slot
            .borrow()
            .as_ref()
            .and_then(|r| r.get_str("phase"))
            .unwrap_or_default();
        if phase == "exit" {
            std::process::exit(0);
        }
        if phase == "installing" && !install_started.get() {
            install_started.set(true);
            let (src, dst, global) = {
                let b = tick_slot.borrow();
                let r = b.as_ref().unwrap();
                (
                    r.get_str("source_dir").unwrap_or_default(),
                    r.get_str("install_dir").unwrap_or_default(),
                    r.get_str("scope").as_deref() == Some("global"),
                )
            };
            // Fresh log + progress for this run.
            if let Some(r) = tick_slot.borrow().as_ref() {
                r.set_list("log_lines", Vec::new());
                r.set_int("progress", 0);
            }
            target.set(0.0);
            building.set(false);
            let (tx, rx) = mpsc::channel::<Msg>();
            let txc = tx.clone();
            let dry = dry_run();
            std::thread::spawn(move || {
                if dry {
                    mock_install(&src, &dst, global, |level, text| {
                        let _ = txc.send(Msg::Line(format!("{level}: {text}")));
                    });
                    let _ = tx.send(Msg::Done(Ok(())));
                } else {
                    let res = backend::install_copper(&src, &dst, global, |level, text| {
                        let _ = txc.send(Msg::Line(format!("{level}: {text}")));
                    });
                    let _ = tx.send(Msg::Done(res));
                }
            });
            *rx_slot.borrow_mut() = Some(rx);
        }
    });

    app.show().run();
    drop(slot);
    Ok(())
}

/// Whether `--dry-run` was passed (mock install, no system changes).
fn dry_run() -> bool {
    std::env::args().any(|a| a == "--dry-run")
}

/// Headless install: no window, run `install_copper` and stream the log to
/// stdout. Source/dir default to the detected source + the scope's default dir;
/// override with `--source DIR` / `--dir DIR`. `--global` picks the all-users
/// scope (and dir).
fn run_headless_install() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| -> Option<String> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
    };
    let global = args.iter().any(|a| a == "--global");

    let q = backend::prereqs_quick();
    let source = flag("--source").unwrap_or(q.detected_source);
    let install = flag("--dir").unwrap_or(if global {
        q.default_global_dir
    } else {
        q.default_local_dir
    });
    if source.is_empty() {
        eprintln!("error: no source dir (pass --source <copper-lang dir>)");
        std::process::exit(2);
    }
    println!("Installing cforge");
    println!("  source : {source}");
    println!("  target : {install}");
    println!("  scope  : {}", if global { "all users" } else { "current user" });
    println!();
    let res = backend::install_copper(&source, &install, global, |level, text| {
        println!("[{level}] {text}");
    });
    match res {
        Ok(()) => {
            println!("\n[ok] Installation complete. Open a NEW terminal, then: cforge --version");
            Ok(())
        }
        Err(e) => {
            eprintln!("\n[error] Installation failed: {e}");
            std::process::exit(1);
        }
    }
}

/// A no-op stand-in for `install_copper` (enabled by `--dry-run`)
/// that emits the same log shape with realistic delays — including a long pause
/// on "Building cforge" — so the installing UI (colored log + progress bar) can
/// be exercised without touching the system.
fn mock_install(source_dir: &str, install_dir: &str, _global: bool, log: impl Fn(&str, &str)) {
    use std::thread::sleep;
    use std::time::Duration;
    let steps: &[(&str, String, u64)] = &[
        ("step", format!("Source: {source_dir}"), 300),
        ("step", format!("Target: {install_dir}"), 300),
        ("ok", "Cargo: cargo 1.95.0".into(), 300),
        ("ok", "Created install directories".into(), 300),
        ("step", "Building cforge in release mode (this can take a while)…".into(), 4000),
        ("ok", "Build successful".into(), 300),
        ("ok", "Installed cforge.exe".into(), 250),
        ("ok", "Copied Cargo.toml".into(), 250),
        ("ok", "Copied std/".into(), 250),
        ("ok", "Copied lson/".into(), 250),
        ("ok", "Installed uninstaller".into(), 250),
        ("ok", "COPPER_PATH = (dry run, nothing written)".into(), 300),
        ("ok", "Added %COPPER_PATH%\\bin to PATH".into(), 300),
        ("step", "Installation complete!".into(), 200),
    ];
    for (lvl, txt, ms) in steps {
        log(lvl, txt);
        sleep(Duration::from_millis(*ms));
    }
}

/// Map an install log line to a progress target (0..100). Milestones come from
/// the `log("step"/"ok", …)` calls in backend.rs; the long `cargo build` between
/// "Building cforge" and "Build successful" has no output, so the tick creeps
/// the bar up on its own there.
fn milestone_progress(line: &str) -> Option<f32> {
    let table = [
        ("Source:", 6.0),
        ("Target:", 9.0),
        ("Cargo:", 12.0),
        ("Created install", 16.0),
        ("Building cforge", 20.0),
        ("Build successful", 74.0),
        ("Installed cforge", 80.0),
        ("Copied Cargo", 83.0),
        ("Copied std", 86.0),
        ("Copied lson", 89.0),
        ("Installed uninstaller", 91.0),
        ("COPPER_PATH", 94.0),
        ("Added", 96.0),
        ("complete", 99.0),
    ];
    table
        .iter()
        .find(|(k, _)| line.contains(k))
        .map(|(_, v)| *v)
}

/// `"1"`/`"0"` — how a bool signal is seeded (it rides the int channel).
fn bit(b: bool) -> String {
    if b {
        "1".into()
    } else {
        "0".into()
    }
}
