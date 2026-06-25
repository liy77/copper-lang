#!/usr/bin/env python3
"""
Copper Language uninstaller — cross-platform and self-contained.

This file is copied into the install directory by install.py, so it must NOT
import the shared _pretty module (it runs far from scripts/). It re-detects
the install scope the same way the installer chose it:

    admin / root  -> global  (C:\\Program Files\\Copper  |  /usr/local/lib/copper)
    normal user   -> local   (%USERPROFILE%\\.copper      |  $HOME/.copper)

It then removes the install tree, drops COPPER_PATH/bin from PATH, and clears
the COPPER_PATH variable. On Windows the PATH edit is done through the registry
(filtering the literal %COPPER_PATH%\\bin marker); on Unix it strips the managed
block from /etc/profile.d/copper.sh, fish conf.d, or your shell rc files.

Usage:
    python uninstall.py
    python uninstall.py --local | --global    # override the auto-detected scope
"""

import argparse
import os
import platform
import shutil
import sys
from pathlib import Path

SYS = platform.system()

BLOCK_BEGIN = "# >>> COPPER PATH (copper-lang) >>>"
BLOCK_END = "# <<< COPPER PATH <<<"
FISH_BLOCK_BEGIN = "# >>> COPPER PATH (copper-lang, fish) >>>"
FISH_BLOCK_END = "# <<< COPPER PATH (copper-lang, fish) <<<"

# --- minimal styling (standalone copy of _pretty's essentials) ----------
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")
    except Exception:
        pass

_COLOR = (not os.environ.get("NO_COLOR")) and sys.stdout.isatty()
if _COLOR and SYS == "Windows":
    os.system("")
_UNI = "utf" in ((getattr(sys.stdout, "encoding", "") or "").lower())
COPPER, GREEN, YELLOW, RED, DIM = "38;5;208", "32", "33", "31", "38;5;245"
_TL, _TR, _BL, _BR, _H, _V = ("╭", "╮", "╰", "╯", "─", "│") if _UNI else ("+", "+", "+", "+", "-", "|")
_AR, _BU, _OK, _NO = ("▸", "·", "✔", "✗") if _UNI else (">", "-", "+", "x")


def _c(t, code):
    return f"\033[{code}m{t}\033[0m" if _COLOR else t


def banner(title, subtitle=None):
    line = f"  {title}" + (f"  {_BU}  {subtitle}" if subtitle else "")
    width = max(len(line) + 2, 40)
    print("\n" + _c(_TL + _H * width + _TR, COPPER))
    print(_c(_V, COPPER) + _c(line.ljust(width), f"{COPPER};1") + _c(_V, COPPER))
    print(_c(_BL + _H * width + _BR, COPPER) + "\n")


def head(m): print(_c(f"\n{_AR} {m}", f"{COPPER};1"))
def info(m): print(_c(f"  {_BU} {m}", "37"))
def ok(m):   print(_c(f"  {_OK} {m}", GREEN))
def warn(m): print(_c(f"  ! {m}", YELLOW))
def fail(m): print(_c(f"  {_NO} {m}", RED))


# --- privilege / scope --------------------------------------------------
def is_admin():
    if SYS == "Windows":
        try:
            import ctypes
            return ctypes.windll.shell32.IsUserAnAdmin() != 0
        except Exception:
            return False
    return os.geteuid() == 0


def resolve_scope(force):
    admin = is_admin()
    glob = (force == "global") or (force is None and admin)
    if glob and not admin:
        fail("Global uninstall needs administrator / root privileges. Re-run elevated.")
        sys.exit(1)
    if SYS == "Windows":
        install_dir = Path(r"C:\Program Files\Copper") if glob else Path(os.environ["USERPROFILE"]) / ".copper"
    else:
        install_dir = Path("/usr/local/lib/copper") if glob else Path.home() / ".copper"
    return ("global" if glob else "local"), install_dir


# --- removal ------------------------------------------------------------
def remove_tree(install_dir):
    head("Removing installation directory")
    if not install_dir.exists():
        info("Installation directory not found.")
        return
    for child in ("Cargo.toml", "bin", "lson", "std", "uninstall.bat", "uninstall.py"):
        p = install_dir / child
        try:
            if p.is_dir():
                shutil.rmtree(p, ignore_errors=True)
            elif p.exists():
                p.unlink()
        except Exception as e:
            warn(f"Could not remove {p}: {e}")
    try:
        install_dir.rmdir()
        ok("Installation directory removed.")
    except OSError:
        warn(f"Some files remain in {install_dir} (likely in use).")


