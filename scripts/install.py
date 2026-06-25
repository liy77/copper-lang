#!/usr/bin/env python3
"""
Copper Language installer — one cross-platform script for Windows, Linux and
macOS (it replaces the old install.bat / install.sh / install-mac.sh trio).

What it does:
  1. cd to the project root (the parent of scripts/).
  2. Verify Cargo.toml + cargo are present.
  3. Pick an install scope from your privilege level:
        admin / root  -> global  (C:\\Program Files\\Copper  |  /usr/local/lib/copper)
        normal user   -> local   (%USERPROFILE%\\.copper      |  $HOME/.copper)
  4. Build cforge in release mode (fresh link — deletes the old binary first).
  5. Build lson from lson-src/ (git submodule).
  6. Copy cforge + alloy + copper-lsp + mui-lsp + Cargo.toml + std/ + built lson
     binary into the install dir (binaries land in bin/, so on PATH).
  7. Register COPPER_PATH and add %COPPER_PATH%/bin to PATH:
      Windows -> HKCU/HKLM registry (REG_EXPAND_SZ) + a settings broadcast
      Unix    -> /etc/profile.d/copper.sh, fish conf.d, or a managed block in your rc files
  8. Drop uninstall.py (+ a uninstall.bat shim on Windows) next to the install.

Usage:
    python scripts/install.py            # auto scope from privileges
    python scripts/install.py --local    # force a per-user install
    python scripts/install.py --global   # force an all-users install (needs admin/root)
"""

import argparse
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _pretty import banner, c, COPPER, DIM, fail, head, info, ok, step, warn  # noqa: E402

SYS = platform.system()
ROOT = Path(__file__).resolve().parent.parent

# Profile block markers (Unix) — kept byte-identical to the old installer so an
# install done by the shell script can still be cleanly removed, and vice versa.
BLOCK_BEGIN = "# >>> COPPER PATH (copper-lang) >>>"
BLOCK_END = "# <<< COPPER PATH <<<"
FISH_BLOCK_BEGIN = "# >>> COPPER PATH (copper-lang, fish) >>>"
FISH_BLOCK_END = "# <<< COPPER PATH (copper-lang, fish) <<<"


# --- privilege detection ------------------------------------------------
def is_admin():
    if SYS == "Windows":
        try:
            import ctypes
            return ctypes.windll.shell32.IsUserAnAdmin() != 0
        except Exception:
            return False
    return os.geteuid() == 0


# --- prerequisites ------------------------------------------------------
def check_prereqs():
    if not (ROOT / "Cargo.toml").exists():
        fail(f"Cargo.toml not found at {ROOT}.")
        info("Keep install.py inside the scripts/ folder of the copper-lang project.")
        sys.exit(1)
    if not shutil.which("cargo"):
        fail("Cargo/Rust is not installed or not on PATH.")
        info("Install Rust from https://rustup.rs/ and try again.")
        sys.exit(1)
    out = subprocess.run(["cargo", "--version"], capture_output=True, text=True).stdout.strip()
    ok(f"Rust toolchain: {out}")


# --- scope resolution ---------------------------------------------------
def resolve_scope(force):
    admin = is_admin()
    if force == "global" and not admin:
        fail("--global needs administrator / root privileges. Re-run elevated.")
        sys.exit(1)
    glob = (force == "global") or (force is None and admin)

    if SYS == "Windows":
        install_dir = Path(r"C:\Program Files\Copper") if glob else Path(os.environ["USERPROFILE"]) / ".copper"
    else:
        install_dir = Path("/usr/local/lib/copper") if glob else Path.home() / ".copper"

    return ("global" if glob else "local"), install_dir


