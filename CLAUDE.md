# CLAUDE.md

Context for AI assistants on **copper-lang** — traps, invariants, conventions.

## What this project is

Copper transpiles to Rust. Binary: `cforge`. Pipeline:
`.crs` → tokenizer → token rewrites → parser → Rust source → `cargo build`.

**Second engine: Alloy** (`crates/alloy-vm`, binary `alloy`) — tree-walking interpreter
over the shared `copper-syntax` AST. Instant like `python`; no transpile, no cargo.
Design specs: `docs/superpowers/specs/2026-06-22-alloy-vm-design.md`.

## Repository layout (canonical)

```
crates/
  copper-syntax/    tokenizer + AST (shared by cforge, alloy, LSPs)
  copper-parser/    streaming transpiler parser
  alloy-vm/         Alloy interpreter lib + `alloy` CLI
                    src/{value,env,error,interp,bytecode,loader,stdlib,stdlib_ext,wasm,intrinsics}.rs
  copper-lsp/       Copper language server
  mui-syntax/       MUI (.mui/.crm) parser
  mui-codegen/      MUI → Rust codegen (M5)
  mui-lsp/          MUI language server
src/                cforge entrypoint + re-exports from crates (historical; real code is in crates/)
alloy-gui/          Dual-mode alloy GUI host (MUI; own workspace; links mocida)
std/cstd.crs        Copper stdlib — single source for both cforge and alloy
docs/superpowers/specs/   Design specs (alloy-vm, bytecode, gui, check-miri, wasm-interop, mui-codegen)
docs/TODO.md        Backlog with priorities
```

## MUI files (`.mui` / `.crm`)

Two verbs:
- **`cforge run foo.mui`** — dev render (M1): shells out to `mui-dev` (never links mocida directly). Resolution: `MUI_DEV_BIN` env → `mocida-rs/target/` → `cargo run -p mui-dev` (found via `MOCIDA_RS_DIR` or sibling `mocida/mocida-rs`).
- **`cforge -c [-r] [-b] foo.mui`** — codegen (M5): lowers AST via `mui-codegen` to a self-contained cargo project in `<output>/mui/`. `-r` builds native binary; `-b` embeds `app.bundle` assets via `include_bytes!`. `mui-codegen` has no mocida dependency (cforge stays portable).

Handlers + live reactivity are emitted as comments/placeholders — M3/M4 evaluator work pending.

## Alloy — commands, .loy, imports

**Commands:**
- `alloy run file.crs` — instant interpret.
- `alloy build file.crs [-o out.loy]` — portable artifact.
- `alloy run app.loy` — run artifact (auto-detected by magic `ALLOYBC\0`).
- `alloy check file.crs [--no-miri]` — verify via real rustc + Miri.
- `cforge vm run/build` / `cforge check` — same VM via cforge.

**`.loy` format** (`crates/alloy-vm/src/bytecode.rs`):
`magic b"ALLOYBC\0"` + `fmt_ver: u16 LE` + bincode(alloy_ver) + bincode(payload).
v1 = `Vec<Item>` (AST). v2 = `Vec<Item>` + `Vec<WasmModule { names, bytes }>` (wasm embedded). v1 still loadable.

**Imports** (`loader.rs`):

| `import { x } from …` | Resolution |
| --- | --- |
| `fs`, `time`, `url`, `net`, `ws`, `json`, `crypto`, `http` | Native Rust in `stdlib.rs` / `stdlib_ext.rs` |
| `cstd` | Interpreted from `std/cstd.crs` (merged); 4 native exceptions: `run`, `list_dir`, `append_file`, `rand_int` (`CSTD_NATIVE`) |
| `math` (sibling `.crs`) | Parsed + merged recursively; bundled into `.loy` |
| `foo` (sibling `.rs`) | Compiled to wasm32 once (cached in `~/.alloy/cache` by hash), run via `wasmi`. Prototype ABI: `i64`/`bool`, requires `#[no_mangle] pub extern "C"`. Plain `pub fn` → `LoadOutcome::NeedsCforge`. |