def clean_path_windows(scope):
    import winreg

    head("Removing %COPPER_PATH%\\bin from PATH")
    if scope == "global":
        root, subkey = winreg.HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"
    else:
        root, subkey = winreg.HKEY_CURRENT_USER, "Environment"

    expanded = os.path.expandvars(r"%COPPER_PATH%\bin")
    with winreg.OpenKey(root, subkey, 0, winreg.KEY_READ | winreg.KEY_WRITE) as key:
        try:
            cur, typ = winreg.QueryValueEx(key, "PATH")
            kept = [p for p in cur.split(";") if p and p != r"%COPPER_PATH%\bin" and p != expanded]
            winreg.SetValueEx(key, "PATH", 0, typ, ";".join(kept))
            ok("PATH cleaned.")
        except FileNotFoundError:
            info("No PATH value to clean.")

        head("Removing COPPER_PATH variable")
        try:
            winreg.DeleteValue(key, "COPPER_PATH")
            ok("COPPER_PATH removed.")
        except FileNotFoundError:
            info("COPPER_PATH was already absent.")
    _broadcast_setting_change()


def _broadcast_setting_change():
    try:
        import ctypes
        ctypes.windll.user32.SendMessageTimeoutW(0xFFFF, 0x1A, 0, "Environment", 0x2, 5000, None)
    except Exception:
        pass


def clean_path_unix(scope):
    head("Removing COPPER_PATH from your shell environment")
    if scope == "global":
        profile = Path("/etc/profile.d/copper.sh")
        if profile.exists():
            profile.unlink()
            ok(f"Removed {profile}")
        else:
            info(f"{profile} not found.")

        fish_profile = Path("/etc/fish/conf.d/copper.fish")
        if fish_profile.exists():
            text = fish_profile.read_text(encoding="utf-8")
            if FISH_BLOCK_BEGIN in text and FISH_BLOCK_END in text:
                fish_profile.unlink()
                ok(f"Removed {fish_profile}")
            else:
                warn(f"{fish_profile} exists but doesn't look managed by Copper; leaving it in place.")
        else:
            info(f"{fish_profile} not found.")
        return

    for name in (".zshrc", ".bashrc", ".profile"):
        f = Path.home() / name
        if not f.exists():
            continue
        lines = f.read_text(encoding="utf-8").splitlines(keepends=True)
        out, skip = [], False
        for ln in lines:
            if ln.strip() == BLOCK_BEGIN:
                skip = True
                continue
            if ln.strip() == BLOCK_END:
                skip = False
                continue
            if not skip:
                out.append(ln)
        if len(out) != len(lines):
            f.write_text("".join(out), encoding="utf-8")
            ok(f"Cleaned {f}")

    fish_profile = Path.home() / ".config" / "fish" / "conf.d" / "copper.fish"
    if fish_profile.exists():
        text = fish_profile.read_text(encoding="utf-8")
        if FISH_BLOCK_BEGIN in text and FISH_BLOCK_END in text:
            fish_profile.unlink()
            ok(f"Removed {fish_profile}")


# --- main ---------------------------------------------------------------
def main():
    ap = argparse.ArgumentParser(prog="uninstall.py", description="Remove the Copper compiler (cforge).")
    grp = ap.add_mutually_exclusive_group()
    grp.add_argument("--local", action="store_const", const="local", dest="force")
    grp.add_argument("--global", action="store_const", const="global", dest="force")
    args = ap.parse_args()

    banner("Copper", "Language Uninstaller")
    scope, install_dir = resolve_scope(args.force)
    info(f"Detected {scope} installation")
    info(f"Target directory: {install_dir}")

    remove_tree(install_dir)
    if SYS == "Windows":
        clean_path_windows(scope)
    else:
        clean_path_unix(scope)

    banner("Copper uninstalled")
    info("Open a new terminal so the PATH change takes effect.")
    print()


if __name__ == "__main__":
    try:
        main()
    except PermissionError as e:
        fail(f"Permission denied: {e}")
        warn("A global uninstall needs admin/root. Re-run elevated.")
        sys.exit(1)
    except KeyboardInterrupt:
        sys.exit(130)
