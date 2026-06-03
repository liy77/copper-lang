# MUI — VSCode extension

Editor support for **MUI** (`.mui` / `.crm`), the mocida UI language. Built to
sit beside the [Copper extension](../vscode): MUI embeds Copper expressions, so
the two share conventions (and the same `mut`/bare bindings — **no `let`**).

## Features

- **Syntax highlighting** (`syntaxes/mui.tmLanguage.json`) — `view`
  declarations, element tags (PascalCase), prop names, `${…}` interpolation,
  enum access (`FontStyle.Bold`), Copper keywords/types/operators, comments,
  strings, numbers, macros.
- **Color swatches + picker** (the "cores") — a `DocumentColorProvider` renders
  an inline swatch next to every `#rrggbb`, `#rrggbbaa`, and `rgba()/rgb()`
  literal and lets you edit it with VSCode's color picker. Writes the value
  back in the same style you used (hex stays hex, rgba stays rgba). Toggle with
  the `mui.colorDecorators` setting.
- **Autocomplete** — context-aware:
  - **Node position** → element names (`Stack`, `Text`, `Button`, …) with
    snippets, plus Copper keywords / reactivity (`view`, `mut`, `signal`,
    `effect`, `if`, `for`, `match`).
  - **Inside an element's `( … )`** → that element's prop names (+ the common
    `UIWidget` props).
  - **After `EnumName.`** → enum members (`FontStyle.` → Bold/Italic/…).
  - **After `prop:`** → known values for that prop (`orientation:` →
    vertical/horizontal, etc.).
- **Hover tooltips** (the description "bubbles") — a `HoverProvider` shows a
  short description when you hover an element (`Text`, `Button`, …), a prop
  (`size`, `color`, …), an enum (`FontStyle`) or its members, and Copper
  keywords / types (`func`, `struct`, `impl`, `signal`, `int`, …).
  - **Copper words use the SAME descriptions as the Copper extension.** They
    come from the shared `editors/copper-lexicon.json` — a single JSON read by
    both `copper-lsp` (embedded via `include_str!`) and this extension (at
    runtime), so the `.crs` and `.mui`/`.crm` hovers can never drift. Edit a
    description there once and both update. (Mocida-only words — `view`,
    `signal`, `computed`, `effect` — live in `src/catalog.ts`.)
- **Diagnostics** (error checking) — reported live as you type:
  - **Bracket balance** — unmatched / mismatched / unclosed `()` `[]` `{}`.
  - **Unknown enum members** — e.g. `FontStyle.Boldd` (only for known enums,
    so no false positives on your own types).
  - **Unknown props** on a known widget — e.g. `bogus:` on `Text`. Strings,
    comments, ternaries (`a ? b : c`) and handler blocks are never mistaken
    for props. Toggle with `mui.diagnostics` / `mui.checkUnknownProps`.

The widget/enum/prop catalog lives in `src/catalog.ts` — mirror of the mocida
widget surface (see `mocida/mui/SPEC.md §7`). It is the single source of truth
for completion **and** hover **and** diagnostics; extend that table to grow all
three at once.

## Run it

The fastest way to try it — the **Extension Development Host**:

1. Open the `editors/vscode-mui` folder in VS Code.
2. Press **F5** (runs the *Run MUI Extension* launch config). This compiles
   `src/ → out/`, bundles the shared lexicon, and opens a second VS Code
   window with the extension loaded and the `examples/mui/` folder already open.
3. Open `hello/hello.mui`, `counter/counter.mui`, or `crm/app.crm`
   and you'll see highlighting, color swatches on the `#hex` values, hover
   bubbles, autocomplete (Ctrl+Space), and live diagnostics.

The bundled examples (all under `examples/mui/` in the repo root):

| Folder | Shows |
| --- | --- |
| `hello/` | minimal view — `Stack` / `Text`, props, `${…}`, colors |
| `counter/` | reactive state (`mut x = signal(…)`), `if/else`, handlers |
| `styled/` | widget styling, anchors, border, cursor |
| `app/` | full `App()` block with window identity + `app.bundle` |
| `card/` | reusable component (`view` with typed params) |
| `dashboard/` | cross-file import (`import { Card } from "../card/card.mui"`) |
| `bundle-demo/` | `app.bundle` asset loading via `mocida://` |
| `crm/` | Copper `struct` / `impl` / `func` alongside a `view` |

## Build & package

```sh
npm install        # @types/vscode, @types/node, typescript
npm run compile    # tsc -> out/  (+ bundles copper-lexicon.json into out/)
npm run package    # builds a mui-<version>.vsix (via @vscode/vsce)
```

`npm run package` produces an installable `.vsix`; install it with
**Extensions: Install from VSIX…** or `code --install-extension mui-*.vsix`.
The `compile` step copies `editors/copper-lexicon.json` into `out/` so the
packaged extension shows the exact same Copper keyword descriptions as
`copper-lsp` (in the dev host it reads the repo copy directly).

## Scope

Highlighting, colors, completion, hover, and diagnostics are **self-contained**
(no language server) — driven by the `src/catalog.ts` table. Deeper analysis
(go-to-definition, cross-file checks, full Copper type-checking) would come
later via a MUI language server built on the `mui-syntax` crate, the same way
`copper-lsp` backs the Copper extension.
