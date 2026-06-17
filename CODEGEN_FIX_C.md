# Codegen Fix C — user-defined `main()`

## Root cause
Copper auto-wraps top-level statements into `fn main() { ... }`
(`Result::write_main_function` in `crates/copper-parser/src/parser/result.rs`).
A user who *also* wrote `func <type> main()` got:
1. a duplicate `fn main` (`error[E0428]`), and
2. an invalid non-unit return type on `fn main` (`error[E0277]`) when the user
   main returned an int.

A second, latent tokenizer bug surfaced once the rename was attempted: in
`func main()` (void return, no declared type) the tokenizer
(`crates/copper-syntax/src/tokenizer/tokenizer.rs`) treats the *first*
identifier after `func` as the return type. So `main` was consumed as the
return type, leaving the function nameless (`fn () { ... }`,
`error: expected identifier, found (`).

## Fix
Three layers:

1. **Tokenizer disambiguation** (`tokenizer.rs`): added `peek_significant_char`
   and a new branch — if the first identifier after `func` is immediately
   followed by `(` (skipping spaces), it is the function *name* (void return),
   emitted as `Identifier` instead of `ReturnType`. `func Type name(...)` is
   unchanged.

2. **Rename on emit** (`parser/mod.rs`): a new `expect_function_name` flag is
   armed when a Copper `func` keyword is consumed. When the name identifier is
   emitted in `parse_any`, if it is `main` the symbol is rewritten to
   `__copper_main` and `Result::set_user_main(kind)` records the return kind
   (`Int` if a non-unit return type was captured, else `Unit`).

3. **Entry synthesis + suppression** (`result.rs`): added
   `enum ReturnKind { Int, Unit }` and `user_main: Option<ReturnKind>`.
   `write_main_function` now branches:
   - `user_main = Some(Int)` → `fn main() { std::process::exit(__copper_main() as i32); }`
   - `user_main = Some(Unit)` → `fn main() { __copper_main(); }`
   - the auto-wrapper is **not** emitted when a user main exists (no duplicate).
   - "meaningful" top-level code is detected by ignoring whitespace and stray
     `;` separators (a user `func` leaves a lone `";\n"` in
     `main_function_code`). If there *is* real top-level code **and** a user
     main, emit `compile_error!("a program cannot have both top-level
     statements and a `main` function; use one or the other")`.

## crun.sh — BEFORE / AFTER

BEFORE (bugC):
```
error[E0428]: the name `main` is defined multiple times
error[E0277]: `main` has invalid return type `i64`
```

AFTER:

| repro | result | output | exit |
| --- | --- | --- | --- |
| `bugC.crs` (`func int main` → return 0) | builds + runs | `hello from main` | 0 |
| `bugC42.crs` (return 42, extra check) | builds + runs | `ret 42` | 42 (exit wrapper propagates) |
| `bugCvoid.crs` (`func main` void) | builds + runs | `void main` | 0 |
| `toplevel.crs` (no func main) | builds + runs | `x+1 = 42` | 0 |

Generated `fn main` for bugC:
```rust
fn __copper_main() -> i64 {
    println!("hello from main");
    return 0;
}
fn main() { std::process::exit(__copper_main() as i32); }
```
Generated for bugCvoid:
```rust
fn __copper_main() -> () { println!("void main"); }
fn main() { __copper_main(); }
```

## Conflict case
`conflict.crs` (top-level statements **and** `func int main`) transpiles to a
`compile_error!("a program cannot have both top-level statements and a `main`
function; use one or the other")` line (plus the renamed function + entry), so
the ambiguity is reported clearly at compile time rather than silently dropped.

## Regression tests
`crates/copper-parser/tests/user_main.rs` (4 tests, all green):
- `int_main_renamed_with_exit_wrapper` — `fn __copper_main() -> i64`,
  `std::process::exit(__copper_main() as i32)`, exactly one `fn main(`.
- `void_main_renamed_with_plain_wrapper` — `__copper_main();`, no
  `std::process::exit`, exactly one `fn main(`.
- `top_level_only_emits_single_main` — exactly one `fn main(`, no rename.
- `both_top_level_and_user_main_is_a_compile_error` — output contains
  `compile_error!`.

## Suite result
`cargo test -p copper-parser -p copper-syntax` — all pass (parser integration
suites + 4 new + copper-syntax 42 unit + grammar conformance). 0 failures.
