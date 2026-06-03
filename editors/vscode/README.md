# Copper for VSCode

Language support for [Copper](https://github.com/liy77/copper-lang) (`.crs`).

## Features

- Syntax highlighting (TextMate grammar).
- Diagnostics (syntax errors, brace/paren/bracket balance).
- Document symbols (functions, structs, classes, impls, imports).
- Hover hints for built-in keywords and type aliases.
- Completion: keywords, file-local symbols, `cstd::*` helpers.

## Requirements

You need the `copper-lsp` binary on `PATH` (or set `copper.serverPath`). Build it from the copper-lang repo:

```sh
cargo build --release -p copper-lsp
# Copy target/release/copper-lsp to a PATH directory, or:
# set copper.serverPath in VSCode settings to its absolute path.
```

## Settings

| Setting | Default | What it does |
|---------|---------|--------------|
| `copper.serverPath` | `copper-lsp` | Executable to launch as the LSP server. |
| `copper.trace.server` | `off` | LSP message tracing — `off` / `messages` / `verbose`. |

## Building the extension

```sh
npm install
npm run compile
```

For local testing: `F5` in VSCode opens an Extension Development Host with the extension loaded.
