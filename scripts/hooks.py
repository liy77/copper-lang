#!/usr/bin/env python3
"""
Activate the repo's pre-commit / pre-push git hooks for this clone. Idempotent.

Cross-platform replacement for install-hooks.bat / install-hooks.sh — it just
points git's core.hooksPath at .githooks/ and (on Unix) makes the hooks
executable.

Usage:
    python scripts/hooks.py
"""

import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _pretty import banner, c, DIM, fail, info, ok  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent


def main():
    banner("Copper", "Git Hooks Setup")

    if not shutil.which("git"):
        fail("git not found on PATH.")
        sys.exit(1)

    if subprocess.run(["git", "config", "core.hooksPath", ".githooks"], cwd=str(ROOT)).returncode != 0:
        fail("Failed to set core.hooksPath.")
        sys.exit(1)

    if platform.system() != "Windows":
        hooks = ROOT / ".githooks"
        if hooks.is_dir():
            for h in hooks.iterdir():
                if h.is_file():
                    os.chmod(h, 0o755)

    ok("Hooks activated from .githooks/")
    info("pre-commit : cargo fmt --check + cargo clippy --all-targets -D warnings")
    info("pre-push   : cargo test")
    print()
    print(c("  Bypass with --no-verify only if you really must:", DIM))
    print(c("      git commit --no-verify", DIM))
    print(c("      git push   --no-verify", DIM))
    print()


if __name__ == "__main__":
    main()
