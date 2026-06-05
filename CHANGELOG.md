# Changelog

All notable changes to Copper / cforge. Versions follow the project's
CalVer scheme (`0.YY.M`, stamped from the build date).

## [Unreleased]

### Added
- **`cforge format`** — a pretty-printer for MUI (`.mui` / `.crm`) files. Tokenizes
  the source (keeping comments + strings verbatim), builds a shallow node tree and
  re-emits it with a **fit-or-break** rule: an element call `Name(prop: v, …)` that
  fits in 100 cols stays on one line, otherwise every argument breaks onto its own
  line; `{ … }` child/view bodies always break into indented statements; short
  inline handlers (`onClick: { x += 1 }`) stay inline, long ones (e.g. a big
  `onKeyInput`) break with the closure param kept on the brace line. Comments and
  string literals are reproduced byte-for-byte and the output is idempotent. Works
  on a file or a directory (recurses, skipping `dist`/`target`/`.git`); `--check`
  reports files that would change (exit 1) without writing, `--stdout` prints the
  result. Implementation: `src/cforge/mui_fmt.rs`.
- **`cforge build <file>`** — a friendly alias for `cforge -c -i <file>`. Accepts
  the same flags (`--release`, `--clean`, `--bundle`, `--output`, `--target`,
  `--verbose`), so `cforge build app.mui --release --clean` behaves exactly like
  `cforge -c -i app.mui -r --clean`. The original `-c -i` form is unchanged.
- **Copper: optional return-type sugar** — `func Type? name(...)` lowers to
  `-> Option<Type>` (the tokenizer folds the trailing `?` into the return-type
  token, so `convert_type` resolves it).
- **MUI: computed dimension props** — `width` / `height` now resolve
  `Window.width` / `Window.height` (and `Screen.*`), arithmetic on them
  (`Window.height - 40`), and another widget's declared size by id
  (`width: left_panel.width`). Shared evaluator in `mui-syntax::style`
  (`DimEnv` / `eval_dim` / `collect_id_dims`); the dev runtime fills the metrics
  from the live window, the codegen from the `app { }` block. `Stack` now
  honours an explicit `width`/`height` instead of a fixed default.
- **MUI: common `x:` / `y:` props on every widget** — each given axis overrides
  the auto-flow layout cursor while a missing axis keeps flowing (`x: 100` pins
  the column, the widget still stacks vertically). Wired through both the
  codegen (`position_args`) and the runtime (`place`). New example:
  `examples/mui/positioned/`.
- **MUI `Popup` component** (runtime + codegen + LSP) — a non-modal floating
  card: a positioned container (`x:` / `y:`) drawn above its siblings via a high
  z-index (default 1000, override with `zIndex:`), with `visible: false` to
  hide. Built from the `Rectangle` container path; `zIndex:` is now honoured on
  any widget. New example: `examples/mui/popup/`.
- **MUI `Dialog` and `Audio` components** (runtime + codegen + LSP):
  `Dialog(cardWidth:, cardHeight:, radius:, cardColor:/background:,
  backdropColor:, dismissOnBackdrop:, visible:) { … }` is a backdrop + centered
  card overlay (mocida's `UIDialog`); `Audio("clip.wav", volume:, autoplay:)` is
  a non-visual one-shot WAV (mocida's `UISound`), kept alive for the document's
  lifetime. New example: `examples/mui/dialog/`.

### Changed
- **Copper: a bare `self` receiver now borrows (`&self`)** on `impl` methods,
  matching the `class` path and the convention across real Copper code
  (read-only accessors). Methods no longer consume the receiver by default, so a
  value can be inspected by many methods without being moved.

### Fixed
- **Copper: `for x in <identifier> { … }` got `,` separators instead of `;`.**
  The struct-literal brace detector mistook the loop body `{` (which follows an
  identifier) for a struct literal, so multi-statement loop bodies transpiled to
  invalid Rust. `for` now arms `expect_block_brace` like the other control-flow
  keywords. (`for i in 0..5` was unaffected — that `{` follows a number.)
- **MUI host build bound against a stale staged mocida SDK.** `apply_mocida_env`
  preferred `mocida/release/stage` even when its headers were older than the C
  source headers, so bindgen generated an incomplete `sys` and the `mocida`
  wrapper failed with ~40 "cannot find function in sys" errors. It now uses the
  staged SDK only when it's at least as fresh as `mocida/src/headers`, otherwise
  binds against the source headers + the source `build/` lib.
- **Copper: generic return / parameter types on `impl` methods** —
  `func Option<T> find(self, xs: Vec<T>)` no longer silently drops the method.
  `process_impl_methods` now gobbles the `<…>` after the return-type base (so it
  finds the real method name) and on param types (so `Vec<T>` isn't split at the
  inner comma). New example: `examples/copper/methods.crs`.
- **Copper: `impl` method bodies now lower through the real statement machinery**
  (a sub-parser) instead of a naive token-join. Multi-statement bodies get `let`
  injection and `;` terminators, instead of collapsing onto one invalid line
  (`len = self.length() if len == 0.0 {…}`).
- **Copper: struct-literal fields are no longer mistaken for typed declarations**
  — `Vec2 { x: self.x }` stayed correct but `x: self.x` (identifier value) became
  `let x: self; .x`. `parse_type_declaration` now skips when the value continues
  as an expression or sits between struct fields.
- **Copper: multi-line struct literals and operator-continued lines** no longer
  get a spurious `;`. The tokenizer tracks struct-literal braces (separating
  fields with `,`) and suppresses the statement `;` after a line that ends in a
  binary/infix operator (`a &&` / `x +` / `obj.`). Regression tests:
  `crates/copper-parser/tests/impl_methods.rs`.

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