# --- build --------------------------------------------------------------
def build_release():
    exe = ROOT / "target" / "release" / ("cforge.exe" if SYS == "Windows" else "cforge")
    if exe.exists():
        step("Removing stale release binary to force a fresh link…")
        exe.unlink()
    head("Building Copper in release mode (cargo build --release --workspace)")
    # --workspace so all members build (alloy, copper-lsp, mui-lsp), not just the
    # root cforge package. Without it cargo builds only the current package.
    if subprocess.run(["cargo", "build", "--release", "--workspace"], cwd=str(ROOT)).returncode != 0:
        fail("cargo build --release failed.")
        sys.exit(1)
    if not exe.exists():
        fail(f"Build reported success but {exe.name} is missing.")
        sys.exit(1)
    ok(f"Built {exe}")
    return exe


def build_lson():
    """Build lson from lson-src/ (git submodule). Returns path to built binary or None."""
    lson_src = ROOT / "lson-src"
    if not (lson_src / "Cargo.toml").exists():
        warn(
            "lson-src/ not found or submodule not initialised — skipping lson build.\n"
            "  Run: git submodule update --init lson-src"
        )
        return None

    binary_name = "lson.exe" if SYS == "Windows" else "lson"
    built_bin = lson_src / "target" / "release" / binary_name

    # lson-src is a git submodule physically nested inside the copper-lang
    # workspace. Even though it's in the root [workspace].exclude list, cargo
    # (1.96) still attaches it to the parent workspace when built from within,
    # failing with "believes it's in a workspace when it's not". The fix cargo
    # itself recommends is an empty [workspace] table in the sub-manifest, which
    # makes it a standalone workspace. We inject it idempotently into the
    # working tree (not committed to the submodule).
    manifest = lson_src / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8")
    if "[workspace]" not in text:
        manifest.write_text(text.rstrip() + "\n\n[workspace]\n", encoding="utf-8")
        info("Added standalone [workspace] table to lson-src/Cargo.toml")

    head("Building lson from lson-src/ (cargo build --release)")
    target_dir = lson_src / "target"
    result = subprocess.run(
        [
            "cargo", "build", "--release",
            "--manifest-path", str(lson_src / "Cargo.toml"),
            "--target-dir", str(target_dir),
        ],
        cwd=str(ROOT),
    )
    if result.returncode != 0:
        warn("lson build failed — it will be built lazily on first use instead.")
        return None

    if not built_bin.exists():
        warn(f"lson build succeeded but binary not found at {built_bin}")
        return None

    ok(f"Built lson: {built_bin}")
    return built_bin


