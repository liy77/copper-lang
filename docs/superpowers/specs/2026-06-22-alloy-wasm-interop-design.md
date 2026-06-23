# Alloy ⇄ Rust via embedded WebAssembly

**Status:** prototype landed — scalar `i64`/`bool` run via wasm in `alloy run` /
`cforge vm run`, **and embedded into `.loy`** by `alloy build` / `cforge vm
build` (v2 format, self-contained: runs with no rustc and no `.rs` on disk).
Richer ABI (strings/structs) + auto-wrappers = next phases.
**Date:** 2026-06-22
**Crates:** `crates/alloy-vm` (`wasm.rs`, `loader.rs`, `interp.rs`, `bin/alloy.rs`)

## Problem

Alloy is a tree-walking interpreter for Copper; it cannot execute Rust. When a
`.crs` imports a sibling `.rs` (`import { add } from math`), Alloy historically
gave up and delegated the **whole** program to `cforge` (transpile → rustc →
`cargo run`) via `LoadOutcome::NeedsCforge`. That loses Alloy's two reasons to
exist: instant startup and a portable `.loy` artifact.

We want Alloy itself to **run the Rust**, while keeping:
- **Instant `run`** for the Copper part (interpreted) and for repeat runs.
- **Portability** — the `.loy` must still run on any platform.

## Why not embed rustc/Miri

Investigated and rejected (see chat + `CLAUDE.md`'s "do not fork rustc"):

- **Miri is not separable from rustc.** Its MIR interpreter lives in
  `rustc_middle` and depends entirely on `TyCtxt` (interners, the query system,
  layouts, monomorphization, const-eval global memory). There is no standalone
  MIR interpreter; "porting Miri" = shipping a nightly rustc (hundreds of MB),
  killing portability and instant startup.
- **cg_clif's JIT** runs whole programs as part of a compiler invocation; it is
  not an embeddable library for compiling Rust *source* at runtime, and
  Rust→CLIF still needs the rustc front-end.

## Decision: compile Rust → WASM, run with an embedded wasm engine

WebAssembly **is** a portable bytecode — exactly what `.loy` already is (today a
bincoded AST). So:

- **Build time** (slow, acceptable): `rustc --target wasm32-unknown-unknown`
  compiles each imported `.rs` to a wasm module.
- **Run time** (fast): an embedded wasm engine ([`wasmi`], pure-Rust → trivial
  cross-compile; `wasmtime`/Cranelift later as a speed mode) instantiates and
  calls it. The Copper code stays interpreted; only the Rust leaves cross into
  wasm.

### `alloy run` flow

```
alloy run file.crs
  ├─ no .rs imports        → pure interpretation (unchanged, instant)
  ├─ .rs imports
  │    ├─ compile each .rs → wasm  (cached by content hash under ~/.alloy/cache)
  │    │     1st run: pays rustc;  later runs / unchanged .rs: cache hit → instant
  │    ├─ instantiate via wasmi; register exports as interpreter functions
  │    └─ interpret Copper; at an imported-name call, marshal args → wasm → result
  └─ app.loy               → if it carries wasm, instantiate & run; else interpret
```

The instant-iteration loop (edit **Copper**, rerun) stays instant because the
`.rs` is unchanged → its wasm is cached. You only pay rustc when the `.rs`
changes — inherent to running real Rust.

### `alloy build` (distribution) — **done**

The compiled wasm is embedded into the `.loy` so it's self-contained:
`bytecode.rs` v2 payload = `Vec<Item>` + `Vec<WasmModule { names, bytes }>`
(`FMT_VERSION` 1→2, v1 still loadable). `alloy run app.loy` /
`cforge vm run app.loy` then need **no rustc** — verified by clearing the cache
and deleting the `.rs`: the `.loy` still runs the Rust.

## Interop ABI

| Phase | Types | Mechanism |
| --- | --- | --- |
| **Prototype (done)** | `i64` / `bool` scalars | direct wasm `i64` params/results; the `.rs` exports `#[no_mangle] pub extern "C"`. Plain `pub fn` (no export) → graceful fallback to cforge. |
| Next | `f64`, strings, `Vec`, slices | guest memory + an alloc/free ABI (pass ptr+len); host reads/writes linear memory. Requires `wasm32-wasip1` + `wasmi`'s WASI for I/O. |
| Later | structs, `Option`/`Result`, traits | the WebAssembly **Component Model** + `wit-bindgen`, or a generated glue crate. |
| Ergonomics | plain `pub fn` (no `extern "C"`) | auto-generate an export-wrapper crate from the imported signatures so users write idiomatic Rust. |

## Current implementation (prototype)

- **`wasm.rs`** — `WasmRuntime` (wasmi `Store`+`Instance`), `from_rs` (compile +
  instantiate), `call_i64` (dynamic-arity `Val::I64` call), `compile_rs_to_wasm`
  (rustc → `wasm32-unknown-unknown` cdylib, FNV-1a content-hash cache under
  `~/.alloy/cache`, overridable via `ALLOY_CACHE_DIR`).
- **`loader.rs`** — `RsImport { names, rs_path }` + `resolve_runnable_wasm`
  collects `.rs` imports (instead of bailing to `NeedsCforge`). `collect` takes
  an optional accumulator so the old cforge path is preserved for `build`.
- **`interp.rs`** — `Interpreter.wasm: HashMap<name, Rc<RefCell<WasmRuntime>>>`,
  `register_wasm`, and a `call_user` fallback that marshals scalars and calls
  `call_wasm`.
- **`bin/alloy.rs`** — `run` compiles+instantiates each `.rs`, verifies every
  imported name is a wasm export (else falls back to cforge), then interprets.
- **Example** — `examples/copper/rust-interop-wasm/` (`mathwasm.rs` exporting
  `add`/`fib`, `main.crs` importing them). 1st run ≈ rustc cost; cached run
  ≈ 12 ms.

## Open questions / risks

- **Binary size:** wasmi is small; a future `wasmtime` mode adds Cranelift (a
  few MB) — still far below shipping rustc.
- **WASI surface:** stdout/fs from inside wasm needs WASI wired into the
  `Linker`; decide host capabilities (sandboxing is a feature, not a bug).
- **`.loy` compat:** the wasm section bumps `FMT_VERSION`; old artifacts stay
  readable via the version gate.
- **Determinism of cache key:** content hash of the `.rs` only — does not yet
  capture rustc version or edition flags. Add those to the key before shipping.
