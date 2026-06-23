# mui-syntax

> Parser + component AST for **MUI** (`.mui` / `.crm`) — the
> [mocida](../../README.md) UI markup language.

Turns `.mui` / `.crm` source into a component AST: `view` declarations, their
params, and the nested element/children tree (`let` / `if` / `for` / `match` /
`effect` nodes), with element args and prop expressions lowered into
[`copper-syntax`](../copper-syntax) `Expr` trees.

Lexing **reuses Copper's own tokenizer**, so MUI and Copper stay in lockstep:
embedded Copper expressions (`${...}`, handlers, signal initializers) parse with
exactly the same rules as a `.crs` file.

## Consumers

| Crate | Uses mui-syntax for |
| --- | --- |
| [`mui-codegen`](../mui-codegen) | the AST → Rust source (release codegen, M5) |
| [`mui-lsp`](../mui-lsp) | the AST + import loader → editor features |
| `mui-dev` / `cforge` | the dev render path (M1) |

## Layout

```
src/
├── lib.rs              `parse(source) -> Document` (never panics; errors in Document::errors)
├── ast.rs              Document, View, Element, Node, Prop, Handler, MuiValue, …
├── loader.rs           cross-file component imports (Registry)
└── style.rs            color / anchor / shadow parsing (Rgba, Anchor, ShadowSpec)
```

## Notes

- **Recoverable** like `copper-syntax`: malformed input yields a partial
  `Document` with errors collected in `Document::errors`.
- Pure parser — no dependency on `mocida` or the runtime, so it builds anywhere.
- See `mocida/mui/ARCHITECTURE.md` for where this sits in the MUI pipeline.
