#!/usr/bin/env python3
"""
Build & run a MUI file (`.mui` / `.crm`) the way that actually works today.

`cforge run foo.mui` uses the *live* M1 render host (`mui-dev`), which is not
present in the mocida-rs workspace — so it fails with "Could not locate the
`mocida-rs` workspace that provides `mui-dev`". This script takes the **codegen
path (M5)** instead: it lowers the MUI file to a real Rust crate (via
`mui-codegen`), native-builds it against the sibling `mocida` crate, and runs
the resulting binary.

It also wires up `MOCIDA_RS_DIR` automatically when copper-lang and mocida sit
side by side, so codegen can find the `mocida` path dependency.

Usage:
    python scripts/mui.py examples/mui/keyboard/keyboard.mui
    python scripts/mui.py examples/mui/app/app.mui --bundle      # embed app.bundle
    python scripts/mui.py foo.mui --debug-build                  # faster, unoptimised
    python scripts/mui.py foo.mui --no-run                       # build only
    python scripts/mui.py foo.mui -o build                       # custom output base
"""

import argparse
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _pretty import banner, fail, head, info, ok, warn  # noqa: E402

SYS = platform.system()
EXE = ".exe" if SYS == "Windows" else ""
ROOT = Path(__file__).resolve().parent.parent
MUI_EXTS = {".mui", ".crm"}


def find_cforge():
    """Prefer a release build, fall back to debug. None if neither exists."""
    for profile in ("release", "debug"):
        p = ROOT / "target" / profile / f"cforge{EXE}"
        if p.exists():
            return p
    return None


def find_mocida_rs():
    """MOCIDA_RS_DIR env, else common side-by-side layouts. Validated by the
    `mocida-sys` marker (same check cforge's locator uses)."""
    def looks_like(d):
        try:
            return "mocida-sys" in (d / "Cargo.toml").read_text(encoding="utf-8")
        except OSError:
            return False

    env = os.environ.get("MOCIDA_RS_DIR")
    if env and looks_like(Path(env)):
        return Path(env)
    for rel in ("../mocida/mocida-rs", "../../mocida/mocida-rs", "mocida/mocida-rs"):
        cand = (ROOT / rel).resolve()
        if looks_like(cand):
            return cand
    return None


def find_mocida_sdk(mocida_rs):
    """Locate a staged mocida C SDK (headers + prebuilt lib) so mocida-sys can
    compile without the user pre-exporting MOCIDA_INCLUDE_DIR / MOCIDA_LIB_DIR.

    Returns (include_dir, lib_dir) or (None, None). Looks for a dir holding
    `include/uikit/` + a `lib/libmocida.{dylib,so,a}` / `mocida.lib`."""
    libnames = ("libmocida.dylib", "libmocida.so", "libmocida.a", "mocida.lib", "mocida.dll")

    def usable(stage):
        inc, lib = stage / "include", stage / "lib"
        if not (inc / "uikit").is_dir():
            return None
        if not any((lib / n).exists() for n in libnames):
            return None
        return inc, lib

    roots = []
    if mocida_rs:
        roots.append(mocida_rs.parent)  # …/mocida (C repo next to mocida-rs)
    roots.append((ROOT / "../mocida").resolve())
    for root in roots:
        for rel in ("release/stage", "mocida/release/stage", "."):
            hit = usable((root / rel).resolve())
            if hit:
                return hit
    return None, None


def runtime_lib_var():
    """Env var + path-sep the OS uses to find shared libs at run-time."""
    if SYS == "Windows":
        return "PATH", os.pathsep
    if SYS == "Darwin":
        return "DYLD_FALLBACK_LIBRARY_PATH", os.pathsep
    return "LD_LIBRARY_PATH", os.pathsep


def crate_bin_name(cargo_toml):
    """Read the package `name = "..."` from the generated Cargo.toml."""
    for line in cargo_toml.read_text(encoding="utf-8").splitlines():
        s = line.strip()
        if s.startswith("name ="):
            return s.split("=", 1)[1].strip().strip('"')
    return None


