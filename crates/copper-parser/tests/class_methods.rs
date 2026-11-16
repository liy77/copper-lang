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

#[test]
fn static_method_has_no_self() {
    let rust = transpile(
        "class Vec2 {\n  x: f64\n  y: f64\n  Vec2(x: f64, y: f64) {\n    self.x = x\n    self.y = y\n  }\n  static Vec2 zero() {\n    return Vec2 { x: 0.0, y: 0.0 }\n  }\n}\n",
    );
    assert!(
        rust.contains("pub fn zero() -> Vec2") || rust.contains("pub fn zero() -> Self"),
        "static zero should have no &self: {rust}"
    );
    assert!(
        !rust.contains("fn zero(&self)"),
        "static zero must not take &self: {rust}"
    );
}

#[test]
fn mut_self_emits_mut_ref() {
    let rust = transpile(
        "class Bag {\n  n: i32\n  Bag(n: i32) {\n    self.n = n\n  }\n  void bump(mut self) {\n    self.n = self.n + 1\n  }\n}\n",
    );
    assert!(
        rust.contains("pub fn bump(&mut self)"),
        "mut self → &mut self: {rust}"
    );
}

#[test]
fn reassignment_inside_method_no_shadow() {
    let rust = transpile(
        "class C {\n  v: i32\n  C(v: i32) {\n    self.v = v\n  }\n  i32 step() {\n    mut x = 0\n    x = self.v\n    return x\n  }\n}\n",
    );
    assert!(
        rust.contains("let mut x") && rust.contains("x = self.v"),
        "reassignment must not inject a second let: {rust}"
    );
    assert!(
        !rust.contains("let x = self.v"),
        "shadowed let on reassignment: {rust}"
    );
}
