# Installing Copper

The Copper compiler ships with a universal installer that exposes `cforge`
globally on your system. Installers live in [`scripts/`](../scripts) at the
project root.

## Prerequisites

- **Windows 10 / 11**, **macOS**, or **Linux**
- Rust + Cargo — install from <https://rustup.rs/>

## Quick install

From the project root:

| Platform | Command |
| --- | --- |
| Windows  | `scripts\install.bat` |
| Linux    | `bash scripts/install.sh` |
| macOS    | `bash scripts/install-mac.sh` |

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

All live in [`scripts/`](../scripts):

| Script | Purpose |
| --- | --- |
| `install.bat` / `install.sh` / `install-mac.sh` | Build a release binary and install it system- or user-wide |
| `uninstall.bat` | Remove the installation, the `COPPER_PATH` env var, and `bin` from `PATH` |
| `build.bat` | Build a release binary into `target/release/` without installing |
| `cleanup.bat` | Remove `target/`, `dist/`, and stray debug logs |
| `diagnose.bat` | Print install diagnostics (Windows) |

## Examples

The repo ships sample programs in [`examples/`](../examples):

```sh
cforge run examples/loops.crs
cforge run examples/interpolation.crs
cforge run examples/collections.crs
cforge run examples/matching.crs
cforge run examples/optional.crs
cforge run examples/ternary.crs
```

## Uninstall

| Install scope | How to uninstall |
| --- | --- |
| Local (Windows) | `"%USERPROFILE%\.copper\uninstall.bat"` |
| Global (Windows) | Run `"C:\Program Files\Copper\uninstall.bat"` **as Administrator** |

Either uninstaller auto-detects scope from your privilege level. You can also
run `scripts\uninstall.bat` directly from the source tree if the installed
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
   - Windows: `scripts\install-hooks.bat`
   - Unix:    `bash scripts/install-hooks.sh`
4. Commit your changes with a clear message.
5. Open a pull request.

Hooks installed by step 3:

- **`pre-commit`** — `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`
- **`pre-push`** — `cargo test`

Bypass with `--no-verify` only in emergencies; the same checks block in CI.
