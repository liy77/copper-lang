# copper-installer — native host for the MUI Copper installer

The installer's **UI lives entirely in `../installer.mui`** and is drawn by the
MUI runtime (`mui-runtime`) — the exact same engine `cforge run` uses, so the
dev preview and this binary render an identical tree. This crate is the thin
**native host** that supplies what the runtime can't:

- runs the prerequisite probes (`backend.rs`) at startup and **seeds** the
  view's signals, so the form opens pre-filled (detected source, cargo version,
  elevation);
- watches `phase`: the view's **Install** button only flips
  `phase = "installing"`; the host then runs `install_copper` on a **background
  thread** and streams each log line into the `log_text` signal (the
  `installing` screen shows it live, UI stays responsive);
- sets `phase = "done"` (and `err` on failure) when the thread finishes;
- **Close** flips `phase = "exit"`, which quits.

Reactive screen switches use the runtime's `build_view_seeded` + dirty-flag
rebuild (see `mui_runtime`): changing a *structural* signal (`phase`) rebuilds
and swaps the tree, seeding from the live values so nothing typed is lost; a
text input changing does **not** rebuild (keeps focus).

## Build & run

The mocida C toolkit must be built first (shared) and the bindgen env set:

```powershell
$env:LIBCLANG_PATH      = "C:\Program Files\LLVM\bin"
$env:MOCIDA_INCLUDE_DIR = "C:\Users\hcsbr\Documents\mocida\mocida\src\headers"
$env:MOCIDA_LIB_DIR     = "C:\Users\hcsbr\Documents\mocida\mocida\build\win32\release"
cargo run            # or: cargo build --release ; .\target\release\copper-installer.exe
```

`installer.mui` is read from disk at runtime (falling back to the copy embedded
with `include_bytes!`), so editing the `.mui` and re-launching shows the change
without a rebuild. Iterate on the design with `cforge run installer.mui` (live
hot-reload + reactive phase switching); use this binary for the real install.

> The install writes to `~/.copper` (or `C:\Program Files\Copper` for All users)
> and the `COPPER_PATH` / `PATH` registry entries — i.e. it modifies the system,
> reversibly (`scripts/uninstall.py`). It is the same layout `install.py` writes.