# --- file install -------------------------------------------------------
def copy_payload(exe, install_dir, lson_bin=None):
    bin_dir = install_dir / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)

    dst_exe = bin_dir / exe.name
    shutil.copy2(exe, dst_exe)
    if SYS != "Windows":
        os.chmod(dst_exe, 0o755)
    ok(f"Installed {exe.name}")

    # Alloy — the Copper interpreter (binary `alloy`, crate alloy-vm). It is a
    # workspace member, so the release build above produced it for free. Ship it
    # in bin/ next to cforge so it lands on PATH via %COPPER_PATH%\bin.
    alloy_name = "alloy.exe" if SYS == "Windows" else "alloy"
    src_alloy = exe.parent / alloy_name
    if src_alloy.exists():
        dst_alloy = bin_dir / alloy_name
        shutil.copy2(src_alloy, dst_alloy)
        if SYS != "Windows":
            os.chmod(dst_alloy, 0o755)
        ok(f"Installed {alloy_name}")
    else:
        warn(f"alloy not found at {src_alloy} — skipping (rebuild the workspace).")

    # Language servers (copper-lsp / mui-lsp) live in bin/ next to cforge, so
    # they land on PATH via %COPPER_PATH%\bin. OndaEngine resolves them through
    # $COPPER_PATH/bin (see onda-launcher/src/lsp.rs server_cmd). They are built
    # for free by the workspace `cargo build --release` (no default-members).
    suffix = ".exe" if SYS == "Windows" else ""
    for lsp in ("copper-lsp", "mui-lsp"):
        src_lsp = exe.parent / f"{lsp}{suffix}"
        if src_lsp.exists():
            dst_lsp = bin_dir / src_lsp.name
            shutil.copy2(src_lsp, dst_lsp)
            if SYS != "Windows":
                os.chmod(dst_lsp, 0o755)
            ok(f"Installed {src_lsp.name}")
        else:
            warn(f"{lsp} not found at {src_lsp} — skipping (rebuild the workspace).")

    if (ROOT / "Cargo.toml").exists():
        shutil.copy2(ROOT / "Cargo.toml", install_dir / "Cargo.toml")
        ok("Installed project metadata (Cargo.toml)")

    # Install lson binary into {install_dir}/lson/lson[.exe].
    if lson_bin is not None and lson_bin.exists():
        lson_dir = install_dir / "lson"
        lson_dir.mkdir(parents=True, exist_ok=True)
        dst_lson = lson_dir / lson_bin.name
        shutil.copy2(lson_bin, dst_lson)
        if SYS != "Windows":
            os.chmod(dst_lson, 0o755)
        ok(f"Installed lson → {dst_lson}")
    else:
        warn("lson not built — it will be compiled from lson-src/ on first use.")

    # std/
    std_src = ROOT / "std"
    if std_src.is_dir():
        dst = install_dir / "std"
        if dst.exists():
            shutil.rmtree(dst)
        shutil.copytree(std_src, dst)
        ok("Installed std/")

    # Ship the uninstaller alongside the install. On Windows we also drop a
    # tiny .bat shim so double-clicking still works without typing `python`.
    src_uninstall = ROOT / "scripts" / "uninstall.py"
    if src_uninstall.exists():
        shutil.copy2(src_uninstall, install_dir / "uninstall.py")
        shim = ROOT / "scripts" / "uninstall.bat"
        if SYS == "Windows" and shim.exists():
            shutil.copy2(shim, install_dir / "uninstall.bat")
        ok("Installed uninstaller")
    else:
        warn("scripts/uninstall.py not found; uninstaller not installed.")


# --- PATH registration: Windows -----------------------------------------
def register_windows(install_dir, scope):
    import winreg

    if scope == "global":
        root, subkey = winreg.HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"
    else:
        root, subkey = winreg.HKEY_CURRENT_USER, "Environment"

    with winreg.OpenKey(root, subkey, 0, winreg.KEY_READ | winreg.KEY_WRITE) as key:
        winreg.SetValueEx(key, "COPPER_PATH", 0, winreg.REG_SZ, str(install_dir))
        ok(f"COPPER_PATH = {install_dir}")

        try:
            cur, _ = winreg.QueryValueEx(key, "PATH")
        except FileNotFoundError:
            cur = ""
        marker = r"%COPPER_PATH%\bin"
        entries = [p for p in cur.split(";") if p]
        if marker in entries:
            info("%COPPER_PATH%\\bin already on PATH.")
        else:
            entries.append(marker)
            winreg.SetValueEx(key, "PATH", 0, winreg.REG_EXPAND_SZ, ";".join(entries))
            ok("Added %COPPER_PATH%\\bin to PATH.")

    _broadcast_setting_change()


def _broadcast_setting_change():
    """Tell already-running shells that the environment changed (best effort)."""
    try:
        import ctypes
        HWND_BROADCAST, WM_SETTINGCHANGE, SMTO_ABORTIFHUNG = 0xFFFF, 0x1A, 0x2
        ctypes.windll.user32.SendMessageTimeoutW(
            HWND_BROADCAST, WM_SETTINGCHANGE, 0, "Environment", SMTO_ABORTIFHUNG, 5000, None
        )
    except Exception:
        pass


