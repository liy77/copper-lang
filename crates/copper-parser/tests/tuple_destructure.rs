//! Tuple destructuring, including nested patterns.

use copper_syntax::tokenizer::tokenizer::Tokenizer;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

#[test]
fn flat_destructure_gets_let() {
    let rust = transpile("func void run() {\n  p = (1, 2)\n  (x, y) = p\n}\n");
    assert!(rust.contains("let (x, y) = p;"), "got: {rust}");
}

#[test]
fn nested_destructure_preserves_pattern() {
    let rust = transpile("func void run() {\n  t = (1, (2, 3))\n  (a, (b, c)) = t\n}\n");
    assert!(rust.contains("let (a, (b, c)) = t;"), "got: {rust}");
}

#[test]
fn deeply_nested_destructure() {
    let rust = transpile("func void run() {\n  n = ((1, 2), (3, 4))\n  ((a, b), (c, d)) = n\n}\n");
    assert!(rust.contains("let ((a, b), (c, d)) = n;"), "got: {rust}");
}

#[test]
fn mut_tuple_destructure() {
    let rust = transpile("func void run() {\n  p = (1, 2)\n  mut (x, y) = p\n}\n");
    assert!(rust.contains("let (mut x, mut y) = p;"), "got: {rust}");
}
