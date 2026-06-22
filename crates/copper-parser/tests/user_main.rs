//! Codegen regression tests for a user-defined `func main()`.
//!
//! Copper auto-wraps top-level statements into `fn main() { ... }`. A user who
//! *also* writes `func <type> main()` used to produce two `fn main`s (a
//! duplicate-symbol error) plus an invalid non-unit return type. The fix
//! renames the user's function to `__copper_main` and synthesizes the real
//! `fn main` entry point that calls it (exiting with its return value for an
//! int return, or just calling it for a unit return), suppressing the
//! colliding auto-wrapper.

use copper_syntax::tokenizer::tokenizer::Tokenizer;

/// Full transpile: Copper source → Rust source.
fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

/// Count non-overlapping occurrences of `fn main(` in the generated Rust.
fn count_main(rust: &str) -> usize {
    rust.matches("fn main(").count()
}

#[test]
fn int_main_renamed_with_exit_wrapper() {
    let rust = transpile("func int main() {\n  println!(\"hi\")\n  return 0\n}\n");
    assert!(
        rust.contains("fn __copper_main() -> i64"),
        "user int main not renamed: {rust}"
    );
    assert!(
        rust.contains("fn main() {") && rust.contains("std::process::exit(__copper_main() as i32)"),
        "exit wrapper missing: {rust}"
    );
    assert_eq!(count_main(&rust), 1, "expected exactly one fn main: {rust}");
}

#[test]
fn void_main_renamed_with_plain_wrapper() {
    let rust = transpile("func main() {\n  println!(\"hi\")\n}\n");
    assert!(
        rust.contains("fn __copper_main()"),
        "user void main not renamed: {rust}"
    );
    assert!(
        rust.contains("__copper_main();"),
        "plain call wrapper missing: {rust}"
    );
    assert!(
        !rust.contains("std::process::exit"),
        "void main should not use exit: {rust}"
    );
    assert_eq!(count_main(&rust), 1, "expected exactly one fn main: {rust}");
}

#[test]
fn top_level_only_emits_single_main() {
    let rust = transpile("mut x = 41\nprintln!(\"x+1 = {}\", x + 1)\n");
    assert_eq!(
        count_main(&rust),
        1,
        "top-level program must produce exactly one fn main: {rust}"
    );
    assert!(
        !rust.contains("__copper_main"),
        "no rename expected without a user main: {rust}"
    );
}

#[test]
fn both_top_level_and_user_main_is_a_compile_error() {
    let rust = transpile("mut y = 5\nprintln!(\"top {}\", y)\nfunc int main() {\n  return 0\n}\n");
    assert!(
        rust.contains("compile_error!"),
        "ambiguous top-level + main should emit compile_error!: {rust}"
    );
}
