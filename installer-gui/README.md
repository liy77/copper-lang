# Copper GUI Installer

A graphical installer for the Copper toolchain, built with **MUI** (Copper's
own declarative UI over the mocida toolkit) — no web stack, no Tauri. A
clickable alternative to `scripts/install.py`: pick install scope, install
directory, hit Install. Windows-first.

## Layout

```
installer-gui/
├── installer.mui       The entire UI/design (declarative MUI). cforge renders
│                       this directly — edit it and re-run to see changes.
├── backend.rs          Install logic (prereq probes, cargo build, file copies,
│                       PATH/registry writes). Imported by installer.mui and
│                       linked by the native host. std-only, no extra crates.
├── app.bundle          App name/id + the logo asset (mocida://icon-128.png).
├── copper-installer/   Native host crate that renders installer.mui through the
│                       MUI runtime AND links backend.rs (the part the generic
│                       mui-dev can't do: real install on a worker thread).
│   ├── Cargo.toml
│   ├── build.rs        Embeds the .exe icon + an asInvoker manifest (winresource).
│   ├── assets/         installer.png / installer.ico (the app icon).
│   └── src/main.rs     Seeds prereqs, drives the install, streams the log.
└── README.md
```

The UI lives **entirely** in `installer.mui`. The `copper-installer` crate is
just the host that supplies a real backend and runs the multi-minute build off
the UI thread — the same MUI runtime the dev preview uses, so both draw the
identical tree.

## Running

```powershell
# Dev: render + hot-reload the .mui through the native host (it links backend.rs,
# so prereqs + the real install work in the loop too).
cforge run installer-gui/installer.mui

# Or run the host directly.
cargo run --release --manifest-path installer-gui/copper-installer/Cargo.toml
```

Because `installer.mui`'s `App() { … host: "copper-installer" }` names a native
host, `cforge run` launches that crate instead of the generic `mui-dev`.

### Headless install (no window)

The host also runs the same pipeline with no UI — handy for CI:

```powershell
copper-installer --install [--global] [--source <copper-lang dir>] [--dir <install dir>]
```

## Building a release

```powershell
cargo build --release --manifest-path installer-gui/copper-installer/Cargo.toml
```

Produces `copper-installer/target/release/copper-installer.exe` — a GUI-subsystem
binary (no console window) with the installer icon baked in. Stage `mocida.dll`
next to it (the mocida-rs build does this; re-copy after any C rebuild).

## How it works

- **`prereqs_quick()` / `prereqs_check()`** (backend.rs) — detect the copper-lang
  source tree (a folder with both `Cargo.toml` and `src/cforge/`), the default
  install dirs, whether `cargo` is on PATH, and whether the process is elevated.
  The host seeds these into the view's signals so the form opens pre-filled.
- The **Install button** only flips `phase = "installing"`. The host sees that,
  runs `install_copper()` on a background thread, and streams each log line into
  the `log_lines` signal (the installing screen shows it live, colour-coded).
- **`install_copper()`** runs `cargo build --release` in the source tree, copies
  `target/release/cforge.exe`, `Cargo.toml`, `std/`, `lson/`, and the uninstaller
  into the install dir, then sets `COPPER_PATH` and appends `%COPPER_PATH%\bin`
  to `PATH` (HKLM `…\Session Manager\Environment` if global, HKCU otherwise) and
  broadcasts `WM_SETTINGCHANGE` — the same layout `scripts/install.py` writes.
- On failure the host fills `err` + `err_lines`; the done screen shows the full
  reason (the cargo stderr), wrapped and scrollable.

## Trade-offs vs. install.py

- **Identical install layout.** Same registry keys, same dir structure, same
  uninstaller — `python scripts/uninstall.py` cleanly removes installs done
  through this GUI.
- **Cargo build runs as a subprocess** — reuses the user's Rust toolchain rather
  than bundling rustc, on a worker thread so the UI stays responsive.
