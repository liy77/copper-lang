# Installing Copper

The Copper compiler ships with a universal installer that exposes `cforge`
globally on your system. Installers live in [`scripts/`](../scripts) at the
project root.

## Prerequisites

- **Windows 10 / 11**, **macOS**, or **Linux**
- **Python 3.7+** — the tooling is a single cross-platform `install.py`
- Rust + Cargo — install from <https://rustup.rs/>

## Quick install

From the project root, one command works everywhere:

```sh
python scripts/install.py
```

There are also thin shims if you prefer to double-click / not type `python`:

| Platform | Command |
| --- | --- |
| Windows  | `scripts\install.bat` |
| Unix     | `bash scripts/install.sh` |

Force a specific scope with `--local` (per-user) or `--global` (all users,
needs admin/root).

The installer auto-detects whether you launched it with admin / root
privileges:

| Privilege | Scope | Path |
| --- | --- | --- |
| Admin / root | Global (all users) | `C:\Program Files\Copper` *(Windows)*, `/usr/local/lib/copper` *(Unix)* |
| Normal user | Local (current user) | `%USERPROFILE%\.copper` *(Windows)*, `$HOME/.copper` *(Unix)* |

Restart your terminal after installation so the new `PATH` is picked up.

## Usage

```sh
# Compile and run
cforge run main.crs

# Run the default file (main.crs in cwd)
cforge run

# Compile only
cforge -c -i main.crs

# Compile every .crs in a directory
cforge -c -i src/

# Show help
cforge --help

# Show version (with build date for pre-releases)
cforge --version
```

## Bundled scripts

All live in [`scripts/`](../scripts) and are plain Python — one file per task,
cross-platform (Windows / Linux / macOS). The `_pretty.py` module holds the
shared copper-themed output styling.

| Script | Purpose |
| --- | --- |
| `install.py` | Build a release binary and install it system- or user-wide |
| `uninstall.py` | Remove the installation, the `COPPER_PATH` env var, and `bin` from `PATH` |
| `build.py` | Build a release binary into `target/release/` without installing |
| `cleanup.py` | Remove `target/`, `dist/`, and stray debug logs |
| `diagnose.py` | Print install diagnostics |
| `hooks.py` | Activate the repo's pre-commit / pre-push git hooks |

Run any of them with `python scripts/<name>.py` (add `--help` for options).
`install.bat` / `install.sh` (and `uninstall.bat`) are thin shims that just
forward to the matching `.py`.

## Examples

The repo ships sample programs in [`examples/`](../examples):

```sh
cforge run examples/copper/loops.crs
cforge run examples/copper/interpolation.crs
cforge run examples/copper/collections.crs
cforge run examples/copper/matching.crs
cforge run examples/copper/optional.crs
cforge run examples/copper/ternary.crs
```

## Uninstall

| Install scope | How to uninstall |
| --- | --- |
| Local (Windows) | `python "%USERPROFILE%\.copper\uninstall.py"` (or double-click `uninstall.bat`) |
| Global (Windows) | Run `python "C:\Program Files\Copper\uninstall.py"` **as Administrator** |
| Unix | `python ~/.copper/uninstall.py` (or `sudo python /usr/local/lib/copper/uninstall.py` for global) |

The uninstaller auto-detects scope from your privilege level. You can also run
`python scripts/uninstall.py` directly from the source tree if the installed
copy is missing or broken — it works the same way.

## Project configuration

Copper projects use `properties.kson` at their root:

```kson
name = "MyProject"
version = "1.0.0"
edition = 2021

[dependencies]
serde_json = "1.0.120"
regex      = "1.10.5"
ai_copper  = { git = "https://github.com/CopperRS/ai_copper.git" }
```

## Troubleshooting

### `cforge` is not recognized

1. Open a **new** terminal (the running one cached the old `PATH`).
2. Confirm the install location is in `PATH`:
   - Windows: `echo %PATH%` should contain `%COPPER_PATH%\bin`.
   - Unix: `echo $PATH` should contain `$HOME/.copper/bin` or `/usr/local/lib/copper/bin`.
3. In PowerShell, `where` is `Where-Object` (a cmdlet), not the PATH search
   utility. Use `where.exe cforge` or `Get-Command cforge -All`.

### Permission errors

- Global install requires admin / root. Re-launch with elevation.
- Local install needs no privileges; install into your home directory instead.

### Rust missing

```sh
cargo --version
rustc --version
```

If either is missing, install via <https://rustup.rs/>.

## Contributing

1. Fork the repository.
2. Create a topic branch.
3. Activate the repo's git hooks once so your commits/pushes run the
   same lint gates CI does:
   - Any OS: `python scripts/hooks.py`
4. Commit your changes with a clear message.
5. Open a pull request.

Hooks installed by step 3:

- **`pre-commit`** — `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`
- **`pre-push`** — `cargo test`

Bypass with `--no-verify` only in emergencies; the same checks block in CI.
