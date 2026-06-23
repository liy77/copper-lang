# copper-parser

> The Copper → Rust **transpiler parser** — the emit pass `cforge` drives.

Consumes the token stream from [`copper-syntax`](../copper-syntax) and walks it,
emitting Rust source into an output buffer. This is one half of the compiler
pipeline:

```
.crs ─► copper-syntax (tokenize) ─► copper-parser (emit) ─► Rust source ─► cargo build
```

The [`copper-lsp`](../copper-lsp) language server and the [`alloy`](../alloy-vm)
interpreter take the **AST** branch of `copper-syntax` instead and never touch
this crate.

## How it works

`Parser::parse()` is one big dispatch loop. Each token is routed to a `parse_*`
method that appends Rust to a two-stream buffer (`result::Result`):

- `value` — module-level items (structs, impls, fns).
- `main_function_code` — top-level statements, wrapped into `fn main()` at the end.

Before the loop runs, token rewrites lower "macro-like" forms (e.g.
`ternary.rs`: `cond ? a : b` → `if cond { a } else { b }`).

## Layout

```
src/
├── lib.rs
└── parser/
    ├── mod.rs            dispatch loop + parse_* methods, brace/chain depth tracking
    ├── result.rs         output buffer (module stream + main-function stream)
    ├── ternary.rs        token rewrite for ternaries
    ├── utils.rs          type aliases (int → i64, …)
    └── scope*.rs         (mostly stub) scope tracking
```

## Invariants

This crate carries the subtle, order-sensitive logic of the transpiler —
function brace depth, optional-chain closing, `let`-injection in `parse_var`,
`match`-arm separators. See the **Parser** section of the root `CLAUDE.md`
before changing anything here; several of these bugs have bitten the repo before.
