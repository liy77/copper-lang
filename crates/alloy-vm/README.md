<p align="center">
  <img src="../../assets/alloy/alloy-logo.png" alt="Alloy" width="180">
</p>

<h1 align="center">Alloy</h1>

<p align="center">
  <em>Copper's fast-iteration VM — a tree-walking interpreter.</em>
</p>

---

## What it is

**Alloy** is the [Copper](../../README.md) interpreter: a separate binary
(`alloy`) that **executes `.crs` directly**, without transpiling to Rust or
invoking `cargo`. It walks the same AST that the production transpiler (`cforge`)
uses — so what runs in Alloy is the same Copper that compiles natively.

Two targets, one language:

| Tool | Path | Purpose |
| --- | --- | --- |
| `cforge` | `.crs` → Rust → `cargo build` | native release build |
| **`alloy`** | `.crs` → AST → interpretation | fast iteration, scripting, REPL |

Rust compatibility is the central invariant: **the same `.crs` produces the
same behaviour** on both paths.

## Usage

```sh
alloy run program.crs
```

Example (`examples/copper/alloy-hello.crs`):

```rust
func int sum_to(n: int) {
    mut total = 0
    for i in 1..n {
        total += i
    }
    return total
}

func main() {
    println("Alloy VM")
    mut s = sum_to(5)
    println("sum 1..5 = ${s}")
}
```

```sh
$ alloy run examples/copper/alloy-hello.crs
Alloy VM
sum 1..5 = 10
```

## Supported today (MVP)

- Scalar literals (`int`, `float`, `bool`, `str`) and `"${expr}"` interpolation.
- Binary/unary operators, ternary, assignment (`=`, `+=`, …), `++`/`--`.
- Control flow: `if`/`else`, `while`, `loop`, `for x in a..b`,
  `break`/`continue`.
- User functions, calls and recursion.
- Built-ins `println` / `print`.

Integer arithmetic is **checked** (overflow and division/remainder by zero
become runtime errors, never a panic). Constructs outside the MVP subset
(struct/enum/impl/match/closures, generics, traits) report a clear runtime
error — they are on the roadmap.

## Roadmap

See the full design in
[`docs/superpowers/specs/2026-06-22-alloy-vm-design.md`](../../docs/superpowers/specs/2026-06-22-alloy-vm-design.md):

1. ✅ Tree-walking core (expr, vars, control flow, functions, `println`).
2. Composite types: struct/class/enum/impl/match/closures.
3. Generics + traits.
4. Rust interop: registered stdlib + cached C-ABI shim on demand.
5. `alloy build` (self-contained binary) + `alloy repl`.
6. *(future)* bytecode backend and/or Cranelift JIT as an optimisation.

## Architecture

```
crates/copper-syntax::program  (Program AST — parsed bodies)
            │
            ▼
crates/alloy-vm                bin: alloy
(tree-walking interpreter)     (run / build / repl / check)
```

- `value.rs` — runtime values (`Value`).
- `env.rs` — chained scopes (`Env`).
- `error.rs` — `RuntimeError` + control flow.
- `interp.rs` — the interpreter (`Interpreter`).
- `src/bin/alloy.rs` — the CLI.

The crate is **isolated**: `cforge` and the `mui-*` crates do not depend on it,
so cross-platform CI is unaffected.
