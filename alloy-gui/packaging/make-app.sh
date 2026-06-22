#!/usr/bin/env bash
# Empacota o binário `alloy` num Alloy.app para macOS, com ícone (.icns) gerado
# de assets/alloy-icon.png. O app é usável no Finder/Dock E por linha de comando
# (Alloy.app/Contents/MacOS/alloy run x.crs).
#
# Pré-requisitos: cargo build --release (gera o binário + libmocida.dylib),
# `sips` e `iconutil` (vêm com o macOS).
#
# Uso:  alloy-gui/packaging/make-app.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
gui_dir="$(dirname "$here")"
target_dir="${CARGO_TARGET_DIR:-$gui_dir/target}/release"
out="$gui_dir/dist/Alloy.app"
icon_png="$gui_dir/assets/alloy-icon.png"

bin="$target_dir/alloy"
if [[ ! -x "$bin" ]]; then
  echo "binário não encontrado em $bin — rode primeiro:" >&2
  echo "  cargo build --release --manifest-path $gui_dir/Cargo.toml" >&2
  exit 1
fi
if [[ ! -f "$icon_png" ]]; then
  echo "ícone ausente: $icon_png (salve o PNG com a caixa de app)" >&2
  exit 1
fi

echo "==> montando $out"
rm -rf "$out"
mkdir -p "$out/Contents/MacOS" "$out/Contents/Resources"

# 1. Binário + dylib do mocida ao lado dele.
cp "$bin" "$out/Contents/MacOS/alloy"
for dylib in "$target_dir"/*.dylib; do
  [[ -f "$dylib" ]] && cp "$dylib" "$out/Contents/MacOS/"
done

# 2. Ícone: PNG -> .iconset -> .icns.
iconset="$(mktemp -d)/Alloy.iconset"
mkdir -p "$iconset"
for size in 16 32 64 128 256 512; do
  sips -z "$size" "$size"     "$icon_png" --out "$iconset/icon_${size}x${size}.png"     >/dev/null
  sips -z "$((size*2))" "$((size*2))" "$icon_png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$out/Contents/Resources/Alloy.icns"

# 3. Info.plist (GUI por padrão; CLI continua acessível pelo binário interno).
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

echo "==> pronto: $out"
echo "    GUI:  open $out"
echo "    CLI:  $out/Contents/MacOS/alloy run arquivo.crs"