**Key decision — don't fork rustc:** embedding Miri = shipping a nightly compiler (100s MB). Rejected. See `2026-06-22-alloy-check-miri-design.md`.

**Values** (`value.rs`): `Int/Float/Bool/Str/Unit/Vec/Tuple/Struct/Enum/Closure`.
`Option`/`Result` = `Enum{ty:"Option"|"Result", variant, payload}`. Manual `PartialEq` (closures never equal). Arithmetic is **checked** — must never panic; return `RuntimeError` with `Span`.

**Return-type checking:** interpreter verifies returned value matches declared type. Lenient for user/unknown types (base name only); strict for scalars/Option/Result/Vec. No return type = void.

## Key invariants & gotchas

### Tokenizer

- **`symbol_token` MUST stay an `if`/`else if` chain.** Consecutive `if`s collapse `),` and `).` into single multi-char tokens, breaking paren-depth tracking. Don't switch back.
- **`?.` is a fused `OptionalChain` token.** Don't handle as separate `?` + `.` in parser.
- **`<` and `>` are `Operator`, not `AngleStart/End`** most of the time. Generic capture accepts both.
- **Block comments stripped before tokenization** (`strip_block_comments`). Newlines preserved for line numbers.
- **`func` sets `seen_func = true`** — next identifier becomes `ReturnType` BEFORE `RUST_KEYWORDS` check, so `func Result<T,E>` survives.
- **`match` sets `expect_match_brace = true`** and uses `match_paren_depth` to find the outer `{`.
- **Newlines emit `;\n` except after `,`, `{`, `}`, `(`, `[`** or when `brace_is_match.last() == Some(true)` (match arms get `,\n` instead).

### Parser

- **`function_brace_depth` is required** in `parse_function_body`. Without it, the first `}` exits the function. Don't remove.
- **Optional chaining: two state pieces** — `chain_delim_depth` (current depth) + `optional_chain_depths` (depth of each open chain). Post-dispatch depth update fires for *every* token kind including `_`; `dispatched_kind` captures this.
- **`is_chain_breaker`** decides when to close a `?.` chain. Default "not a breaker" is usually wrong for new value-side TokenKinds.
- **`parse_var` treats `Identifier =` as top-level assignment needing `let`.** Existing guards: skip if var is `_` (match wildcard) or next token is `>` (`=>` in match). Regressions here produce `let _ = ...` or `let X = >...`.
- **`parse_var`/`parse_mut` skip `let` when preceded by `&`, `&&`, `*`** — enables `&mut y` borrows and `*mptr = expr` deref-assigns.
- **`parse_var`'s JSON-detection branch is ~150 lines.** Historically regression-prone. Touch with care + a covering example.
- **Control-flow keywords get both leading AND trailing space** in `parse_any` (`if`, `else`, `loop`, `while`, `for`, `in`, `match`, `return`, `break`, `continue`, `as`, `let`, `mut`, `pub`, `ref`, `move`, `yield`).
- **`In` has a special arm** that emits `" in "` literally — the only keyword between an identifier and expression.
- **`match` arms use `,` not `;`** — enforced via `brace_is_match.last()` in `line_break_token`. Don't disable.
- **Don't stub the parser back to a minimal echo.** Happened once (cd22d82); restoration was non-trivial.

### copper-syntax AST parser (Alloy)

- **Recoverable parser** — partial tree + `ParseError`s on bad input. `alloy run` rejects non-empty `prog.errors`. Earlier these were silently discarded (`_errs`).
- **Inside `(...)` identifiers are `Param`/`ParamType`, not `Identifier`.** `expect_ident` and closure-param parsing must accept both.
- **`?` vs ternary** disambiguated by `ternary_colon_ahead`: ternary iff `:` follows at the same paren depth before statement end.
- **`mut (a, b) = …`** = mutable tuple destructuring, not a `mut name =` binding.

