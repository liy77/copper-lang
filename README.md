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

## Install

See [`docs/INSTALL.md`](./docs/INSTALL.md) for the full guide. TL;DR:

| Platform | Command |
| --- | --- |
| Windows  | `scripts\install.bat` |
| Linux    | `bash scripts/install.sh` |
| macOS    | `bash scripts/install-mac.sh` |

The installer auto-detects admin / root and picks a global or per-user
install accordingly.

## Compile a project

```sh
cforge -c -i ./src
```

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

## Run a file

```sh
cforge run ./main.crs
```

`cforge run` (no arg) defaults to `./main.crs` in the current directory.

## Language features

### Classes

```crs
class Person {
    name: String
    name2: String
    inttest: i32

    Person(name: String, name2: String) {
        self.name = name
        self.name2 = "Carlos".to_string()
        self.inttest = 32
    }

    void test(self) {
        println!("{} {}", self.name, self.name2)
    }
}

Person::new("Liy".to_string(), "Jones".to_string()).test()
```

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

## Project layout

```
copper-lang/
├── src/                # Compiler source (Rust)
├── examples/           # Runnable .crs sample programs
├── scripts/            # install / build / cleanup / diagnose / uninstall
├── docs/               # INSTALL.md and other guides
├── std/                # Copper standard library (.crs)
├── lson/               # LSON parser binaries (per-OS)
├── assets/             # Logos
├── main.crs            # Default file used by `cforge run` with no argument
├── properties.kson     # Project metadata + dependencies
├── Cargo.toml          # Rust crate definition (cforge)
├── build.rs            # Stamps the build date into pre-release versions
└── README.md
```

## Examples

```sh
cforge run examples/loops.crs           # loop / while / for / break / continue
cforge run examples/interpolation.crs   # "Hello $name", "${expr}"
cforge run examples/collections.crs     # vec literals, closures, ?
cforge run examples/matching.crs        # match arms, if let, while let
cforge run examples/optional.crs        # `?.` optional chaining
cforge run examples/ternary.crs         # `cond ? a : b`
```

## Contributing

Pull requests are welcome. Please open an issue first for non-trivial changes
so we can discuss the approach.
