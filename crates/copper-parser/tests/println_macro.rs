//! Transpile-level regression tests for bang-less `println(...)` calls.
//!
//! Rust's print macros need a format string as the first argument. Copper
//! lets you write `println(x)` / `println("Hi $name")` without the bang; the
//! transpiler must produce valid Rust:
//!   * a non-string first arg gets a `"{}"` (one per top-level arg) injected,
//!   * an interpolated string renders as macro args (no nested `format!`),
//!   * a call that already supplies a format string is left untouched.

use copper_syntax::tokenizer::tokenizer::Tokenizer;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

#[test]
fn bare_variable_arg_gets_format_string() {
    let rust = transpile("func void run() {\n  x = 5\n  println(x)\n}\n");
    assert!(rust.contains(r#"println!("{}", x)"#), "got: {rust}");
}

#[test]
fn two_bare_args_get_two_placeholders() {
    let rust = transpile("func void run() {\n  a = 1\n  b = 2\n  println(a, b)\n}\n");
    assert!(rust.contains(r#"println!("{} {}", a, b)"#), "got: {rust}");
}

#[test]
fn interpolated_string_renders_as_macro_args_not_nested_format() {
    let rust = transpile("func void run() {\n  name = \"world\"\n  println(\"hi $name\")\n}\n");
    assert!(rust.contains(r#"println!("hi {}", name)"#), "got: {rust}");
    assert!(!rust.contains("println!(format!"), "nested format!: {rust}");
}

#[test]
fn explicit_format_string_is_untouched() {
    let rust = transpile("func void run() {\n  a = 1\n  println(\"already {}\", a)\n}\n");
    assert!(rust.contains(r#"println!("already {}", a)"#), "got: {rust}");
}

#[test]
fn explicit_bang_form_is_untouched() {
    let rust = transpile("func void run() {\n  b = 2\n  println!(\"explicit {}\", b)\n}\n");
    assert!(rust.contains(r#"println!("explicit {}", b)"#), "got: {rust}");
}

#[test]
fn empty_call_stays_empty() {
    let rust = transpile("func void run() {\n  println()\n}\n");
    assert!(rust.contains("println!()"), "got: {rust}");
}
