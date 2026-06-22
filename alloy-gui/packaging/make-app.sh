#!/usr/bin/env bash
# Packages the `alloy` binary into an Alloy.app bundle for macOS, with an icon
# (.icns) generated from assets/alloy-icon.png. The app works in Finder/Dock
# AND on the command line (Alloy.app/Contents/MacOS/alloy run x.crs).
#
# Prerequisites: cargo build --release (produces the binary + libmocida.dylib),
# `sips` and `iconutil` (both ship with macOS).
#
# Usage:  alloy-gui/packaging/make-app.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
gui_dir="$(dirname "$here")"
target_dir="${CARGO_TARGET_DIR:-$gui_dir/target}/release"
out="$gui_dir/dist/Alloy.app"
icon_png="$gui_dir/assets/alloy-icon.png"

bin="$target_dir/alloy"
if [[ ! -x "$bin" ]]; then
  echo "binary not found at $bin — build it first:" >&2
  echo "  cargo build --release --manifest-path $gui_dir/Cargo.toml" >&2
  exit 1
fi
if [[ ! -f "$icon_png" ]]; then
  echo "icon missing: $icon_png (save the PNG with the app box)" >&2
  exit 1
fi

echo "==> assembling $out"
rm -rf "$out"
mkdir -p "$out/Contents/MacOS" "$out/Contents/Resources"

# 1. Binary + mocida dylib beside it.
cp "$bin" "$out/Contents/MacOS/alloy"
for dylib in "$target_dir"/*.dylib; do
  [[ -f "$dylib" ]] && cp "$dylib" "$out/Contents/MacOS/"
done

# 2. Icon: PNG -> .iconset -> .icns.
iconset="$(mktemp -d)/Alloy.iconset"
mkdir -p "$iconset"
for size in 16 32 64 128 256 512; do
  sips -z "$size" "$size"     "$icon_png" --out "$iconset/icon_${size}x${size}.png"     >/dev/null
  sips -z "$((size*2))" "$((size*2))" "$icon_png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$out/Contents/Resources/Alloy.icns"

# 3. Info.plist (GUI by default; CLI is still accessible via the inner binary).
cat > "$out/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Alloy</string>
  <key>CFBundleDisplayName</key><string>Alloy</string>
  <key>CFBundleIdentifier</key><string>net.liy77.alloy</string>
  <key>CFBundleExecutable</key><string>alloy</string>
  <key>CFBundleIconFile</key><string>Alloy</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

echo "==> done: $out"
echo "    GUI:  open $out"
echo "    CLI:  $out/Contents/MacOS/alloy run file.crs"
