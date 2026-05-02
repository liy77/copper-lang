#!/usr/bin/env sh
# Activate the repo's pre-commit / pre-push hooks for this clone. Idempotent.

set -e

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")" && pwd)"
cd "$SCRIPT_DIR/.."

if ! command -v git >/dev/null 2>&1; then
    echo "[ERROR] git not found in PATH."
    exit 1
fi

git config core.hooksPath .githooks
chmod +x .githooks/* 2>/dev/null || true

echo "[SUCCESS] Hooks activated from .githooks/"
echo "  pre-commit  : cargo fmt --check + cargo clippy -D warnings"
echo "  pre-push    : cargo test"
echo
echo "Bypass with --no-verify only if you really must:"
echo "  git commit --no-verify"
echo "  git push   --no-verify"
