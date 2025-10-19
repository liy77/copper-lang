#!/usr/bin/env bash
set -euo pipefail

echo "========================================"
echo "    Copper Language Installer"
echo "========================================"
echo

# Change to the script's directory
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Check if Cargo.toml exists in current directory
if [[ ! -f "Cargo.toml" ]]; then
  echo "[ERROR] Cargo.toml not found in current directory."
  echo "Please run this installer from the copper-lang project root directory."
  read -r -p "Press any key to exit..." -n 1 || true; echo
  exit 1
fi

# Check if Rust is installed
if ! command -v cargo >/dev/null 2>&1; then
  echo "[ERROR] Cargo/Rust is not installed or not in PATH."
  echo "Please install Rust from https://rustup.rs/ and try again."
  read -r -p "Press any key to exit..." -n 1 || true; echo
  exit 1
fi

echo "[INFO] Checking Rust installation..."
cargo --version
echo

# Check if running as administrator (root)
if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
  echo "[INFO] Running as Administrator: YES"
  echo "[INFO] Will install globally for all users"
  INSTALL_DIR="/usr/local/lib/copper"
  REG_KEY="(profile.d)"
  INSTALL_TYPE="global"
else
  echo "[INFO] Running as Administrator: NO"
  echo "[INFO] Will install locally for current user"
  INSTALL_DIR="${HOME}/.copper"
  REG_KEY="(~/.shell-rc)"
  INSTALL_TYPE="local"
fi

BIN_DIR="$INSTALL_DIR/bin"
echo "[INFO] Installation directory: $INSTALL_DIR"
echo

# Create installation directory
echo "[INFO] Creating installation directory..."
mkdir -p "$INSTALL_DIR" || { echo "[ERROR] Failed to create installation directory."; read -r -p "Press any key to exit..." -n 1 || true; echo; exit 1; }

mkdir -p "$BIN_DIR" || { echo "[ERROR] Failed to create bin directory."; read -r -p "Press any key to exit..." -n 1 || true; echo; exit 1; }

# Build the project in release mode
echo "[INFO] Building Copper in release mode..."
if ! cargo build --release; then
  echo "[ERROR] Failed to build the project."
  read -r -p "Press any key to exit..." -n 1 || true; echo
  exit 1
fi

# Copy the executable
echo "[INFO] Installing cforge executable..."
if [[ ! -f "target/release/cforge" ]]; then
  echo "[ERROR] Built executable not found at target/release/cforge."
  read -r -p "Press any key to exit..." -n 1 || true; echo
  exit 1
fi
cp -f "target/release/cforge" "$BIN_DIR/cforge" || { echo "[ERROR] Failed to copy executable."; read -r -p "Press any key to exit..." -n 1 || true; echo; exit 1; }
chmod 755 "$BIN_DIR/cforge"

# Copy Cargo.toml for version detection
echo "[INFO] Installing project metadata..."
cp -f "Cargo.toml" "$INSTALL_DIR/Cargo.toml" 2>/dev/null || true

# Copy the lson directory
echo "[INFO] Installing lson dependencies..."
if [[ -d "lson" ]]; then
  rm -rf "$INSTALL_DIR/lson"
  if ! cp -R "lson" "$INSTALL_DIR/lson"; then
    echo "[ERROR] Failed to copy lson directory."
    read -r -p "Press any key to exit..." -n 1 || true; echo
    exit 1
  fi
fi

# Copy std directory
echo "[INFO] Installing standard library..."
if [[ -d "std" ]]; then
  rm -rf "$INSTALL_DIR/std"
  if ! cp -R "std" "$INSTALL_DIR/std"; then
    echo "[ERROR] Failed to copy std directory."
    read -r -p "Press any key to exit..." -n 1 || true; echo
    exit 1
  fi
fi

# Set COPPER_PATH environment variable and add to PATH
echo "[INFO] Setting COPPER_PATH environment variable..."
BLOCK=$'# >>> COPPER PATH (copper-lang) >>>\nexport COPPER_PATH="'$INSTALL_DIR$'"\ncase ":$PATH:" in\n  *:"$COPPER_PATH/bin":*) ;;\n  *) export PATH="$PATH:$COPPER_PATH/bin" ;;\nesac\n# <<< COPPER PATH <<<'

if [[ "$INSTALL_TYPE" == "global" ]]; then
  # Global: place an env script in /etc/profile.d
  PROFILE_DIR="/etc/profile.d"
  PROFILE_FILE="$PROFILE_DIR/copper.sh"
  mkdir -p "$PROFILE_DIR"
  printf "%s\n" "$BLOCK" | tee "$PROFILE_FILE" >/dev/null
  chmod 644 "$PROFILE_FILE"
  echo "[SUCCESS] COPPER_PATH set to $INSTALL_DIR"
  echo "[INFO] Adding %COPPER_PATH%\\bin to PATH... (via /etc/profile.d)"
  echo "[SUCCESS] Added."
