# Copper codegen fixes — struct-literal `;` (Bug A) + string-field coercion (Bug B)

Branch `fix/copper-codegen` in `/Volumes/MCUDevDisk/copper-lang/.worktrees/codegen`.

Struct-using programs failed to **compile** the generated Rust. The existing
suite only checks transpiled strings, so it missed them. Verified each fix by
actually building + running the generated Rust via `crun.sh`.

While reproducing the two named bugs I found a third, more fundamental blocker:
**space-separated struct *definition* fields fused** (`x: int  y: int` →
`x:i64y:i64`), which is what produced the first rustc errors. The task's repros
use space-separated fields, so this had to be fixed too. All three fixes share
the struct-literal/struct-field detection and are reported together.

---

## Root causes

### Field-fusion in struct *definitions* (blocker, surfaced first)
`parse_struct_definition` (`crates/copper-parser/src/parser/mod.rs`) flushed an
accumulated field only on a `Comma` or `Newline` token. For same-line,
space-separated fields (`x: int  y: int`) there is neither between fields, so
the next field name (an `Identifier` arriving while not in name position) was
appended to the previous field's *type* → `x:i64y:i64`, `done:boolpriority:int`.
The reflect codegen then recorded the wrong field names too.

Additional wrinkles found by token-dumping the loop:
- the `:` separator arrives as `Operator(":")` in some contexts, not the
  dedicated `Colon` token;
- a primitive type keyword like `bool` arrives as `Keyword`, handled by the
  catch-all `_ =>` arm (neither the `Identifier` nor the `Type` arm).

### Bug A — struct-literal binding had no terminating `;`
The newline→`;` decision lives in the **tokenizer**
(`crates/copper-syntax/src/tokenizer/tokenizer.rs`, `line_break_token`). A line
*ending in `}`* (previous token `BraceEnd`) suppresses the `;`, which is correct
for a block close (`if {...}`, fn body) but wrong for a struct literal in value
position (`mut p = Point { x: 1, y: 2 }`) — that is an expression and needs the
`;`. The next line then began with `println` → `error: expected ;`.

### Bug B — string literal in a struct field wasn't coerced
A plain string literal emitted as `&str` (`name: "Ana"`), but the field is
`String` → `error[E0308]: mismatched types`. The transpiler emitted string
literals verbatim with no struct-field-value context.

---

## The fixes

**Tokenizer** (`crates/copper-syntax/src/tokenizer/tokenizer.rs`,
`crates/copper-syntax/src/tokenizer/tokens.rs`):
- Added a `struct_brace: bool` field to `Token`. The tokenizer already knows
  (via its `brace_is_struct` stack) whether a `{`/`}` opens/closes a struct
  literal; it now stamps that decision onto the brace token so the parser can
  read it. (Guarded so the speculative zero-length `self.token(Symbol, "")` at
  the end of `symbol_token` does not clobber a real brace's flag.)
- Added `last_closed_brace_was_struct`, set when a `}` pops a struct-literal
  entry. `line_break_token` now emits the terminator after such a `}` — `;` at
  statement level, `,` when the literal is itself a field value nested inside an
  enclosing match arm / struct literal. (Bug A.)

**Parser** (`crates/copper-parser/src/parser/mod.rs`):
- `parse_struct_definition`: added a `seen_type` flag. A name-position
  `Identifier` arriving after a complete `name: type` now flushes the field
  first (same-line separation), `Operator(":")` is treated like `Colon`, and the
  `_ =>` arm marks `seen_type` for keyword types like `bool`. (Field fusion.)
- Main loop: a `struct_lit_stack: Vec<Option<usize>>` mirrors brace nesting
  using the token's `struct_brace` flag; `Some(depth)` records the delim depth
  just inside a struct literal. A `String` token whose enclosing struct-literal
  entry is `Some(d)` **and** whose current depth is exactly `d` (i.e. a direct
  field value, not buried in a nested call/array) is emitted as `"..".into()`.
  Rust infers `.into()` to the field type (works for `String` and `&str`); a
  bare `mut name = "Brian"` (no struct) is untouched. (Bug B.)

---

## crun.sh BEFORE / AFTER

### bugA.crs — `struct Point { x: int  y: int }` / `mut p = Point { x: 1, y: 2 }`
BEFORE:
```
error: found single colon in a struct field type path
error: expected `,`, or `}`, found `:`
error: expected `;`, found `println`
--- BUILD ERRORS ---
```
AFTER:
```
--- RUN ---
1 2
```

### bugB.crs — `struct User { name: string  age: int }` / `mut u = User { name: "Ana", age: 30 }`
BEFORE:
```
error: found single colon in a struct field type path
error: expected `,`, or `}`, found `:`
error: expected `;`, found `println`
--- BUILD ERRORS ---
```
(after the field-fusion + `;` fixes, before the coercion fix it was
`error[E0308]: mismatched types`.)

AFTER:
```
--- RUN ---
Ana 30
```

### bugAB.crs — reflect import + 3-field struct (string/bool/int), no manual `;`/`.to_string()`
BEFORE:
```
error: found single colon in a struct field type path
error: expected `,`, or `}`, found `:`
--- BUILD ERRORS ---
```
AFTER:
```
--- RUN ---
type=Todo n=3
k=title
k=done
k=priority
```

Final generated Rust for bugB (note `;` after `}` and `"Ana".into()`):
```rust
struct User {
    name: String,
    age: i64,
}
fn main() {
    let mut u = User {
        name: "Ana".into(),
        age: 30,
    };
    println!("{} {}", u.name, u.age);
}
```

---

## Regression tests added

`crates/copper-parser/tests/struct_literal_codegen.rs` (6 tests):
- `struct_literal_binding_is_terminated` — the literal's closing `}` is followed
  by `;` (Bug A).
- `struct_literal_string_field_is_coerced` — emits `"Ana".into()` (Bug B).
- `space_separated_struct_fields_are_separated` — fields don't fuse (`i64y`).
- `bare_string_binding_stays_str` — `mut name = "Brian"` stays `&str`, no
  `.into()` (no-regression guard).
- `compiles_point`, `compiles_user` — shell out to `rustc` and actually compile
  the generated Rust (auto-skip if `rustc` is unavailable). Both pass here.

---

## Test suite

`cargo test -p copper-parser -p copper-syntax`: **all green**, including the new
6 tests. No prior tests changed or removed.

## No-regression checks (verified by running)
- `mut name = "Brian"` → `let mut name = "Brian";` (bare `&str`, no `.into()`).
- `if n > 3 { ... }`, `for i in 0..3 { ... }`, `while ... { ... }` blocks get no
  spurious `;` after their closing `}` (a small program with all three builds
  and runs).
