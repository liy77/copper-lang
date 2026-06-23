# copper-lsp

> Language server for the [Copper](../../README.md) language (`.crs`).

A [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)
implementation (JSON-RPC over stdio) that gives editors live feedback on Copper
source. Built on [`copper-syntax`](../copper-syntax)'s **AST** — the same parser
the interpreter uses — so the editor never drifts from the compiler.

## Features

- **Diagnostics** — surfaces `copper-syntax` parse errors as you type.
- **Completion** — keywords, types, imported symbols, stdlib methods.
- **Hover** — docs for builtins and stdlib.
- **Go-to-definition** for imports.

## Layout

```
src/
├── main.rs              stdio JSON-RPC entrypoint (tower-lsp + tokio)
├── server.rs            the Backend (request handlers)
├── docs.rs              hover/documentation content
├── imports.rs           import resolution
├── lexicon.rs           keyword/type catalog for completion
├── rust_prelude.rs      Rust prelude symbols surfaced to Copper
└── stdlib_methods.rs    builtin method catalog
```

## Running

The binary is `copper-lsp`. The installer ships it into `$COPPER_PATH/bin`, where
editor integrations (e.g. OndaEngine) resolve it. Point your LSP client at the
`copper-lsp` executable for `.crs` files.
