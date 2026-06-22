# Assets do Alloy GUI

Coloque aqui os ícones (o app os referencia via `app.bundle` e `build.rs`):

- **`alloy-icon.png`** — ícone com a caixa de app (fonte do ícone de janela,
  do `.ico` Windows e do `.icns` macOS). Origem: `assets/alloy/alloy-icon.png`
  do repo.
- **`alloy-icon.ico`** — gerado do PNG para o `build.rs` embutir no `.exe`
  (Windows). Gere com, p.ex., ImageMagick:
  `magick alloy-icon.png -define icon:auto-resize=256,128,64,48,32,16 alloy-icon.ico`

No macOS o `.icns` é gerado automaticamente pelo `packaging/make-app.sh` a
partir do `alloy-icon.png` (via `sips`/`iconutil`) — não precisa commitar.
