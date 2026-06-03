#!/usr/bin/env python3
"""
Build the Copper compiler (cforge) into target/release/ without installing it.

Cross-platform replacement for the old build.bat.

Usage:
    python scripts/build.py            # release build
    python scripts/build.py --debug    # debug build (faster, unoptimised)
"""

import argparse
import platform
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _pretty import banner, c, DIM, fail, head, info, ok  # noqa: E402

SYS = platform.system()
ROOT = Path(__file__).resolve().parent.parent


def main():
    ap = argparse.ArgumentParser(prog="build.py", description="Build cforge (no install).")
    ap.add_argument("--debug", action="store_true", help="debug build instead of release")
    args = ap.parse_args()

    banner("Copper", "Language Builder")
    info(f"Working directory: {ROOT}")

    if not (ROOT / "Cargo.toml").exists():
        fail(f"Cargo.toml not found at {ROOT}.")
        sys.exit(1)
    if not shutil.which("cargo"):
        fail("Cargo/Rust is not installed or not on PATH (https://rustup.rs/).")
        sys.exit(1)

    profile = "debug" if args.debug else "release"
    head(f"Building Copper ({profile})")
    cmd = ["cargo", "build"] + ([] if args.debug else ["--release"])
    if subprocess.run(cmd, cwd=str(ROOT)).returncode != 0:
        fail("Build failed.")
        sys.exit(1)

    exe = ROOT / "target" / profile / ("cforge.exe" if SYS == "Windows" else "cforge")
    banner("Build complete")
    ok(f"Executable: {exe}")
    print()
    print(c("  You can now", DIM))
    info("Install it:   python scripts/install.py")
    info(f"Or run it:    {exe} run main.crs")
    print()


if __name__ == "__main__":
    main()
