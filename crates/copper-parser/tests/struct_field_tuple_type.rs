//! Regression: a struct field whose type is a tuple / nested generic
//! (`Vec<(String, int)>`) must lower correctly — no spurious comma after `(`,
//! commas inside the type are NOT field boundaries, and Copper aliases inside
//! the type (`int` -> `i64`) are converted.

use copper_syntax::tokenizer::tokenizer::Tokenizer;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

#[test]
fn tuple_inside_generic_field_type() {
    let rust = transpile("struct R { fields: Vec<(String, int)> }\n");
    assert!(
        rust.contains("fields: Vec<(String, i64)>"),
        "tuple/nested-generic field type mangled: {rust}"
    );
    assert!(!rust.contains("Vec<(,"), "spurious comma after `(`: {rust}");
}

#[test]
fn simple_generic_field_type_still_works() {
    let rust = transpile("struct S { items: Vec<int> }\n");
    assert!(
        rust.contains("items: Vec<i64>"),
        "simple generic field broke: {rust}"
    );
}

#[test]
fn struct_literal_binding_without_semicolon_compiles() {
    // Bug 4: a `mut r = R { fields: vec![] }` (value ends in `}` containing a
    // `vec![]`) must still get a statement terminator.
    let rust = transpile(
        "struct R { fields: Vec<(String, int)> }\n\
         mut r = R { fields: vec![] }\n\
         println!(\"{}\", r.fields.len())\n",
    );
    assert!(
        rust.contains("let mut r = R { fields: vec![] };"),
        "struct-literal binding missing terminating `;`: {rust}"
    );
}
