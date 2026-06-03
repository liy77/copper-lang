#!/usr/bin/env python3
"""
Remove development artifacts (target/, dist/, stray debug logs) from the
project root, leaving everything needed for distribution in place.

Cross-platform replacement for the old cleanup.bat.

Usage:
    python scripts/cleanup.py
"""

import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _pretty import banner, c, DIM, head, info, ok  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent

RETAINED = [
    ("Cargo.toml, Cargo.lock, build.rs", "Rust project metadata"),
    ("src/", "compiler source"),
    ("scripts/", "install / build / diagnose tools"),
    ("docs/", "installation guide"),
    ("lson/, std/", "runtime assets"),
    ("properties.kson", "project configuration"),
    ("examples/", ".crs sample programs"),
    ("main.crs", "default file for `cforge run`"),
]


def remove(path, label):
    if path.is_dir():
        shutil.rmtree(path, ignore_errors=True)
        ok(f"{label} removed")
    elif path.exists():
        path.unlink()
        ok(f"{label} removed")
    else:
        info(f"{label} already absent")


def main():
    banner("Copper", "Project Cleanup")
    info(f"Cleaning development artifacts in {ROOT}")

    head("Removing build output")
    remove(ROOT / "target", "target/")
    remove(ROOT / "dist", "dist/")
    remove(ROOT / "cforge_tokenizer_debug.log", "cforge_tokenizer_debug.log")

    head("Retained for distribution")
    for name, why in RETAINED:
        print(c(f"  · {name:<34}", "37") + c(why, DIM))

    banner("Cleanup complete")
    info("To distribute: archive this directory; users run python scripts/install.py.")
    print()


if __name__ == "__main__":
    main()
