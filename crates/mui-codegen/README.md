# mui-codegen

> Release codegen for **MUI** (`.mui` / `.crm`) — **M5** in
> `mocida/mui/ARCHITECTURE.md`.

Lowers the [`mui-syntax`](../mui-syntax) component AST into **Rust source** that
builds a [mocida](../../README.md) widget tree via `mocida-rs` (the same builders
the dev runtime walks in memory, emitted instead as readable code). The output
is a self-contained `main.rs`: one `fn <view>() -> Result<Children>` per `view`,
plus a `main()` that opens a window with the entry view.

## The two MUI verbs

```
cforge run foo.mui      → dev render        (M1, mui-dev walks the AST live)
cforge -c [-r] foo.mui  → generate code     (M5, this crate) [+ native build]
```

`-c` writes the Rust crate; `-r` also `cargo build --release`s it into a native
binary.

## Pure AST → text

This crate has **no dependency on `mocida` or the runtime**, so it builds
anywhere (portable CI) and `cforge` can call it without dragging in the C
library. Only the *generated* code depends on `mocida` — and it's meant to be
read, reviewed, and built.

## Layout

```
src/
├── lib.rs              entry: AST → generated Rust source
├── emit.rs             text emission (the widget-tree builder code)
└── eval.rs             compile-time evaluation of prop expressions
```

## Coverage

`Stack`, `Text`, positional labels, `size:` / `color:` / `orientation:` /
`gap:` / `padding:` props, `${...}` interpolation with view-param defaults and
`signal(...)` initial values. Handlers and live reactivity are emitted as TODOs
for now (the M3/M4 evaluator work, ported into codegen later).
