# CLAUDE.md

Project context for AI assistants working on **copper-lang**. Read this
first — it captures the moving parts, conventions, and the known traps that
have already burned us.

## What this project is

Copper is a high-level language that **transpiles to Rust**. The compiler
binary is `cforge`, written in Rust, distributed by the bundled installer.

Pipeline:

```
.crs source ─► tokenizer ─► token rewrites ─► parser ─► Rust source
                                                          │
                                                          ▼
                                                       cargo build
```

`cforge run foo.crs` does the full pipeline plus a final `cargo run` inside
`dist/rust/`.

### MUI files (`.mui` / `.crm`)

`cforge run foo.mui` (or `.crm`) does **not** transpile — it renders the file
as a live mocida UI by shelling out to the `mui-dev` host (see
`src/cforge/mui.rs` and `mocida/mui/ARCHITECTURE.md`). cforge stays portable:
it never links the mocida C library or `mui-runtime` directly (that would break
the Linux/macOS CI), it just locates and launches the `mui-dev` binary the same
way `run()` launches `cargo`. Renderer resolution: `MUI_DEV_BIN` env →
pre-built `mui-dev[.exe]` under a discovered `mocida-rs/target/` (DLLs staged
beside it) → `cargo run -p mui-dev` in the workspace (found via `MOCIDA_RS_DIR`
or by looking for a sibling `mocida/mocida-rs`). This is the **dev** path (M1:
static render). The `mui-syntax` / `mui-runtime` / `mui-dev` crates live in
copper-lang and the mocida-rs workspace respectively.

