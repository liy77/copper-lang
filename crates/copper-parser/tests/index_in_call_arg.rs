//! Regression: indexing an identifier in call-argument position (`g(v[i])`)
//! must stay an index expression, not be rewritten into `g(vvec![i])`. In an
//! argument the identifier is tokenized as `Param`, which the bracket
//! discriminator must treat as a value (so the following `[` is an index).

use copper_syntax::tokenizer::tokenizer::Tokenizer;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

#[test]
fn index_in_argument_stays_index() {
    let rust = transpile(
        "func int g(x: int) { return x }\n\
         mut v = vec![10, 20]\n\
         mut i = 0\n\
         println!(\"{}\", g(v[i]))\n",
    );
    assert!(
        rust.contains("g(v[i])"),
        "index in arg position not preserved: {rust}"
    );
    assert!(
        !rust.contains("vvec!"),
        "index mis-rewritten into a vec literal: {rust}"
    );
}

#[test]
fn list_literal_in_argument_still_lowers() {
    // A bona-fide list literal in argument position must still become `vec!`.
    let rust = transpile(
        "func int s(xs: Vec<int>) { return 0 }\n\
         println!(\"{}\", s([1, 2, 3]))\n",
    );
    assert!(
        rust.contains("s(vec![1, 2, 3])"),
        "list literal in arg position not lowered: {rust}"
    );
}
