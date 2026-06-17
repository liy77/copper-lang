//! Multi-line method chains must lower as a single expression, not as
//! several broken statements. The tokenizer suppresses the `;` when a line
//! *ends* with a continuation token; the parser's join pass handles the
//! common leading-`.` form (line ends with `)`/`]`, next line opens `.`).

use copper_syntax::tokenizer::tokenizer::Tokenizer;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

#[test]
fn leading_dot_chain_joins_into_one_expression() {
    let rust = transpile(
        "func i32 run() {\n  \
           result = vec![1, 2, 3]\n    \
             .iter()\n    \
             .map(|x| x * 2)\n    \
             .sum()\n  \
           return result\n\
         }\n",
    );
    assert!(
        rust.contains(".iter().map(|x| x * 2).sum()"),
        "chain not joined: {rust}"
    );
    // No stray statement separators inside the chain.
    // The chain lowers to one line; no broken per-call statements.
    assert!(!rust.contains(".iter();"), "stray ; broke the chain: {rust}");
    assert!(!rust.contains(".sum() -> "), "chain fused with next token: {rust}");
}

#[test]
fn normal_statements_still_terminate() {
    let rust = transpile("func void run() {\n  a = 1\n  b = 2\n}\n");
    assert!(rust.contains("let a = 1;"), "got: {rust}");
    assert!(rust.contains("let b = 2;"), "got: {rust}");
}