else
  # Local: append to common user shell profiles (avoid duplicating)
  add_block_if_missing() {
    local f="$1"
    touch "$f"
    if ! grep -q ">>> COPPER PATH (copper-lang) >>>" "$f"; then
      printf "\n%s\n" "$BLOCK" >> "$f"
      echo "[INFO] Updated: $f"
    else
      echo "[INFO] Already configured: $f"
    fi
  }
  add_block_if_missing "${HOME}/.bashrc"
  add_block_if_missing "${HOME}/.profile"
  add_block_if_missing "${HOME}/.zshrc"
  echo "[SUCCESS] COPPER_PATH set to $INSTALL_DIR"
  echo "[INFO] Adding %COPPER_PATH%\\bin to PATH... (in your shell profile)"
  echo "[SUCCESS] Added."
fi

# Create uninstaller
echo "[INFO] Creating uninstaller..."
UNINSTALL_PATH="$INSTALL_DIR/uninstall.sh"
cat > "$UNINSTALL_PATH" <<'UNINSTALL'
#!/usr/bin/env bash
set -euo pipefail

echo "========================================"
echo "    Copper Language Uninstaller"
echo "========================================"
echo

INSTALL_DIR="__INSTALL_DIR__"
INSTALL_TYPE="__INSTALL_TYPE__"

if [[ "$INSTALL_TYPE" == "global" ]]; then
  if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
    echo "[ERROR] This uninstaller requires administrator privileges."
    echo "Please run as administrator and try again (e.g., sudo)."
    read -r -p "Press any key to exit..." -n 1 || true; echo
    exit 1
  fi
fi

echo "[INFO] Removing installation directory..."
if [[ -d "$INSTALL_DIR" ]]; then
  echo "[INFO] Removing files..."
  [[ -f "$INSTALL_DIR/Cargo.toml" ]] && rm -f "$INSTALL_DIR/Cargo.toml"
  [[ -d "$INSTALL_DIR/bin" ]] && rm -rf "$INSTALL_DIR/bin"
  [[ -d "$INSTALL_DIR/lson" ]] && rm -rf "$INSTALL_DIR/lson"
  [[ -d "$INSTALL_DIR/std"  ]] && rm -rf "$INSTALL_DIR/std"
  rmdir "$INSTALL_DIR" 2>/dev/null || true
  echo "[SUCCESS] Installation directory removed (some files may remain if in use)."
else
  echo "[INFO] Installation directory not found."
fi

echo
echo "[INFO] Removing from PATH..."
if [[ "$INSTALL_TYPE" == "global" ]]; then
  if [[ -f "/etc/profile.d/copper.sh" ]]; then
    rm -f "/etc/profile.d/copper.sh"
    echo "[SUCCESS] Removed from PATH."
  else
    echo "[INFO] /etc/profile.d/copper.sh not found."
  fi
else
  clean_file() {
    local f="$1"
    [[ -f "$f" ]] || return 0
    awk '
      BEGIN{skip=0}
      />>> COPPER PATH \(copper-lang\) >>>/{skip=1}
      /<<< COPPER PATH <<</{skip=0; next}
      skip==0{print}
    ' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
  }
  clean_file "${HOME}/.bashrc"
  clean_file "${HOME}/.profile"
  clean_file "${HOME}/.zshrc"
  echo "[SUCCESS] Removed from PATH."
fi

echo
echo "[INFO] Removing COPPER_PATH environment variable..."
# (Handled implicitly by removing the profile block.)

echo
echo "[SUCCESS] Copper Language has been uninstalled."
echo "Please restart your terminal to apply PATH changes."
read -r -p "Press any key to exit..." -n 1 || true; echo
UNINSTALL
chmod +x "$UNINSTALL_PATH"
sed -i'' -e "s|__INSTALL_DIR__|$INSTALL_DIR|g" -e "s|__INSTALL_TYPE__|$INSTALL_TYPE|g" "$UNINSTALL_PATH"

echo
echo "========================================"
echo "    Installation Complete!"
echo "========================================"
echo
echo "Installation type: $INSTALL_TYPE"
echo "Installation directory: $INSTALL_DIR"
echo "Executable location: $BIN_DIR/cforge"
echo
echo "[SUCCESS] Copper Language (cforge) has been installed successfully!"
echo
echo "IMPORTANT:"
echo "- Please restart your terminal to apply PATH changes"
echo "- After restart, you can use 'cforge' command from anywhere"
echo "- To uninstall, run: $UNINSTALL_PATH"
if [[ "$INSTALL_TYPE" == "global" ]]; then
  echo "  (as administrator)"
fi
echo
echo "Usage examples:"
echo "  cforge run main.crs"
echo "  cforge -c -i main.crs"
echo "  cforge run myproject.crs -o custom_output"
echo
read -r -p "Press any key to exit..." -n 1 || true; echo
