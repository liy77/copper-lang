//! Regression tests for two struct-literal codegen bugs that produced
//! Rust which failed to *compile* (the older string-only tests missed them):
//!
//! * Bug A — a `let`/`mut` binding whose value is a struct literal got no
//!   terminating `;` (`let mut p = Point { x: 1, y: 2 }` ran straight into the
//!   next statement). A struct literal in value position is an expression and
//!   needs the `;`, unlike a block close (`if {...}`).
//! * Bug B — a string literal used as a struct field value was emitted as a
//!   bare `&str` (`name: "Ana"`), which does not coerce to a `String` field.
//!   It is now emitted as `"Ana".into()`.
//!
//! Each test asserts on the emitted Rust; the `compiles_*` tests additionally
//! shell out to `rustc` (skipped automatically if `rustc` is unavailable) to
//! prove the output actually builds.

use copper_syntax::tokenizer::tokenizer::Tokenizer;
use std::io::Write;
use std::process::Command;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

/// Compile `rust_src` with `rustc` into a throwaway binary. Returns `Ok(())`
/// on success, `Err(stderr)` on a compile error, and `Ok(())` (skip) if
/// `rustc` cannot be found.
fn rustc_compiles(rust_src: &str, stem: &str) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("copper_slc_{stem}"));
    let _ = std::fs::create_dir_all(&dir);
    let src_path = dir.join("main.rs");
    let out_path = dir.join("out_bin");
    {
        let mut f = std::fs::File::create(&src_path).map_err(|e| e.to_string())?;
        f.write_all(rust_src.as_bytes()).map_err(|e| e.to_string())?;
    }
    let output = match Command::new("rustc")
        .arg("--edition=2021")
        .arg(&src_path)
        .arg("-o")
        .arg(&out_path)
        .output()
    {
        Ok(o) => o,
        // rustc not installed in this environment — skip the compile leg.
        Err(_) => return Ok(()),
    };
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

// Space-separated fields (no commas), as the language allows.
const POINT: &str =
    "struct Point { x: int  y: int }\nmut p = Point { x: 1, y: 2 }\nprintln!(\"{} {}\", p.x, p.y)\n";

const USER: &str = "struct User { name: string  age: int }\n\
  mut u = User { name: \"Ana\", age: 30 }\n\
  println!(\"{} {}\", u.name, u.age)\n";

#[test]
fn struct_literal_binding_is_terminated() {
    // Bug A: the `}` that closes the struct literal must be followed by `;`.
    let rust = transpile(POINT);
    // Isolate the binding (`let mut p = Point { ... }`), not the struct
    // *definition* (`struct Point { ... }`), then check its closing `}` is
    // followed by `;`.
    let binding = rust
        .split_once("= Point")
        .map(|(_, rest)| rest)
        .expect("struct-literal binding not emitted");
    let closed = binding
        .split_once('}')
        .map(|(_, rest)| rest.trim_start())
        .unwrap_or("");
    assert!(
        closed.starts_with(';'),
        "struct-literal binding not terminated with `;`: {rust}"
    );
}

#[test]
fn struct_literal_string_field_is_coerced() {
    // Bug B: a string literal field value is emitted as `"..".into()`.
    let rust = transpile(USER);
    assert!(
        rust.contains("\"Ana\".into()"),
        "string field not coerced with .into(): {rust}"
    );
}

#[test]
fn space_separated_struct_fields_are_separated() {
    // The struct definition's space-separated fields must not fuse
    // (`x:i64y:i64`); each field is its own `name: Type,` entry.
    let rust = transpile(POINT);
    assert!(
        !rust.contains("i64y"),
        "space-separated struct fields fused together: {rust}"
    );
}

#[test]
fn bare_string_binding_stays_str() {
    // Guard: a plain `mut name = "Brian"` (no struct) must NOT get `.into()`.
    let rust = transpile("mut name = \"Brian\"\nprintln!(\"{}\", name)\n");
    assert!(
        rust.contains("\"Brian\"") && !rust.contains("\"Brian\".into()"),
        "bare string binding should stay &str: {rust}"
    );
}

#[test]
fn compiles_point() {
    let rust = transpile(POINT);
    if let Err(e) = rustc_compiles(&rust, "point") {
        panic!("generated Rust failed to compile:\n{e}\n--- src ---\n{rust}");
    }
}

#[test]
fn compiles_user() {
    let rust = transpile(USER);
    if let Err(e) = rustc_compiles(&rust, "user") {
        panic!("generated Rust failed to compile:\n{e}\n--- src ---\n{rust}");
    }
}
