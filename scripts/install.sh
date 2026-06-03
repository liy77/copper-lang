#!/usr/bin/env bash
# Thin wrapper: all install logic lives in install.py (it auto-detects Linux
# vs macOS). This just finds a Python 3 interpreter and forwards every arg.
#   ./install.sh            # auto scope from privileges
#   ./install.sh --local    # per-user install
#   sudo ./install.sh       # all-users install
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if command -v python3 >/dev/null 2>&1; then PY=python3
elif command -v python >/dev/null 2>&1; then PY=python
else echo "Python 3 not found on PATH (tried python3, python). Install it and retry." >&2; exit 1
fi
exec "$PY" "$DIR/install.py" "$@"
