#!/usr/bin/env python3
"""Cross-compila e empacota o runtime `alloy` para várias plataformas.

O `alloy` vive no crate portátil `alloy-vm` (sem mocida; deps puro-Rust), então
cross-compila limpo. Para cada alvo cujo toolchain está instalado, builda em
release e empacota o binário em `dist/alloy/alloy-<versão>-<target>.{tar.gz|zip}`.
Alvos sem toolchain são pulados com aviso (em vez de falhar) — útil localmente.

Uso:
    python scripts/release-alloy.py            # todos os alvos disponíveis
    python scripts/release-alloy.py --host     # só o alvo do host
"""

import argparse
import os
import shutil
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "dist" / "alloy"

TARGETS = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
]


def alloy_version() -> str:
    cargo = (ROOT / "crates" / "alloy-vm" / "Cargo.toml").read_text(encoding="utf-8")
    for line in cargo.splitlines():
        if line.strip().startswith("version"):
            return line.split("=", 1)[1].strip().strip('"')
    return "0.0.0"


def installed_targets() -> set:
    try:
        out = subprocess.run(
            ["rustup", "target", "list", "--installed"],
            capture_output=True, text=True, check=True,
        ).stdout
        return {t.strip() for t in out.splitlines() if t.strip()}
    except Exception:
        return set()


def host_target() -> str | None:
    try:
        out = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True).stdout
        for line in out.splitlines():
            if line.startswith("host:"):
                return line.split(":", 1)[1].strip()
    except Exception:
        pass
    return None


def build_target(target: str) -> Path | None:
    print(f"==> building alloy for {target}")
    r = subprocess.run(
        ["cargo", "build", "--release", "--target", target, "-p", "alloy-vm", "--bin", "alloy"],
        cwd=ROOT,
    )
    if r.returncode != 0:
        print(f"    !! build falhou para {target} (pulando)")
        return None
    exe = "alloy.exe" if "windows" in target else "alloy"
    bin_path = ROOT / "target" / target / "release" / exe
    return bin_path if bin_path.is_file() else None


def package(target: str, bin_path: Path, version: str):
    DIST.mkdir(parents=True, exist_ok=True)
    stem = f"alloy-{version}-{target}"
    exe_name = bin_path.name
    if "windows" in target:
        out = DIST / f"{stem}.zip"
        with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
            z.write(bin_path, exe_name)
            readme = ROOT / "crates" / "alloy-vm" / "README.md"
            if readme.is_file():
                z.write(readme, "README.md")
    else:
        out = DIST / f"{stem}.tar.gz"
        with tarfile.open(out, "w:gz") as t:
            t.add(bin_path, arcname=exe_name)
            readme = ROOT / "crates" / "alloy-vm" / "README.md"
            if readme.is_file():
                t.add(readme, arcname="README.md")
    print(f"    -> {out.relative_to(ROOT)}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", action="store_true", help="empacota só o alvo do host")
    args = ap.parse_args()

    version = alloy_version()
    print(f"Alloy runtime release v{version}")

    if args.host:
        h = host_target()
        targets = [h] if h else []
    else:
        avail = installed_targets()
        h = host_target()
        if h:
            avail.add(h)
        targets = [t for t in TARGETS if t in avail]
        skipped = [t for t in TARGETS if t not in avail]
        for t in skipped:
            print(f"    (pulado: {t} — toolchain não instalado; `rustup target add {t}`)")

    if not targets:
        print("Nenhum alvo disponível. Instale toolchains com `rustup target add <target>`.")
        return 1

    built = 0
    for t in targets:
        bin_path = build_target(t)
        if bin_path:
            package(t, bin_path, version)
            built += 1

    print(f"\nPronto: {built}/{len(targets)} alvos empacotados em {DIST.relative_to(ROOT)}/")
    return 0 if built else 1


if __name__ == "__main__":
    sys.exit(main())
