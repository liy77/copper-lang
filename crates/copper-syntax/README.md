# copper-syntax

> Tokenizer + AST for the [Copper](../../README.md) language.

The **shared front-end** of the Copper toolchain. Every consumer that needs to
understand Copper source starts here:

| Consumer | What it uses |
| --- | --- |
| `cforge` (via `copper-parser`) | the **token stream** — walked directly to emit Rust |
| `copper-lsp` | the **AST** — for diagnostics, hover, completion |
| `alloy` (`alloy-vm`) | the **AST with parsed bodies** — walked by the interpreter |
| `mui-syntax` | the **tokenizer** — embedded Copper expressions in `.mui` |

Because all paths share this crate, the language never drifts between the
transpiler, the interpreter, and the editor.

## Layout

```
src/
├── tokenizer/          char-level scanner (kind, tokens, tokenizer, interpolation)
├── lexicon.rs          keyword / type tables
├── ast.rs              item-level AST (Item, Func, Struct, Import, Span, …)
├── expr.rs             expression AST (ExprKind, BinOp, Literal, StrTemplate, …)
├── program.rs          `parse_program` → Program (items + parsed bodies + errors)
└── utils/              helpers
```

## Two views of the same source

- **Token stream** — `tokenizer::Tokenizer::new(src).tokenize()`. The transpiler
  consumes this; emission is line-oriented and order-sensitive (see the
  tokenizer invariants in the root `CLAUDE.md`).
- **AST** — `program::parse_program(src)` → `Program`. **Recoverable**: malformed
  input yields a partial tree plus `ParseError`s in `prog.errors` rather than
  aborting. The interpreter and LSP both rely on this.

## Notes

- Serde derives live on the AST types so Alloy can serialize a `Program` into its
  portable `.loy` artifact.
- No dependency on `cforge`, `mocida`, or any runtime — pure, portable, CI-safe.
