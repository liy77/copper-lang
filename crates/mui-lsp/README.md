# mui-lsp

> Language server for the **MUI** markup language (`.mui` / `.crm`).

A [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)
implementation (JSON-RPC over stdio) for mocida UI files. Driven entirely by
[`mui-syntax`](../mui-syntax) — the real parser + import loader — so the editor
never drifts from the compiler.

## Features

- **Diagnostics** — `mui-syntax` parse errors as you type.
- **Completion** — widgets, props, enum members, imported components.
- **Hover** — widget/prop documentation.
- **Document symbols** — the `view` / element outline.
- **Color swatches** — inline previews for color props.
- **Go-to-definition** for component imports.

## Layout

```
src/
├── main.rs             stdio JSON-RPC entrypoint (tower-lsp + tokio)
├── server.rs           the Backend (request handlers)
├── catalog.rs          widget + prop catalog driving completion/hover
└── docs.rs             documentation content
```

## Running

The binary is `mui-lsp`. The installer ships it into `$COPPER_PATH/bin`. Point
your LSP client at the `mui-lsp` executable for `.mui` / `.crm` files.
