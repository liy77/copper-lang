# Alloy GUI Assets

Place icons here (the app references them via `app.bundle` and `build.rs`):

- **`alloy-icon.png`** — icon with the app box (source for the window icon,
  the Windows `.ico`, and the macOS `.icns`). Origin: `assets/alloy/alloy-icon.png`
  in the repo.
- **`alloy-icon.ico`** — generated from the PNG for `build.rs` to embed in the
  `.exe` (Windows). Generate with, e.g., ImageMagick:
  `magick alloy-icon.png -define icon:auto-resize=256,128,64,48,32,16 alloy-icon.ico`

On macOS the `.icns` is generated automatically by `packaging/make-app.sh` from
`alloy-icon.png` (via `sips`/`iconutil`) — no need to commit it.
