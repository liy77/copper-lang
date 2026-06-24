<img src="./assets/copperlang.png"/>

A high-level language that transpiles to Rust. Copper aims to keep Rust's
performance while giving you a syntax that reads like a modern scripting
language — string interpolation, optional chaining, ternaries, and JS-style
loop / match / closure forms — backed by `cforge`, the Copper toolchain.

```crs
import * from std.io

name = input!("What's your name?")
println!("Your name is $name")
```

<p align="center">
    <img src="./assets/cforge.png" width=300 height=300 />
</p>

## Two execution paths

| Path | Tool | How it works | When to use |
| --- | --- | --- | --- |
| **Transpile → Rust** | `cforge` | `.crs` → Rust source → `cargo build` | Production, full Rust semantics |
| **Interpreter** | `alloy` | tree-walks the AST directly, no compile step | Fast iteration, scripting, CI |

`alloy run file.crs` is instant — like `python` or `node`. `cforge run file.crs`
gives you native speed after the first compile. Both share the same
`copper-syntax` AST so behaviour is identical.

## Install

There's also a [native installer](./installer-gui) — built with MUI
(mocida's declarative UI; no web stack, no Tauri), Windows-only for now —
that wraps the script below in a clickable form.

### Script install

See [`docs/INSTALL.md`](./docs/INSTALL.md) for the full guide. TL;DR — one
cross-platform Python installer (needs Python 3.7+):

```sh
python scripts/install.py
```

On Windows you can also double-click `scripts\install.bat`; on Unix run
`bash scripts/install.sh`. Both just forward to `install.py`. The installer
auto-detects admin / root and picks a global or per-user install accordingly.

Pass `--workspace` to also build and install `alloy`, `lson`, and the LSP
servers in one shot.

## Compile and run (cforge)

```sh
cforge run ./main.crs          # transpile + cargo run
cforge -c -i ./src             # compile only, directory input
cforge run examples/copper/loops.crs
```

`cforge run` (no arg) defaults to `./main.crs` in the current directory.

### Flags

| Flag | Full name | Description | Example |
| --- | --- | --- | --- |
| `-i` | `--input`   | Input file or directory | `cforge -c -i main.crs` |
| `-o` | `--output`  | Output directory | `cforge -o ./build` |
| `-t` | `--target`  | Cross-compile target | `cforge -t windows` |
| `-c` | `--compile` | Compile (no run) | `cforge -c -i main.crs` |
|      | `--clean`   | Clean the output directory | `cforge --clean` |
| `-V` | `--verbose` | Verbose output | `cforge -V run main.crs` |
| `-v` | `--version` | Print version (with build date for pre-releases) | `cforge -v` |

## Alloy — the Copper interpreter

`alloy` interprets `.crs` files directly with no compile step and can also
produce a portable `.loy` artifact that runs anywhere without a Rust toolchain.

```sh
alloy run file.crs             # instant — no compile
alloy build file.crs           # compile to a portable .loy artifact
alloy run app.loy              # run the artifact (no rustc needed)
alloy check file.crs           # verify with real Rust + Miri
```

### Portable `.loy` artifacts

A `.loy` is a self-contained binary (magic header + bincoded AST). If the
program imports sibling `.rs` files Alloy compiles them to WebAssembly once
(cached by content hash in `~/.alloy/cache`) and embeds the wasm into the
artifact — so the final `.loy` runs on any platform with no rustc and no
`.rs` files on disk.

```sh
alloy build myapp.crs -o myapp.loy   # bundles sibling .rs as wasm
alloy run myapp.loy                  # works anywhere, no rustc
```

### Importing Rust from Copper

Alloy can call into sibling `.rs` files via WebAssembly — the Copper stays
interpreted (instant); only the Rust crosses into wasm:

```crs
// math.rs  (sibling file)
// #[no_mangle] pub extern "C" fn fib(n: i64) -> i64 { ... }

import { fib } from math

println!("{}", fib(10))   // → 55, runs via embedded wasm
```

Current prototype ABI: `i64`/`bool` scalars with `#[no_mangle] pub extern "C"`.
Richer types (strings, `Vec`, structs) are the next phase.

### Alloy GUI playground

`alloy-gui/` is a dual-mode `alloy` binary that adds a MUI playground — open
a `.crs` file, edit it, and see output live. Hot-reload re-evaluates on save;
resize splits output to the right on wide screens.

## Language features

### Classes

```crs
class Greeter {
    name: String

    Greeter(name: String) {
        self.name = name
    }

    void hello(self) {
        println!("Hello, {}!", self.name)
    }
}

Greeter::new("Brian".to_string()).hello()
```

The class lowers to a Rust `struct` + `impl`. The constructor (same name
as the class) becomes `pub fn new(...) -> Self`, and methods declared as
`void name(self)` become `pub fn name(&self)`. Use `cforge run main.crs`
to see it print `Hello, Brian!`.

### Loops, match, optional chaining, ternary

```crs
mut count = 0
loop {
    count++
    if count == 3 { break }
}

mut user: Option<User> = Some(User { name: "Brian".to_string(), age: 30 })
mut age = user?.age              // Some(30)

mut grade = score >= 9 ? "A" : "B"

match n {
    0 => 0,
    1 | 2 | 3 => 1,
    n if n < 0 => -1,
    _ => 2
}
```

See [`examples/`](./examples) for runnable demos of each feature.

### Built-in `cstd` standard library

Rust's standard library is powerful but verbose for everyday scripting tasks
(reading a line of input, sleeping, getting the current time, running a shell
command). Copper ships an embedded `cstd` module — import what you need and
the compiler injects only the helpers you used:

```copper
import { input, read_int, sleep_ms, now_ms, run, exit, env, exists } from cstd

name = input("Your name? ")
age  = read_int("Age: ")
println!("Hello $name ($age)")

started = now_ms()
sleep_ms(100)
println!("slept ~{}ms", now_ms() - started)

println!("HOME = {}", env("HOME"))
println!("ls -> {}", run("ls"))
```

Available functions: `input`, `readln`, `read_int`, `read_float`, `to_int`,
`to_float`, `trim`, `split`, `join`, `exit`, `die`, `panic_if`, `sleep_ms`,
`now_ms`, `env`, `args`, `read_file`, `write_file`, `append_file`, `exists`,
`is_file`, `is_dir`, `list_dir`, `run`, `rand_int`.

The library lives in [`std/cstd.crs`](./std/cstd.crs) — written in Copper,
shared between the transpiler and Alloy (single source of truth).

## Project layout

```
copper-lang/
├── src/                # cforge compiler entrypoint + legacy re-exports
├── crates/
│   ├── copper-syntax/  # Tokenizer + AST (shared by cforge, alloy, LSPs)
│   ├── copper-parser/  # Streaming transpiler parser
│   ├── alloy-vm/       # Alloy interpreter + `alloy` CLI
│   ├── copper-lsp/     # Copper language server (hover, completion, goto-def)
│   ├── mui-syntax/     # MUI (.mui/.crm) parser
│   ├── mui-codegen/    # MUI → Rust codegen (M5 native path)
│   └── mui-lsp/        # MUI language server
├── alloy-gui/          # Alloy GUI playground (MUI host, standalone workspace)
├── installer-gui/      # Native MUI installer (Windows-first)
├── examples/
│   ├── copper/         # .crs demos (loops, interpolation, matching, cstd, ...)
│   └── mui/            # .mui/.crm demos (hello, counter, keyboard, app, ...)
├── scripts/            # Python tooling: install / build / cleanup / diagnose / hooks
├── docs/               # INSTALL.md, TODO.md, design specs
├── std/                # Copper standard library (cstd.crs)
├── lson/               # LSON parser binaries (per-OS)
├── assets/             # Logos
├── main.crs            # Default file for `cforge run` with no argument
├── properties.kson     # Project metadata + dependencies
├── Cargo.toml          # Workspace root
└── build.rs            # Stamps the build date into pre-release versions
```

## Examples

```sh
# cforge (transpile → Rust)
cforge run examples/copper/loops.crs           # loop / while / for / break / continue
cforge run examples/copper/interpolation.crs   # "Hello $name", "${expr}"
cforge run examples/copper/collections.crs     # vec literals, closures, ?
cforge run examples/copper/matching.crs        # match arms, if let, while let
cforge run examples/copper/optional.crs        # `?.` optional chaining
cforge run examples/copper/ternary.crs         # `cond ? a : b`
cforge run examples/copper/cstd.crs            # built-in stdlib (input, sleep, env, ...)
cforge run examples/copper/unsafe.crs          # `unsafe func` and `unsafe { ... }` blocks

# Alloy (interpreter — instant)
alloy run examples/copper/loops.crs
alloy run examples/copper/cstd.crs
alloy build examples/copper/collections.crs -o collections.loy
alloy run collections.loy

# MUI examples (need mui-dev on PATH):
cforge run examples/mui/hello/hello.mui
cforge run examples/mui/counter/counter.mui
cforge run examples/mui/keyboard/keyboard.mui
cforge run examples/mui/app/app.mui
```

## Roadmap / TODO

Full backlog with details: [`docs/TODO.md`](./docs/TODO.md).

**Language (cforge)**
- [x] Core transpile pipeline (`.crs` → Rust), `cforge run` / `-c`
- [x] Variables, functions, classes/structs/impl, imports
- [x] Loops, `match` (guards/`_`/`|`), `if let` / `while let`
- [x] Optional chaining `?.`, ternary `?:`, string interpolation `"${expr}"`
- [x] `unsafe` blocks + functions, raw pointers, generic return types
- [x] Optional return-type sugar + generic params on `impl` methods
- [x] `cstd` standard library, `.rs` interop (sibling Rust files)
- [x] CalVer versioning + build date in `--version`
- [ ] Generics in free-function parameters (`func<T> name(arg: T)`)
- [ ] `?:` ternary inside `${…}` interpolation
- [ ] Type aliases (`type Foo = Bar<T>`) inside `static`/`as` casts

**Alloy interpreter**
- [x] Tree-walking interpreter for the full `copper-syntax` AST
- [x] `cstd` interpreted from `std/cstd.crs` (single source, no Rust reimplementation)
- [x] Portable `.loy` artifacts (`alloy build` / `alloy run`)
- [x] Imported `.rs` files run via embedded WebAssembly (wasm cache, embedded in `.loy`)
- [x] `alloy check` — auto-provisioned Miri/rustc verification
- [x] Return-type checking at runtime
- [x] Alloy GUI playground (MUI, hot-reload, native file dialog)
- [ ] Wasm ABI: strings, `Vec`, structs (currently `i64`/`bool` scalars only)
- [ ] Auto-generate export wrappers so plain `pub fn` works without `extern "C"`
- [ ] Short-circuit `&&` / `||` (both sides are currently pre-evaluated)

**MUI front-end (`.mui` / `.crm`)**
- [x] `cforge run x.mui` dev render (via `mui-dev`)
- [x] `cforge -c [-r] x.mui` codegen + native build (via `mui-codegen`)
- [x] Component import/reuse, `app { }` config + bundles
- [x] `onKeyInput` keyboard handlers, `Dialog` overlay, `Audio` component
- [x] Common `x:`/`y:` props on every widget
- [ ] Reactive `if`/`for`/`match` in node position (structural codegen — design ready)
- [ ] Codegen feature parity with the interpreter (string signals, conditional color)

**Installer**
- [x] MUI-based native installer (no Tauri/web)
- [x] `--workspace` flag builds and installs alloy + LSPs in one shot
- [ ] Signed installer artifact + CI release packaging

## Contributing

Pull requests are welcome. Please open an issue first for non-trivial changes
so we can discuss the approach.

After cloning, activate the repo's git hooks once so commits and pushes
run the same lint gates CI does:

```sh
python scripts/hooks.py
```

This sets `core.hooksPath` to `.githooks/`. From then on:

- `pre-commit` runs `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`
- `pre-push` runs `cargo test`

Bypass with `--no-verify` only when you have to (the same checks fail in CI
afterwards).
