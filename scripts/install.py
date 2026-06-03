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
  5. Copy cforge + Cargo.toml + std/ + lson/ into the install dir.
  6. Register COPPER_PATH and add %COPPER_PATH%/bin to PATH:
        Windows -> HKCU/HKLM registry (REG_EXPAND_SZ) + a settings broadcast
        Unix    -> /etc/profile.d/copper.sh  or  a managed block in your rc files
  7. Drop uninstall.py (+ a uninstall.bat shim on Windows) next to the install.

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
    head("Building Copper in release mode (cargo build --release)")
    if subprocess.run(["cargo", "build", "--release"], cwd=str(ROOT)).returncode != 0:
        fail("cargo build --release failed.")
        sys.exit(1)
    if not exe.exists():
        fail(f"Build reported success but {exe.name} is missing.")
        sys.exit(1)
    ok(f"Built {exe}")
    return exe


# --- file install -------------------------------------------------------
def copy_payload(exe, install_dir):
    bin_dir = install_dir / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)

    dst_exe = bin_dir / exe.name
    shutil.copy2(exe, dst_exe)
    if SYS != "Windows":
        os.chmod(dst_exe, 0o755)
    ok(f"Installed {exe.name}")

    if (ROOT / "Cargo.toml").exists():
        shutil.copy2(ROOT / "Cargo.toml", install_dir / "Cargo.toml")
        ok("Installed project metadata (Cargo.toml)")

    for d in ("lson", "std"):
        src = ROOT / d
        if src.is_dir():
            dst = install_dir / d
            if dst.exists():
                shutil.rmtree(dst)
            shutil.copytree(src, dst)
            ok(f"Installed {d}/")

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

    if scope == "global":
        profile = Path("/etc/profile.d/copper.sh")
        profile.parent.mkdir(parents=True, exist_ok=True)
        profile.write_text(block + "\n", encoding="utf-8")
        os.chmod(profile, 0o644)
        ok(f"COPPER_PATH set via {profile}")
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
    copy_payload(exe, install_dir)

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
