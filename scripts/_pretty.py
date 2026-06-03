#!/usr/bin/env python3
"""
Shared terminal styling for Copper's Python tooling.

Every script in scripts/ (except the self-contained uninstaller, which has
to run after being copied away from this module) imports these helpers so
the install / build / cleanup / diagnose output all share one look:

    from _pretty import banner, head, step, info, ok, warn, fail, c, COPPER

The palette is copper-themed (256-colour orange accent) and degrades to
plain text when stdout is not a TTY or NO_COLOR is set. Box/glyph characters
fall back to ASCII when the console can't encode UTF-8.
"""

import os
import platform
import sys

_SYS = platform.system()


# Reconfigure stdout/stderr to UTF-8 up front (Python 3.7+). On modern Windows
# terminals this makes the box-drawing glyphs render; if it fails we fall back
# to ASCII below.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")
    except Exception:
        pass


def _supports_color():
    if os.environ.get("NO_COLOR"):
        return False
    if not sys.stdout.isatty():
        return False
    if _SYS == "Windows":
        os.system("")  # enable ANSI VT processing on Windows 10+ consoles
    return True


def _supports_unicode():
    enc = (getattr(sys.stdout, "encoding", "") or "").lower()
    return "utf" in enc


_COLOR = _supports_color()
_UNI = _supports_unicode()

# --- palette ------------------------------------------------------------
COPPER = "38;5;208"   # warm copper orange — the brand accent
DIM = "38;5;245"      # muted grey for secondary text
GREEN = "32"
YELLOW = "33"
RED = "31"
CYAN = "36"
BOLD = "1"

# --- glyphs (UTF-8 with ASCII fallback) ---------------------------------
G_TL, G_TR, G_BL, G_BR, G_H, G_V = ("╭", "╮", "╰", "╯", "─", "│") if _UNI else ("+", "+", "+", "+", "-", "|")
G_ARROW = "▸" if _UNI else ">"
G_BULLET = "·" if _UNI else "-"
G_OK = "✔" if _UNI else "+"
G_WARN = "!"
G_FAIL = "✗" if _UNI else "x"


def c(text, code):
    """Wrap text in an ANSI SGR sequence (no-op when colour is disabled)."""
    return f"\033[{code}m{text}\033[0m" if _COLOR else text


# --- structured output --------------------------------------------------
def banner(title, subtitle=None):
    """A rounded box header in the copper accent colour."""
    line = f"  {title}"
    if subtitle:
        line += f"  {G_BULLET}  {subtitle}"
    width = max(len(line) + 2, 40)
    top = G_TL + G_H * width + G_TR
    bot = G_BL + G_H * width + G_BR
    print()
    print(c(top, COPPER))
    print(c(G_V, COPPER) + c(line.ljust(width), f"{COPPER};{BOLD}") + c(G_V, COPPER))
    print(c(bot, COPPER))
    print()


def head(msg):
    """A section header — copper arrow + bold text."""
    print(c(f"\n{G_ARROW} {msg}", f"{COPPER};{BOLD}"))


def step(msg):
    """A neutral progress line."""
    print(c(f"  {G_BULLET} {msg}", DIM))


def info(msg):
    print(c(f"  {G_BULLET} {msg}", "37"))


def ok(msg):
    print(c(f"  {G_OK} {msg}", GREEN))


def warn(msg):
    print(c(f"  {G_WARN} {msg}", YELLOW))


def fail(msg):
    print(c(f"  {G_FAIL} {msg}", RED))


def rule():
    print(c("  " + G_H * 38, DIM))
