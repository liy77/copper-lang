# Changelog

All notable changes to Copper / cforge. Versions follow the project's
CalVer scheme (`0.YY.M`, stamped from the build date).

## [v0.26.6] - 2026-06-03

First tagged release. Copper is a high-level language that transpiles to Rust,
shipped as the `cforge` compiler, with a declarative UI front-end (**MUI**) on
top of the mocida toolkit.

### Added
- **Copper language front-end** — tokenizer, parser and LSP. Variables
  (`mut x = 1`), `func` declarations, classes/structs/impl, imports, loops,
  `match` (guards / `_` / `|`), `if let`/`while let`, vec literals, closures,
  the try operator, optional chaining (`obj?.field`), string interpolation
  (`"$name"` / `"${expr}"`), ternary (`cond ? a : b`), generic return types,
  and `unsafe` blocks/functions.
- **`cforge` compiler** with `run` (transpile + build + run) and `-c`/`--compile`
  (emit Rust), plus the bundled `cstd` standard library.
- **MUI front-end** (`crates/mui-syntax`, `mui-codegen`, `mui-lsp`):
  - `cforge run x.mui` / `.crm` renders a declarative UI live via the `mui-dev`
    host (no transpile).
  - `cforge -c [-r] x.mui` lowers the component AST to a self-contained Rust
    crate (`mui-codegen`), optionally building a native binary; `-b` embeds the
    `app.bundle` assets for a single-file executable.
- **MUI components**: reactive `counter`, `onKeyInput` keyboard handling,
  styling + anchors, reusable components with params + cross-file `import`,
  `app.bundle` assets, and **`Video` / `WebView`** elements (runtime + codegen),
  with cross-platform `Video` corner `radius`.
- **VS Code extension** for Copper + MUI.
- **MUI-based GUI installer** (declarative `.mui`, no Tauri/webview shell) with a
  scope-detecting install flow.
- **Examples** under `examples/copper/` (language features) and `examples/mui/`
  (hello, counter, keyboard, styled, app, card, dashboard, bundle-demo, crm,
  video, webview).
- Cross-platform Python tooling (`scripts/`), CalVer + git-hash versioning, and
  a CI matrix (fmt / clippy / check / test / build) on Linux, Windows, macOS.

### Notes
- MUI handlers and live reactivity in the **codegen** path are still emitted as
  placeholders for now (the dev/runtime path has them); `loop:` is a reserved
  word, so use `repeat:` for video looping.