### Versioning & build

- **`build.rs` uses `rerun-if-changed=.cforge-build-date-trigger`** (a non-existent path) to force rebuild every cargo invocation. Don't "fix" it to `rerun-if-changed=build.rs` — the date would freeze.
- `with_build_date` only stamps the date for versions containing `alpha`/`beta`/`rc`.
- `scripts/install.py` deletes `target/release/cforge(.exe)` before `cargo build --release` to force a fresh link.

### Scripts

- **`scripts/uninstall.py` must be self-contained** — it's copied to the install dir (away from `_pretty.py`), so it MUST NOT `import _pretty`. All other scripts import it freely.
- New scripts: inherit `_pretty.py` styling; resolve root via `Path(__file__).resolve().parent.parent` — no `cd`.

### Alloy GUI host (mocida)

Build `alloy-gui/` against sibling `mocida-rs` at `/Volumes/MCUDevDisk/mocida/mocida-rs`.
Required env vars: `MOCIDA_INCLUDE_DIR`, `MOCIDA_LIB_DIR`, `MOCIDA_LIB_NAME=mocida`, `MOCIDA_STATIC=0`, `SDL3_INCLUDE_DIR`. Runtime: `DYLD_FALLBACK_LIBRARY_PATH=<mocida>/mocida/build`.

**mui-runtime quirks (any `.mui` host must handle these):**
- **`renderer: vulkan` fails on macOS** (returns NULL). Omit → Metal auto-picked.
- **Button color is literal-only** — conditional `if/else` on `background:`/`textColor:` falls back to default blue. Use two Buttons via `if/else` with literal colors instead. Stack/Text/Rectangle DO evaluate conditional colors.
- **`Input`/`TextField` is single-line.** Use `TextArea` for multi-line (renders real `\n`).
- **`Image(source:"mocida://name")`** needs an absolute path registered via `mocida::bundle::set(...)` — relative resolution depends on CWD.
- **Stack `align:` = cross-axis; `justify:` = main-axis.** `align: stretch` is NOT valid. To left-pin in a stretched column: explicit `width:` + `justify: start`.
- **Resize is host-driven** — poll `UIApp_GetWidthG/HeightG` each tick; rebuild when size changes. Same for `take_dirty()` → `UIApp_SetChildren`.
- **Hot-reload** reads `.mui` from disk each tick (mtime check). `backend.rs` is compiled in and cannot hot-reload.
- **No native file picker** — use `rfd` crate for OS file dialog.

## Adding a new feature

1. **Where does it live?** Tokenizer = atomic syntactic forms (`?.`, `$ident`). Token rewrite = macro-like context-local rewrites (`ternary.rs`). Parser dispatch = emission that needs result state.
2. **New TokenKind:** add to `kind.rs`, emit in tokenizer, add dispatch arm in `parser/mod.rs`, add to `is_chain_breaker`, add `examples/copper/<feature>.crs`.
3. **Token rewrite:** `Vec<Token>` → `Vec<Token>`, iterate-until-fixed-point, synthetic tokens via `Token::new(..., generated: true)`, wire into `Parser::new` after whitespace filter.
4. **Regression suite:** `for f in examples/copper/*.crs; do ./target/debug/cforge run "$f"; done`
5. **Preserve `cforge --version`** output after touching `build.rs` or `main.rs`.

## What NOT to change without asking

- Parser stub → don't echo-stub it (see cd22d82).
- `main.crs` location → `cforge run` defaults to `./main.crs`.
- `match`-block separator switch in `line_break_token` → makes arms use `,` not `;`.
- Stacked `if`s in `symbol_token` → see tokenizer invariants.
- `cforge_tokenizer_debug.log` → keep gitignored, not a feature.
- Don't push/force-push/open PRs without explicit ask.

