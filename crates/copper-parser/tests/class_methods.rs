//! Class lowering: struct + constructor + instance methods, emitted at
//! module level so sibling functions can reference the type.

use copper_syntax::tokenizer::tokenizer::Tokenizer;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

const COUNTER: &str = "class Counter {\n\
  value: i32\n\
  Counter(value: i32) {\n\
    self.value = value\n\
  }\n\
  i32 get() {\n\
    return self.value\n\
  }\n\
  i32 plus(n: i32) {\n\
    return self.value + n\n\
  }\n\
}\n";

#[test]
fn constructor_keeps_primitive_param() {
    let rust = transpile(COUNTER);
    assert!(
        rust.contains("pub fn new(value: i32) -> Self"),
        "ctor param dropped: {rust}"
    );
}

#[test]
fn methods_emit_with_self_and_params() {
    let rust = transpile(COUNTER);
    assert!(
        rust.contains("pub fn get(&self) -> i32"),
        "get missing: {rust}"
    );
    assert!(
        rust.contains("pub fn plus(&self, n: i32) -> i32"),
        "plus missing/params dropped: {rust}"
    );
}

#[test]
fn selfless_method_gets_implicit_self() {
    // A method that uses `self.x` in the body but doesn't declare `self`.
    let rust = transpile(
        "class G {\n  name: String\n  G(name: String) {\n    self.name = name\n  }\n  String hi() {\n    return \"x\"\n  }\n}\n",
    );
    assert!(
        rust.contains("pub fn hi(&self) -> String"),
        "selfless method dropped: {rust}"
    );
}

#[test]
fn class_emitted_at_module_level() {
    let rust = transpile(COUNTER);
    // struct/impl come before any `fn main`, not nested inside it.
    let struct_pos = rust.find("struct Counter").expect("struct");
    let main_pos = rust.find("fn main").unwrap_or(usize::MAX);
    assert!(struct_pos < main_pos, "class not at module level: {rust}");
    // No stray `;` terminating the impl block.
    assert!(
        !rust.contains("}\n};") && !rust.contains("    };"),
        "stray ; after impl: {rust}"
    );
}