`cforge -c -i foo.mui` (or `.crm`) is the **codegen path (M5)** — the same
`-c`/`--compile` flag that transpiles `.crs`, with the file extension picking
the backend. It lowers the component AST to readable **Rust source** (via the
`mui-codegen` crate, `crates/mui-codegen`), writes a self-contained cargo
project to `<output>/mui/` (a `main.rs` building the mocida tree + a
`Cargo.toml` with a path dep on `mocida`), and prints the generated code. Plain
`-c` stops there (like `-c foo.crs` writes the `.rs` without running it); add
`-r`/`--release` to also `cargo build --release` it into a native binary (DLLs
auto-staged by `mocida-sys`'s build script). A sibling `app.bundle` and the
assets it lists are staged into `<output>/mui/` so the build finds them; add
`-b`/`--bundle` to instead **embed** them into the executable (`include_bytes!`
in `main.rs` + self-extract to a temp dir at startup via `EmbedSpec` in
mui-codegen and `stage_bundle` in `src/cforge/mui.rs`) for a self-contained
binary. `mui-codegen` is pure AST→text (no mocida dependency), so cforge stays
portable; only the *generated* crate links mocida. Handlers + live reactivity
are emitted as comments/placeholders for now (the M3/M4 evaluator work, ported
into codegen later).

So the two MUI verbs mirror the `.crs` ones: `cforge run foo.mui` = dev render
(M1), `cforge -c [-r] foo.mui` = generate code [+ native build] (M5).

## Repository layout

```
copper-lang/
├── src/                    Rust source for the cforge compiler
│   ├── main.rs             CLI entrypoint (clap, --version, build date)
│   ├── tokenizer/
│   │   ├── kind.rs         TokenKind enum
│   │   ├── tokens.rs       Token + Data variants
│   │   ├── tokenizer.rs    Char-level scanner, brace tracking, ?. detection
│   │   └── interpolation.rs `$ident` / `${expr}` parser for strings
│   ├── parser/
│   │   ├── mod.rs          Big dispatch loop, parse_* methods, depth tracking
│   │   ├── result.rs       Output buffer (module-level + main_function_code)
│   │   ├── ternary.rs      Token rewrite for `cond ? a : b` → `if … { … } else { … }`
│   │   ├── utils.rs        Type aliases (`int` → `i64`, etc.)
│   │   ├── scope.rs        (mostly stub) variable scope tracking
│   │   └── scope_manager.rs (mostly stub)
│   ├── cforge/
│   │   ├── mod.rs          Compile pipeline, run command, single-file → main.rs
│   │   ├── kson.rs         properties.kson reader
│   │   ├── properties.rs   Cargo.toml generator
│   │   ├── fetch.rs, vprint.rs
│   └── utils/              Misc helpers (Consumed, ConsumedTrait, etc.)
├── examples/               Runnable samples — add new ones here
│   ├── copper/             Copper (.crs) demos
│   │   ├── loops.crs       loop / while / for / break / continue
│   │   ├── interpolation.crs  "$name" / "${expr}"
│   │   ├── collections.crs vec literals, closures, ?
│   │   ├── matching.crs    match arms, if let, while let, multi-line comments
│   │   ├── optional.crs    `?.` optional chaining
│   │   ├── ternary.crs     `cond ? a : b`
│   │   ├── cstd.crs        cstd stdlib usage
│   │   ├── unsafe.crs      unsafe func / unsafe blocks
│   │   └── rust-interop/   mixing .crs + .rs
│   └── mui/                MUI (.mui / .crm) demos — each in its own subfolder
│       ├── hello/          minimal view
│       ├── counter/        reactive state + if/else
│       ├── keyboard/       onKeyInput keyboard handling (event.key)
│       ├── styled/         widget styling + anchors
│       ├── app/            full App() block with bundle
│       ├── card/           reusable component (view with params)
│       ├── dashboard/      cross-file import (imports card/)
│       ├── bundle-demo/    app.bundle + asset loading
│       └── crm/            .crm (Copper + MUI in one file)
├── scripts/                Cross-platform Python tooling (needs Python 3.7+)
│   ├── _pretty.py          Shared copper-themed terminal styling (banner/ok/warn/…)
│   ├── install.py          Build + install cforge; scope-detecting (admin → global, else local)
│   ├── uninstall.py        Standalone (no _pretty import — gets copied to the install dir)
│   ├── build.py, cleanup.py, diagnose.py, hooks.py
│   └── install.bat/.sh, uninstall.bat   Thin shims that just forward to the .py
├── docs/
│   └── INSTALL.md          User-facing install guide
├── std/                    Copper standard library
│   ├── cstd.crs            Copper-written helpers (input, readln, exit, ...)
│   ├── cstd_native.rs      Rust-only helpers Copper can't transpile yet
│   └── import.crs          Old dynamic-import demo (unused)
├── installer-gui/          MUI GUI installer (Windows-first). The UI is the
│   ├── installer.mui      single source: a declarative MUI view (no web/Tauri).
│   ├── backend.rs         Install logic (cargo build + copy + PATH); std-only.
│   ├── copper-installer/  Native host crate (renders installer.mui + links backend.rs)
│   └── README.md          `cforge run installer-gui/installer.mui`
├── lson/                   Runtime assets that the installer copies
├── assets/                 Logos
├── main.crs                Default file used by `cforge run` with no arg
├── properties.kson         Project config used by `cforge generate_toml`
├── Cargo.toml, Cargo.lock  Rust crate metadata
├── build.rs                Stamps `COPPER_BUILD_DATE` env into the binary
├── README.md               Top-level overview
└── CLAUDE.md               This file
```

## How a `.crs` file becomes a running program

1. **`tokenizer::Tokenizer::new(source)`** strips block comments
   (`strip_block_comments`) and runs `clean_source`. The latter normalises
   line endings and tracks compensation offsets for accurate location data.

2. **`tokenize()`** walks line by line. Per char it tries
   `identifier_token`, `number_token`, `string_token`, `comment_token`,
   `regex_token`, `operator_token`, `symbol_token`, `whitespace_token`,
   `line_break_token` — first one that consumes wins. The order matters:
   `comment_token` runs before `regex_token` so `//` doesn't look like a
   regex.

3. **Token classification quirks worth remembering:**
   - `loop` / `while` / `break` / `continue` / `in` get specialised
     `TokenKind` values (Loop, While, Break, Continue, In) — the parser
     uses these to route through `parse_any` for keyword spacing.
   - `func` sets `seen_func = true`. The next identifier becomes
     `TokenKind::ReturnType` **before** the RUST_KEYWORDS check, so
     `func Result<T, E>` doesn't lose `Result` to the keyword table.
   - `match` sets `expect_match_brace = true` and uses
     `match_paren_depth` to find the **outer** `{` that opens the body
     (skipping `{` inside `match foo({}) { … }`).
   - `?` followed immediately by `.` is fused into a single
     `OptionalChain` token. A bare `?` stays the Rust try operator.
   - The newline emission (`line_break_token`) appends `;\n` for
     statement terminators **except** after `,`, `{`, `}`, `(`, `[`, or
     when `brace_is_match.last() == Some(true)` — match arms get `,\n`.

4. **`Parser::new`**:
   1. Filters whitespace + comment tokens.
   2. **`ternary::rewrite()`** lowers `cond ? a : b` into `if cond { a } else { b }`
      at the token-stream level, before the main parser sees them.

5. **`Parser::parse()`** is one big dispatch loop. Each iteration:
   1. Captures `(kind, value)` of the current token.
   2. **`maybe_close_optional_chains_for(kind, value)`** — if any open
      `?.` chain should end here (`is_chain_breaker`), emits `)` and pops.
   3. `match kind` runs the right `parse_*` method or falls to the catch-all.
   4. Updates `chain_delim_depth` based on the kind we just processed
      (open delim → +1, close → −1).
   5. `consume_var(&mut self.current)` advances by the returned consumed
      count.

6. **`result::Result`** is the output buffer. Two streams:
   - `value` — module-level items (struct/impl/fn).
   - `main_function_code` — top-level statements wrapped into `fn main()`
     by `write_main_function()` at the end.
   - `is_inside_function`, `is_function`, `is_inside_impl` decide which
     stream `append` writes to. `force_append` always writes to `value`.

7. **`cforge::compile`** writes the parser output to `dist/rust/src/<file>.rs`,
   with one quirk: a single `.crs` input (no `--input` directory) is always
   written to `dist/rust/src/main.rs`, regardless of source name. That
   matches Cargo's default binary so `cargo run` always executes the file
   you just compiled.

8. **`cforge::generate_toml`** materialises `dist/rust/Cargo.toml` from
   `properties.kson` plus any deps the parser detected (`uses_json`,
   `uses_xml`, `uses_toml`, plus regex / crate detections in
   `parser::result`).

9. **`cforge::run`** runs `cargo build && cargo run` in `dist/rust/`.

## Key invariants & gotchas

### Tokenizer

- **`symbol_token` MUST stay an `if` / `else if` chain.** The original code
  used consecutive `if`s, which let `),` and `).` collapse into single
  multi-char tokens (kind `Symbol` or `Dot`, value `),` / `).`). This broke
  paren-depth tracking and corrupted parser output. Don't switch back.
- **`?.` is a fused token (`OptionalChain`).** Don't try to handle it as
  separate `?` + `.` in the parser; the tokenizer already merged them.
- **`<` and `>` are tokenized as `Operator` (not `AngleStart`/`AngleEnd`)**
  most of the time, because comparison signs are matched before the
  angle-bracket rule. The generic-return-type capture in the parser
  accepts both kinds for that reason.
- **Block comments (`/* … */`) are stripped before tokenization** via
  `strip_block_comments` so the line-by-line scanner doesn't have to span
  newlines. Newlines inside block comments are preserved to keep line
  numbers honest.

### Parser

- **Function brace depth lives in `Parser::function_brace_depth`.** Without
  it, `parse_function_body` exits the function on the *first* inner `}` —
  match blocks, if/else, nested scopes all break that. Don't remove the
  counter.
- **Optional chaining uses two pieces of state:**
  `chain_delim_depth` (current paren/bracket/brace depth) and
  `optional_chain_depths` (depth of each open chain). The post-dispatch
  depth update needs to fire for *every* token kind, including the catch-
  all `_` arm; the dispatch loop captures `dispatched_kind` precisely so
  this works.
- **`is_chain_breaker` decides when to close a `?.` chain.** Add new
  TokenKinds to its match arms; default is "not a breaker", which is
  usually wrong for new value-side kinds.
- **`parse_var` is the source of subtle bugs.** It treats `Identifier =`
  as a top-level assignment that needs `let`. Two existing guards: skip
  when the var is `_` (match wildcard) or when the token after `=` is `>`
  (i.e. `=>` in match). If you see "let _ = ..." or "let X = >..." in
  generated output, this is the culprit.
- **Control-flow keywords get *both* leading and trailing space** in
  `parse_any` (`if`, `else`, `loop`, `while`, `for`, `in`, `match`,
  `return`, `break`, `continue`, `as`, `let`, `mut`, `pub`, `ref`,
  `move`, `yield`). `n if x > 0 =>` would collapse to `nif x > 0 =>`
  without the leading space.
- **`In` has a special dispatch arm** that emits `" in "` (literal spaces)
  rather than going through `parse_any`. It's the only keyword positioned
  *between* an identifier and an expression, where leading space is
  critical.
- **`match` arms are separated by `,` not `;`.** This is enforced in the
  tokenizer's `line_break_token` via `brace_is_match.last()`. If you add
  new statement-like contexts that use `{ ... }`, decide whether they
  want `,` or `;` separators.

### Versioning & build date

- **`build.rs`** computes today's UTC date with stdlib-only `civil_from_days`
  (Howard Hinnant's algorithm) and exposes it as `COPPER_BUILD_DATE`.
- It uses `cargo:rerun-if-changed=.cforge-build-date-trigger` (a path
  that doesn't exist) to force `build.rs` to rerun on every cargo
  invocation. Without this, the date freezes the next time only `.rs`
  files change. Don't "fix" it back to `rerun-if-changed=build.rs`.
- `with_build_date(version)` in `main.rs` only stamps the date when the
  version string contains `alpha`, `beta`, or `rc`. Stable releases stay
  clean.
- **`scripts/install.py`** deletes `target/release/cforge(.exe)` before
  `cargo build --release` to force a fresh link, keeping the installed
  binary in sync with the latest build.

### Scripts

- **All tooling in `scripts/` is cross-platform Python** (one `.py` per
  task: `install`, `uninstall`, `build`, `cleanup`, `diagnose`, `hooks`).
  The old per-OS `.bat`/`.sh` were collapsed into these. Each resolves the
  project root via `Path(__file__).resolve().parent.parent` — no `cd`.
  Shared output styling lives in `scripts/_pretty.py`; import from it for
  any new script (`from _pretty import banner, head, ok, warn, fail, …`).
- **`scripts/uninstall.py` must stay self-contained** — it is copied into
  the install dir (away from `_pretty.py`), so it inlines its own tiny
  styling helpers and must NOT `import _pretty`. The other scripts run
  from `scripts/` and import it freely.
- Thin shims (`install.bat`, `install.sh`, `uninstall.bat`) only locate a
  Python 3 interpreter and forward args to the matching `.py`. Keep them
  trivial — all real logic stays in Python. `install.py` ships the
  uninstaller by copying `uninstall.py` (+ `uninstall.bat` on Windows).
- `install.py` / `uninstall.py` auto-detect scope from privilege level
  (Windows: `ctypes…IsUserAnAdmin`; Unix: `geteuid()==0`) → global
  (`HKLM` / `Program Files` / `/usr/local/lib/copper` / `/etc/profile.d`)
  or local (`HKCU` / `~/.copper` / shell rc block). On Windows the PATH
  edit uses `winreg` (REG_EXPAND_SZ, filtering the literal
  `%COPPER_PATH%\bin` marker) plus a `WM_SETTINGCHANGE` broadcast; on Unix
  it manages a `>>> COPPER PATH >>>` block in `/etc/profile.d/copper.sh`
  or the user's rc files. Override with `--local` / `--global`.

## How to add a new feature

The pattern that's worked well in this codebase:

1. **Decide where the transformation lives.** Tokenizer is best for
   atomic syntactic forms (e.g. `?.`, `$name` interpolation). Parser
   token rewrite (like `ternary.rs`) is best for "macro-like" rewrites
   that depend on local context. Parser dispatch is best when emission
   needs awareness of result state (function context, brace depth).

2. **For new TokenKinds:**
   - Add to `kind.rs` enum + `to_string`.
   - Emit it in the tokenizer.
   - Add a dispatch arm in `parser/mod.rs` near the related arms.
   - Add it to `is_chain_breaker` if it should affect `?.` chains.
   - Add an `examples/copper/<feature>.crs` and confirm it compiles end-to-end.

3. **For token rewrites (like `ternary.rs`):**
   - Operate on `Vec<Token>` → `Vec<Token>`.
   - Iterate-until-fixed-point if multiple matches in one stream.
   - Use `Token::new(kind, value, len, Data::None, /*generated:*/ true)`
     for synthetic tokens.
   - Wire it into `Parser::new` after the whitespace filter and before
     the main loop.

4. **Always confirm with the regression suite.** Run all `examples/copper/*.crs`
   after non-trivial changes:

   ```sh
   for f in examples/copper/*.crs; do ./target/debug/cforge.exe run "$f"; done
   ```

5. **Preserve `cforge --version` output.** It should print `CForge v…`
   and `Copper v…-alpha.N (build YYYY-MM-DD)` after every build —
   double-check after touching `build.rs` or `main.rs`.

## What you can change freely

- `src/parser/`, `src/tokenizer/`, `src/cforge/` — main implementation.
  Make focused changes; document non-obvious invariants in comments.
- `examples/copper/` — add new `.crs` programs to demo features.
- `examples/mui/` — add new `.mui`/`.crm` programs (each in its own subfolder).
- `docs/INSTALL.md` and `README.md` — keep them in sync with reality.
- `scripts/` — update install / build / diagnose tooling.
- `properties.kson` — project metadata for the bundled demo.

## What you should NOT change without asking

- **Don't stub the parser back to a "minimal echo" form.** It happened
  before (commit `cd22d82` left a 45-line stub that was being mistaken
  for the real parser); restoring the full version was non-trivial.
- **Don't move `main.crs` out of the project root.** `cforge run` with no
  argument defaults to `./main.crs`; that's a documented affordance.
- **Don't disable the `match`-block separator switch in `line_break_token`.**
  It is what makes match arms emit `,` instead of `;`. Without it,
  generated code fails compilation with `expected ;, found ,`.
- **Don't reintroduce stacked `if`s in `symbol_token`.** See "Tokenizer"
  invariants above.
- **Don't change `cforge_tokenizer_debug.log` into a feature.** It was a
  one-off debug aid — keep it gitignored and out of the source tree.
- **Don't push to remote, force-push, or open PRs without explicit user
  ask.** This is a personal project and the user runs `install.py` /
  `cforge run` interactively to verify changes.

## Testing & verification

```sh
# Build the compiler
cargo build              # debug
cargo build --release    # release (also what install.py runs)

# Lint / format checks (CI runs these)
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check
cargo test               # 6 tests in src/tokenizer/interpolation.rs

# End-to-end: compile and run a Copper sample
./target/debug/cforge.exe run examples/loops.crs

# Re-install after release-mode changes (any OS)
python scripts/install.py   # then open a NEW terminal
```

CI matrix (`.github/workflows/ci.yml`): fmt / clippy (`-D warnings`) /
check / test / build on Linux, Windows, macOS.

`src/main.rs` carries a crate-level `#![allow(...)]` for ~11 stylistic
clippy categories that come from untouched legacy code (interior-mutable
`Lazy<T>` consts, `module_inception` for `tokenizer/tokenizer.rs`,
`to_string_trait_impl`, etc.). When you rewrite a module covered by
those allows, drop the relevant entry — anything outside the allow-list
fails the build.

### Toolchain pin

`rust-toolchain.toml` pins the channel to `stable` and auto-installs
`rustfmt` + `clippy`. Cargo respects it the moment anyone enters the
directory, so the rustc and clippy used locally always match what
`dtolnay/rust-toolchain@stable` installs in CI.

If a fresh clippy lint lands in a newer `stable` and starts breaking
CI, two options:

1. Fix the lint (preferred — the CI is honest about what's flagged).
2. Freeze: change `channel = "stable"` to `channel = "1.95.0"` (or
   whatever is current) so updates become deliberate. Don't do this
   without a comment explaining what regression triggered the freeze.

After a `rustup update` that bumps stable, run `cargo clean` once —
proc-macro DLLs (tokio-macros, serde_derive, clap_derive, ...) are
ABI-tied to the rustc that built them and refuse to load with a new
compiler. The pin doesn't help here because the pin floats with stable;
freezing to a specific version is the only way to avoid this entirely.

### Pre-commit / pre-push hooks

`.githooks/pre-commit` runs `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`.
`.githooks/pre-push` runs `cargo test`.

Activate once per clone with `python scripts/hooks.py` (any OS). The setup
script just runs `git config core.hooksPath .githooks`.

When working on changes:

- Don't disable the hooks — they catch the lint surface that has bitten
  the repo (e.g. clippy 1.95 introducing `collapsible_match` /
  `filter_next` errors that the older local clippy missed).
- If a hook reports a real bug in your change, fix it. If the lint is a
  legitimate false positive in untouched legacy code, add it to the
  crate-level allow-list with a comment explaining why.

## Language features currently supported

| Feature | Source form | Lowering |
| --- | --- | --- |
| Variables | `mut x = 1`, `x = 1` | `let mut x = 1;`, `let x = 1;` |
| Increment / decrement | `x++`, `x--` | `x += 1;`, `x -= 1;` |
| Functions | `func ReturnType name(args)` | `fn name(args) -> ReturnType` |
| Classes / structs / impl | similar to Rust | passes through, with method-receiver fixups |
| Imports | `import x from std`, `import { io } from std` | `use std as x;`, `use std::{io};` |
| Loops | `loop`, `while`, `for x in iter` | identical to Rust |
| Match (with guards, `_`, `\|`) | `match n { 0 => 0, n if n < 0 => -1, _ => 2 }` | identical |
| `if let` / `while let` | identical to Rust | identical |
| Vec literals | `[1, 2, 3]` | `vec![1, 2, 3]` (kept as `[…]` only when indexing) |
| Closures | `\|x\| x * 2` | identical |
| Try operator | `expr?` | identical |
| Optional chaining | `obj?.field` | `obj.as_ref().map(\|c\| c.field)` |
| String interpolation | `"$name"`, `"${expr}"` | `format!("…", args)` or raw macro args |
| Ternary | `cond ? a : b` | `if cond { a } else { b }` |
| Multi-line comments | `/* … */` | stripped before tokenization |
| Generic return types | `func Result<T, E> name()` | `fn name() -> Result<T, E>` |
| Unsafe blocks | `unsafe { ... }` | identical to Rust |
| Unsafe functions | `unsafe func i32 deref(p: *const i32)` | `unsafe fn deref(p: *const i32) -> i32` |
| Rust files alongside Copper | a `.rs` file in the input | copied verbatim into the crate (see below) |

## Mixing Rust files with Copper (`.rs` input)

`cforge::compile` (`src/cforge/mod.rs`) classifies each input file by extension:
`.crs` is transpiled as before; **`.rs` is linked in verbatim** (never tokenized
as Copper); anything else is skipped (so a project dir may hold assets/kson/md
without breaking the build).

- **A lone `.rs`** (`cforge run foo.rs` / `cforge -c -i foo.rs`) is emitted as the
  crate's `main.rs` — cforge builds/runs a plain Rust program.
- **A `.rs` next to `.crs` files** (directory input) is copied to
  `dist/rust/src/<stem>.rs` and a `pub mod <stem>;` line is prepended to the
  generated `main.rs`. So a Copper file can `import { fib } from math` (→
  `use math::{fib};`) to call into a sibling `math.rs`. Module name = sanitized
  file stem (`rust_module_name`). See `examples/copper/rust-interop/`.

This mirrors MUI's foreign imports (`import { x } from "./native.rs"` in a
`.mui`), which materialize the same way via `src/cforge/mui.rs`.

## The `cstd` standard library

`import { input, exit, sleep_ms, ... } from cstd` triggers a special path:

1. The parser sees `from cstd` and sets `result.cstd_used = true`.
2. At end of `Parser::parse()`, if `cstd_used`, it calls
   `Parser::transpile_cstd_module()` which:
   - `include_str!`s `std/cstd.crs` (Copper source)
   - Tokenizes + sub-parses it
   - Strips the trailing `fn main() {}` the sub-parser emits unconditionally
   - Promotes every line starting with `fn ` to `pub fn ` so `use cstd::{X}`
     resolves
   - Concatenates `std/cstd_native.rs` (raw Rust) for helpers Copper can't
     express yet
   - Wraps the lot in `#[allow(dead_code)] pub mod cstd { ... }`
3. Prepends to `result.value`.

### Adding a new cstd helper

Prefer `std/cstd.crs` (Copper). Constraints to keep in mind:

- **No multi-line method chains.** Copper's tokenizer ends statements at
  newlines; chaining `.foo()` on the next line emits `;\n.foo()` and
  breaks. Keep chains on one line, or use a temp var.
- **No `&[T]` slice types in params.** `&[String]` is tokenized as a vec
  literal and emits `&vec![String]`. Use `Vec<String>` or move the helper
  to `std/cstd_native.rs`.
- **No `cfg!(target_os = "windows")`.** `parse_var` injects `let` before
  the `=`, breaking the macro. Move OS-conditional code to native.
- **`pub func` doesn't work.** The `pub` leaks to the next statement. The
  promotion to `pub fn` happens automatically on injection.

If a helper hits these limitations, drop it into `std/cstd_native.rs` as
plain Rust. Both files are bundled via `include_str!` in
`src/parser/mod.rs`, so just rebuild cforge.

## `unsafe` support

- `unsafe { ... }` blocks transpile straight through (the keyword is in
  the spaced-keyword list in `parse_any` so it doesn't fuse with `{`).
- `unsafe func ...` is handled by a one-token lookahead in `parse_any`:
  when `unsafe` is followed by `func`, the `unsafe` is swallowed and a
  flag (`pending_unsafe_fn`) tells `parse_function` to call
  `result.enter_unsafe_function()` (emits `unsafe fn `) instead of
  `enter_function()` (emits `fn `). Without this swallow, the `unsafe`
  would land in `main_function_code` and fuse with the following
  statement (`unsafelet x = ...`).
- Raw pointer types (`*const T`, `*mut T`) work because `const`, `static`,
  `dyn`, `async`, `await`, `extern`, `where` are now in the spaced-
  keyword list, so they don't fuse with neighbouring identifiers.

## Known limitations to be aware of

- **Generics in function or struct parameters** (`func<T> name(arg: T)`)
  are **not** supported yet. The mirror of return-type capture would be
  the implementation path, but it hasn't been written.
- **`?:` ternary inside `${expr}`** in interpolated strings doesn't get
  rewritten — interpolation captures the expression as an opaque string at
  tokenizer time, before `ternary::rewrite` runs. Pull the ternary out of
  the interpolation as a workaround.
- **`obj?.field`** translates to `.as_ref().map(...)`, which means the
  result is `Option<&T>` (a reference). For non-`Copy` payloads users may
  need explicit `.clone()` or `.cloned()`.
- **`parse_var`'s JSON-detection branch** is large (~150 lines). Refactors
  here historically introduce subtle regressions — touch it only with a
  good reason and a covering example.
- **`scope.rs` and `scope_manager.rs`** are present but mostly unused.
  Don't over-invest in them until a feature actually needs them.
- **`parse_var` / `parse_mut` skip `let` injection** when the preceding
  token is `&`, `&&`, or `*`. That's what makes `&mut y` (borrow) and
  `*mptr = expr` (deref-assign) survive transpilation. New prefix
  contexts (e.g. a future `move` capture in expression position) need
  explicit handling there.

## Communication conventions

- The user prefers Portuguese-Brazilian (PT-BR) chat replies, terse,
  occasional code blocks. Reports of regressions should include the exact
  output and what was expected.
- For non-trivial design decisions, brainstorm and present 2-3 options
  with tradeoffs before implementing.
- After completing a task, summarize files touched + behaviour change.
  Avoid noisy step-by-step recaps of every tool call.
