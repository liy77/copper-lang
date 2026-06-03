#!/usr/bin/env python3
"""
Build, package and install Copper's VSCode editor extensions straight into
your local VSCode — no Marketplace round-trip.

For every extension under editors/ that has a package.json (currently the
Copper `.crs` extension and the MUI `.mui`/`.crm` extension) it:

  1. npm install            — pull the build deps (typescript, language client)
  2. npm run compile        — tsc → out/extension.js
  3. vsce package           — produce a <name>-<version>.vsix
  4. code --install-extension <vsix> --force

The .vsix files are left in each extension's folder so you can also share or
re-install them by hand.

Usage:
    python scripts/install_extensions.py                 # all extensions
    python scripts/install_extensions.py vscode          # only editors/vscode
    python scripts/install_extensions.py vscode vscode-mui
    python scripts/install_extensions.py --package-only  # build .vsix, don't install
    python scripts/install_extensions.py --list          # show what would be built
"""

import argparse
import platform
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _pretty import banner, c, DIM, COPPER, fail, head, info, ok, step, warn  # noqa: E402

SYS = platform.system()
ROOT = Path(__file__).resolve().parent.parent
EDITORS = ROOT / "editors"


def find_tool(name):
    """Resolve an executable, accounting for Windows' .cmd/.exe shims."""
    return shutil.which(name)


def run(cmd, cwd, stdin_text=None):
    """Run a command, streaming output. Returns the exit code."""
    step("$ " + " ".join(Path(c).name if i == 0 else c for i, c in enumerate(cmd)))
    proc = subprocess.run(
        cmd,
        cwd=str(cwd),
        input=stdin_text,
        text=True,
    )
    return proc.returncode


def discover_extensions(filters):
    """All editors/<dir> that have a package.json, optionally filtered by name."""
    found = []
    if EDITORS.is_dir():
        for child in sorted(EDITORS.iterdir()):
            if (child / "package.json").is_file():
                if not filters or child.name in filters:
                    found.append(child)
    return found


def package_and_install(ext_dir, npm, npx, code, package_only):
    import json

    pkg = json.loads((ext_dir / "package.json").read_text(encoding="utf-8"))
    name = pkg.get("name", ext_dir.name)
    version = pkg.get("version", "0.0.0")
    display = pkg.get("displayName", name)
    has_runtime_deps = bool(pkg.get("dependencies"))

    head(f"{display}  ({ext_dir.relative_to(ROOT)})")

    # 1. deps
    if run([npm, "install"], ext_dir) != 0:
        fail(f"npm install failed for {name}.")
        return False

    # 2. compile (tsc → out/). Skip gracefully if there's no compile script.
    if "compile" in pkg.get("scripts", {}):
        if run([npm, "run", "compile"], ext_dir) != 0:
            fail(f"npm run compile failed for {name}.")
            return False
    else:
        info("No 'compile' script — skipping TypeScript build.")

    # 3. package → .vsix
    vsix = ext_dir / f"{name}-{version}.vsix"
    if vsix.exists():
        vsix.unlink()
    vsce_cmd = [npx, "--yes", "@vscode/vsce", "package", "--out", str(vsix)]
    # Bundle prod node_modules only when there are real runtime deps; otherwise
    # tell vsce to skip the dependency scan so it doesn't choke on dev-only trees.
    if not has_runtime_deps:
        vsce_cmd.append("--no-dependencies")
    # Feed "y" so vsce's "missing repository/license" confirmation can't block us.
    if run(vsce_cmd, ext_dir, stdin_text="y\n") != 0 or not vsix.exists():
        fail(f"vsce package failed for {name}.")
        return False
    ok(f"Packaged {vsix.name}")

    # 4. install into VSCode
    if package_only:
        info("--package-only: leaving installation to you.")
        return True
    if not code:
        warn("VSCode 'code' CLI not found on PATH — skipping install.")
        info(f"Install it by hand:  code --install-extension {vsix}")
        return True
    if run([code, "--install-extension", str(vsix), "--force"], ext_dir) != 0:
        fail(f"code --install-extension failed for {name}.")
        return False
    ok(f"Installed {display} into VSCode")
    return True


def main():
    ap = argparse.ArgumentParser(
        prog="install_extensions.py",
        description="Build, package and install the Copper VSCode extensions.",
    )
    ap.add_argument("names", nargs="*", help="extension folder names under editors/ (default: all)")
    ap.add_argument("--package-only", action="store_true", help="build the .vsix but don't install")
    ap.add_argument("--list", action="store_true", help="list discovered extensions and exit")
    args = ap.parse_args()

    banner("Copper", "Editor Extensions")

    extensions = discover_extensions(args.names)
    if not extensions:
        if args.names:
            fail(f"No extension matched {args.names} under {EDITORS}.")
        else:
            fail(f"No packageable extensions found under {EDITORS}.")
        sys.exit(1)

    if args.list:
        head("Discovered extensions")
        for e in extensions:
            info(f"{e.name}  →  {e.relative_to(ROOT)}")
        return

    # prerequisites
    npm = find_tool("npm")
    npx = find_tool("npx")
    code = find_tool("code")
    if not npm or not npx:
        fail("Node.js (npm/npx) is required. Install it from https://nodejs.org/ and retry.")
        sys.exit(1)
    node = find_tool("node")
    if node:
        ver = subprocess.run([node, "--version"], capture_output=True, text=True).stdout.strip()
        ok(f"Node.js {ver}")
    if not code and not args.package_only:
        warn("VSCode 'code' CLI not on PATH — extensions will be packaged but not installed.")
        info("In VSCode: Command Palette → 'Shell Command: Install code command in PATH'.")

    results = {}
    for ext in extensions:
        results[ext.name] = package_and_install(ext, npm, npx, code, args.package_only)

    head("Summary")
    failed = [n for n, okk in results.items() if not okk]
    for n, okk in results.items():
        (ok if okk else fail)(f"{n}: {'done' if okk else 'failed'}")
    if not args.package_only and code and not failed:
        print()
        info("Reload VSCode (Developer: Reload Window) to activate the extensions.")
    print()
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
