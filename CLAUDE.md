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
├── examples/               Runnable .crs samples — DO add new ones here
│   ├── loops.crs           loop / while / for / break / continue
│   ├── interpolation.crs   "$name" / "${expr}"
│   ├── collections.crs     vec literals, closures, ?
│   ├── matching.crs        match arms, if let, while let, multi-line comments
│   ├── optional.crs        `?.` optional chaining
│   └── ternary.crs         `cond ? a : b`
├── scripts/                All install / build / cleanup tooling
│   ├── install.bat         Windows; cd's to project root via "%~dp0\.."
│   ├── install.sh          Linux
│   ├── install-mac.sh      macOS
│   ├── uninstall.bat       Static, scope-detecting (admin → global, else local)
│   ├── build.bat, cleanup.bat, diagnose.bat
├── docs/
│   └── INSTALL.md          User-facing install guide
├── std/, lson/             Runtime assets that the installer copies
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
- **`scripts/install.bat`** deletes `target/release/cforge.exe` before
  `cargo build --release` to force a fresh link, keeping the installed
  binary in sync with the latest build.

### Scripts

- All scripts in `scripts/` operate from the project root. They start
  with `cd /d "%~dp0\.."` (Windows) or `cd "$SCRIPT_DIR/.."` (Unix). If
  you add a new script there, follow that pattern.
- **`scripts/uninstall.bat` is the canonical, hand-written uninstaller.**
  An earlier version was generated inline by `install.bat` via stacked
  `echo` lines; the escape soup produced a broken `if ^ neq 0` and
  variables like `^^^^!`. The current `install.bat` simply `copy`s the
  static file. Don't reintroduce the inline generator.
- `uninstall.bat` auto-detects scope by checking `net session` (admin →
  global / `HKLM` / `Program Files`; else local / `HKCU` / `~/.copper`).
  It uses a PowerShell one-liner to surgically remove `%COPPER_PATH%\bin`
  from `PATH` because cmd's substring substitution mangles long PATHs
  with parens / semicolons.

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
   - Add an `examples/<feature>.crs` and confirm it compiles end-to-end.

3. **For token rewrites (like `ternary.rs`):**
   - Operate on `Vec<Token>` → `Vec<Token>`.
   - Iterate-until-fixed-point if multiple matches in one stream.
   - Use `Token::new(kind, value, len, Data::None, /*generated:*/ true)`
     for synthetic tokens.
   - Wire it into `Parser::new` after the whitespace filter and before
     the main loop.

4. **Always confirm with the regression suite.** Run all `examples/*.crs`
   after non-trivial changes:

   ```sh
   for f in examples/*.crs; do ./target/debug/cforge.exe run "$f"; done
   ```

5. **Preserve `cforge --version` output.** It should print `CForge v…`
   and `Copper v…-alpha.N (build YYYY-MM-DD)` after every build —
   double-check after touching `build.rs` or `main.rs`.

## What you can change freely

- `src/parser/`, `src/tokenizer/`, `src/cforge/` — main implementation.
  Make focused changes; document non-obvious invariants in comments.
- `examples/` — add new `.crs` programs to demo features.
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
  ask.** This is a personal project and the user runs `install.bat` /
  `cforge run` interactively to verify changes.

## Testing & verification

```sh
# Build the compiler
cargo build              # debug
cargo build --release    # release (also what install.bat runs)

# Lint / format checks (CI runs these)
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check
cargo test               # 6 tests in src/tokenizer/interpolation.rs

# End-to-end: compile and run a Copper sample
./target/debug/cforge.exe run examples/loops.crs

# Re-install after release-mode changes (Windows)
scripts\install.bat      # then open a NEW terminal
```

CI matrix (`.github/workflows/ci.yml`): fmt / clippy (`-D warnings`) /
check / test / build on Linux, Windows, macOS.

`src/main.rs` carries a crate-level `#![allow(...)]` for ~11 stylistic
clippy categories that come from untouched legacy code (interior-mutable
`Lazy<T>` consts, `module_inception` for `tokenizer/tokenizer.rs`,
`to_string_trait_impl`, etc.). When you rewrite a module covered by
those allows, drop the relevant entry — anything outside the allow-list
fails the build.

### Pre-commit / pre-push hooks

`.githooks/pre-commit` runs `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`.
`.githooks/pre-push` runs `cargo test`.

Activate once per clone with `scripts\install-hooks.bat` (Windows) or
`bash scripts/install-hooks.sh` (Unix). The setup script just runs
`git config core.hooksPath .githooks`.

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

## Communication conventions

- The user prefers Portuguese-Brazilian (PT-BR) chat replies, terse,
  occasional code blocks. Reports of regressions should include the exact
  output and what was expected.
- For non-trivial design decisions, brainstorm and present 2-3 options
  with tradeoffs before implementing.
- After completing a task, summarize files touched + behaviour change.
  Avoid noisy step-by-step recaps of every tool call.