## cstd standard library

`import { ... } from cstd` → sets `result.cstd_used = true` → `Parser::transpile_cstd_module()`:
1. `include_str!` of `std/cstd.crs` → tokenize + sub-parse.
2. Strip trailing `fn main() {}`.
3. Promote `fn ` → `pub fn ` (auto; `pub func` doesn't work — `pub` leaks to next stmt).
4. Append `std/cstd_native.rs` (deprecated; see policy below).
5. Wrap in `#[allow(dead_code)] pub mod cstd { ... }` and prepend to `result.value`.

**Constraints for `std/cstd.crs`:**
- No multi-line method chains — tokenizer terminates at newlines; use temp vars.
- No `&[T]` slice params — tokenized as vec literal (`&vec![T]`). Use `Vec<T>` or native.
- No `cfg!(target_os = "windows")` — `parse_var` injects `let` before `=`, breaking macro.

## Standard-library policy

**`std/*.crs` is written in Copper, not Rust.** `std/*_native.rs` is deprecated — do not add new native files. When Copper can't express something yet, fix the transpiler. A `.crs` may use Rust-native spellings, fully-qualified paths, method chains, generics, `format!` specifiers — those all transpile through.

## `unsafe` support

- `unsafe { ... }` blocks pass through (keyword in `parse_any` spaced-keyword list).
- `unsafe func ...`: one-token lookahead swallows `unsafe` before `func`, sets `pending_unsafe_fn` → `result.enter_unsafe_function()` emits `unsafe fn `. Without the swallow, `unsafe` fuses with the next statement (`unsafelet x = ...`).
- `*const T` / `*mut T` work because `const`, `static`, `dyn`, `async`, `await`, `extern`, `where` are in the spaced-keyword list.

## Known limitations

**Transpiler:**
- **No generics in free-function parameters** — `func<T> name(arg: T)` not supported.
- **`?:` ternary inside `${expr}`** not rewritten (captured opaque before `ternary::rewrite`).
- **`obj?.field`** → `Option<&T>` (reference). Non-`Copy` payloads may need `.clone()`.
- **Type aliases** (`type Foo = Bar<T>`) don't lower inside `static`/`as` casts.

**Alloy:**
- **No static borrow/ownership checking** — dynamic `Rc<RefCell>`; pointer aliasing doesn't propagate. Use `cforge` or `alloy check` for real borrow-checking.
- **`&&`/`||` are not short-circuit** — both operands pre-evaluated before `eval_binary`.
- **Wasm ABI is prototype-only** — `i64`/`bool` + `extern "C"` only. Strings/structs = next phase.
- **`.loy` v2** bundles wasm from imported `.rs`; self-contained, no rustc needed at run time.

## Testing & verification

```sh
cargo build                    # debug
cargo build --release          # release (what install.py runs)
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test                     # tests in copper-syntax interpolation
for f in examples/copper/*.crs; do ./target/debug/cforge run "$f"; done
python scripts/install.py      # reinstall; open NEW terminal after
```

CI: fmt / clippy (`-D warnings`) / check / test / build on Linux, Windows, macOS.

`src/main.rs` has `#![allow(...)]` for ~11 legacy clippy categories. Drop entries when rewriting covered modules.

**Hooks:** activate once with `python scripts/hooks.py`. `pre-commit` = fmt+clippy; `pre-push` = test. Don't disable. Add false-positive lints to the allow-list with a comment, don't use `--no-verify`.

**Toolchain:** `rust-toolchain.toml` pins `stable`. After `rustup update`, run `cargo clean` once (proc-macro DLLs are ABI-tied to the compiler version).

## Communication conventions

- PT-BR replies, terse, occasional code blocks. Regressions: exact output + expected.
- Non-trivial design decisions: brainstorm 2-3 options with tradeoffs before implementing.
- After a task: files touched + behaviour change. No step-by-step recap of tool calls.