# --- PATH registration: Unix --------------------------------------------
def register_unix(install_dir, scope):
    block = "\n".join([
        BLOCK_BEGIN,
        f'export COPPER_PATH="{install_dir}"',
        'case ":$PATH:" in',
        '  *:"$COPPER_PATH/bin":*) ;;',
        '  *) export PATH="$PATH:$COPPER_PATH/bin" ;;',
        "esac",
        BLOCK_END,
    ])
    fish_block = "\n".join([
        FISH_BLOCK_BEGIN,
        f'set -gx COPPER_PATH "{install_dir}"',
        'if not contains -- "$COPPER_PATH/bin" $PATH',
        '    set -gx PATH $PATH "$COPPER_PATH/bin"',
        'end',
        FISH_BLOCK_END,
    ])

    if scope == "global":
        profile = Path("/etc/profile.d/copper.sh")
        profile.parent.mkdir(parents=True, exist_ok=True)
        profile.write_text(block + "\n", encoding="utf-8")
        os.chmod(profile, 0o644)
        ok(f"COPPER_PATH set via {profile}")

        fish_profile = Path("/etc/fish/conf.d/copper.fish")
        fish_profile.parent.mkdir(parents=True, exist_ok=True)
        fish_profile.write_text(fish_block + "\n", encoding="utf-8")
        os.chmod(fish_profile, 0o644)
        ok(f"COPPER_PATH set via {fish_profile}")
    else:
        for name in (".zshrc", ".bashrc", ".profile"):
            f = Path.home() / name
            existing = f.read_text(encoding="utf-8") if f.exists() else ""
            if BLOCK_BEGIN in existing:
                info(f"Already configured: {f}")
            else:
                with f.open("a", encoding="utf-8") as fh:
                    fh.write(("\n" if existing and not existing.endswith("\n") else "") + block + "\n")
                ok(f"Updated: {f}")

        fish_profile = Path.home() / ".config" / "fish" / "conf.d" / "copper.fish"
        fish_profile.parent.mkdir(parents=True, exist_ok=True)
        if fish_profile.exists() and FISH_BLOCK_BEGIN in fish_profile.read_text(encoding="utf-8"):
            info(f"Already configured: {fish_profile}")
        else:
            fish_profile.write_text(fish_block + "\n", encoding="utf-8")
            ok(f"Updated: {fish_profile}")


# --- main ---------------------------------------------------------------
def main():
    ap = argparse.ArgumentParser(prog="install.py", description="Install the Copper compiler (cforge).")
    grp = ap.add_mutually_exclusive_group()
    grp.add_argument("--local", action="store_const", const="local", dest="force",
                     help="force a per-user install (no admin needed)")
    grp.add_argument("--global", action="store_const", const="global", dest="force",
                     help="force an all-users install (needs admin/root)")
    args = ap.parse_args()

    banner("Copper", "Language Installer")
    check_prereqs()

    scope, install_dir = resolve_scope(args.force)
    head(f"{scope.capitalize()} install  →  {install_dir}")

    exe = build_release()
    lson_bin = build_lson()
    copy_payload(exe, install_dir, lson_bin)

    head("Registering COPPER_PATH and PATH")
    if SYS == "Windows":
        register_windows(install_dir, scope)
    else:
        register_unix(install_dir, scope)

    banner("Installation complete", scope)
    info(f"Install dir : {install_dir}")
    info(f"Executable  : {install_dir / 'bin' / exe.name}")
    print()
    print(c("  Next steps", f"{COPPER}"))
    info("Open a NEW terminal so the PATH change is picked up.")
    info("Then run cforge from anywhere:")
    print(c("      cforge run main.crs", DIM))
    print(c("      cforge -c -i main.crs", DIM))
    print(c("      cforge --version", DIM))
    print(c("      alloy run main.crs", DIM))
    uninst = install_dir / ("uninstall.bat" if SYS == "Windows" else "uninstall.py")
    info(f"Uninstall   : {uninst}" + ("  (run as Administrator)" if scope == "global" else ""))
    print()


if __name__ == "__main__":
    try:
        main()
    except PermissionError as e:
        fail(f"Permission denied: {e}")
        warn("A global install needs admin/root. Re-run elevated, or use --local.")
        sys.exit(1)
    except KeyboardInterrupt:
        sys.exit(130)