def main():
    ap = argparse.ArgumentParser(
        prog="mui.py", description="Build & run a .mui/.crm file via the codegen path."
    )
    ap.add_argument("file", help="the .mui / .crm file to build and run")
    ap.add_argument("-o", "--output", default="dist", help="output base dir (default: dist)")
    ap.add_argument("-b", "--bundle", action="store_true", help="embed app.bundle into the binary")
    ap.add_argument("--debug-build", action="store_true", help="build the crate unoptimised")
    ap.add_argument("--no-run", action="store_true", help="build only, don't launch the binary")
    args = ap.parse_args()

    banner("Copper", "MUI Build & Run")

    src = Path(args.file)
    if not src.exists():
        fail(f"File not found: {src}")
        sys.exit(1)
    if src.suffix.lower() not in MUI_EXTS:
        fail(f"Not a MUI file (.mui/.crm): {src}")
        sys.exit(1)

    cforge = find_cforge()
    if not cforge:
        fail("cforge not built. Run: python scripts/build.py  (or cargo build)")
        sys.exit(1)
    if not shutil.which("cargo"):
        fail("Cargo/Rust not on PATH (https://rustup.rs/).")
        sys.exit(1)

    env = dict(os.environ)
    mocida = find_mocida_rs()
    if mocida:
        env["MOCIDA_RS_DIR"] = str(mocida)
        info(f"mocida-rs: {mocida}")
    else:
        warn("mocida-rs workspace not found; codegen may fail to resolve `mocida`.")
        warn("Set MOCIDA_RS_DIR, or place copper-lang and mocida side by side.")

    # mocida-sys needs the C SDK (headers + prebuilt lib). Honor anything the
    # user already exported; otherwise auto-detect a staged SDK and prefer the
    # dynamic lib (the .dylib/.so self-resolves its system frameworks, so the
    # final link stays clean). Stash the lib dir for the run step's loader path.
    lib_dir = None
    if env.get("MOCIDA_INCLUDE_DIR") and env.get("MOCIDA_LIB_DIR"):
        lib_dir = Path(env["MOCIDA_LIB_DIR"])
        info(f"mocida SDK (from env): {env['MOCIDA_INCLUDE_DIR']}")
    else:
        inc, lib_dir = find_mocida_sdk(mocida)
        if inc and lib_dir:
            env["MOCIDA_INCLUDE_DIR"] = str(inc)
            env["MOCIDA_LIB_DIR"] = str(lib_dir)
            env.setdefault("MOCIDA_STATIC", "0")  # dynamic: clean linking
            info(f"mocida SDK: {inc.parent}")
        else:
            warn("mocida C SDK not found; mocida-sys will need MOCIDA_INCLUDE_DIR /")
            warn("MOCIDA_LIB_DIR (point them at a staged mocida build).")

    # 1. Codegen + native build (the M5 path). -r unless --debug-build.
    head(f"Lowering {src.name} to Rust + native build")
    cmd = [str(cforge), "-c", "-i", str(src), "-o", args.output]
    if not args.debug_build:
        cmd.append("-r")
    if args.bundle:
        cmd.append("-b")
    if subprocess.run(cmd, cwd=str(ROOT), env=env).returncode != 0:
        fail("Codegen / build failed.")
        sys.exit(1)

    out_crate = ROOT / args.output / "mui"
    cargo_toml = out_crate / "Cargo.toml"
    if not cargo_toml.exists():
        fail(f"Expected generated crate at {out_crate} (no Cargo.toml).")
        sys.exit(1)

    if args.debug_build:
        # `cforge -c` without -r only writes source; build it ourselves.
        head("Building crate (debug)")
        if subprocess.run(["cargo", "build"], cwd=str(out_crate), env=env).returncode != 0:
            fail("cargo build failed.")
            sys.exit(1)

    name = crate_bin_name(cargo_toml) or "mui_app"
    profile = "debug" if args.debug_build else "release"
    binary = out_crate / "target" / profile / f"{name}{EXE}"
    if not binary.exists():
        fail(f"Built binary not found: {binary}")
        sys.exit(1)

    ok(f"Binary: {binary}")
    if args.no_run:
        return

    # 2. Run it. Put the mocida lib dir on the OS loader path so the dynamic
    # libmocida (+ SDL3) resolve at startup.
    if lib_dir:
        var, sep = runtime_lib_var()
        prev = env.get(var, "")
        env[var] = str(lib_dir) + (sep + prev if prev else "")
    head(f"Running {name}")
    rc = subprocess.run([str(binary)], cwd=str(out_crate), env=env).returncode
    sys.exit(rc)


if __name__ == "__main__":
    main()
