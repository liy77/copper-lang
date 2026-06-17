//! Postfix `++` / `--` must terminate their statement. The tokenizer emits
//! them as two single-char `+`/`-` operators; without recognising the pair,
//! the trailing `+` looked like a binary continuation and the `;` was
//! dropped, fusing the increment with the next line (`count+= 1\n if ...`).

use copper_syntax::tokenizer::tokenizer::Tokenizer;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

#[test]
fn increment_before_if_is_terminated() {
    let rust = transpile(
        "func void run() {\n  mut count = 0\n  loop {\n    count++\n    if count == 3 {\n      break\n    }\n  }\n}\n",
    );
    assert!(rust.contains("count += 1;"), "missing terminator: {rust}");
    assert!(!rust.contains("count+= 1\n if") && !rust.contains("count += 1 if"), "fused: {rust}");
}

#[test]
fn decrement_before_if_is_terminated() {
    let rust = transpile(
        "func void run() {\n  mut n = 5\n  while n > 0 {\n    n--\n    if n == 2 {\n      continue\n    }\n  }\n}\n",
    );
    assert!(rust.contains("n -= 1;"), "missing terminator: {rust}");
}

#[test]
fn binary_plus_continuation_still_joins() {
    // A line genuinely ending in `+` (binary) stays a continuation.
    let rust = transpile("func i32 run() {\n  x = 1 +\n    2\n  return x\n}\n");
    assert!(rust.contains("1 +") && rust.contains("2"), "got: {rust}");
    assert!(!rust.contains("1 +;"), "binary + wrongly terminated: {rust}");
}
