//! Regression: `from` is contextual — it is the import keyword only inside
//! `import { ... } from <module>`. Everywhere else it must lower as a plain
//! identifier so `From` trait impls, `X::from(...)` paths, `.from(...)` method
//! calls, and a variable literally named `from` all work.

use copper_syntax::tokenizer::tokenizer::Tokenizer;

/// Full transpile: Copper source → Rust source.
fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

#[test]
fn from_trait_impl_keeps_generics_and_method() {
    let src = "struct V { n: int }\n\
        impl From<int> for V {\n\
          func V from(x: int) { return V { n: x } }\n\
        }\n\
        mut a = V::from(5)\n\
        println!(\"{}\", a.n)\n";
    let rust = transpile(src);
    assert!(
        rust.contains("impl From<i64> for V"),
        "impl head wrong (generics/for dropped): {rust}"
    );
    assert!(
        rust.contains("fn from(x: i64) -> V"),
        "from method dropped: {rust}"
    );
    assert!(
        rust.contains("V::from(5)"),
        "path call `V::from` lost its `from`: {rust}"
    );
    // The struct literal field must NOT get a spurious `let`.
    assert!(
        !rust.contains("let n: x"),
        "struct-literal field mangled into a let: {rust}"
    );
}

#[test]
fn from_as_plain_identifier() {
    let src = "mut from = 3\n\
        mut x = from + 1\n\
        println!(\"{}\", x)\n";
    let rust = transpile(src);
    assert!(
        rust.contains("let mut from = 3"),
        "`from` not lowered as identifier: {rust}"
    );
    assert!(
        rust.contains("from + 1"),
        "`from` lost in expression: {rust}"
    );
}

#[test]
fn import_from_still_works() {
    let src = "import { println } from std::io\n\
        println!(\"{}\", 1)\n";
    let rust = transpile(src);
    assert!(
        rust.contains("use io::") || rust.contains("std::io"),
        "import/from no longer parses: {rust}"
    );
}
