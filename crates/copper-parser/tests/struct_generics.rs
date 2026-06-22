//! Generic structs and newline-separated fields.

use copper_syntax::tokenizer::tokenizer::Tokenizer;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

#[test]
fn generic_param_is_preserved() {
    let rust = transpile("struct Pair<T> {\n  first: T\n  second: T\n}\n");
    assert!(rust.contains("struct Pair<T>"), "generic dropped: {rust}");
    assert!(rust.contains("first: T,"), "got: {rust}");
    assert!(rust.contains("second: T,"), "got: {rust}");
    // Fields must not fuse.
    assert!(!rust.contains("Tsecond"), "fields fused: {rust}");
}

#[test]
fn multiple_generic_params() {
    let rust = transpile("struct Map<K, V> {\n  key: K\n  value: V\n}\n");
    assert!(rust.contains("struct Map<K, V>"), "got: {rust}");
    assert!(
        rust.contains("key: K,") && rust.contains("value: V,"),
        "got: {rust}"
    );
}

#[test]
fn newline_separated_fields_dont_fuse() {
    let rust = transpile("struct V3 {\n  x: f64\n  y: f64\n  z: f64\n}\n");
    assert!(rust.contains("x: f64,"), "got: {rust}");
    assert!(rust.contains("y: f64,"), "got: {rust}");
    assert!(rust.contains("z: f64,"), "got: {rust}");
    assert!(
        !rust.contains("f64y") && !rust.contains("f64z"),
        "fused: {rust}"
    );
}

#[test]
fn comma_separated_fields_no_double_comma() {
    let rust = transpile("struct Point { x: int, y: int }\n");
    assert!(
        rust.contains("x: i64,") && rust.contains("y: i64,"),
        "got: {rust}"
    );
    assert!(!rust.contains(",,"), "double comma: {rust}");
}
