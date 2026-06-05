//! Transpile-level regression tests for `impl` method lowering.
//!
//! These guard the bugs fixed when real Copper code (OndaEngine's `vec2.crs` /
//! `aabb.crs`) transpiled to broken Rust: impl-method bodies were rebuilt by a
//! naive token-join that dropped statement boundaries, mis-fired `let` on
//! struct-literal fields, kept `self` by value, and inserted spurious `;` inside
//! multi-line struct literals and operator-continued expressions.

use copper_syntax::tokenizer::tokenizer::Tokenizer;

/// Full transpile: Copper source → Rust source.
fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

#[test]
fn multi_statement_method_body_lowers_each_statement() {
    let rust = transpile(
        "struct V { x: f64 }\n\
         impl V {\n\
             func f64 norm(self) {\n\
                 len = self.x\n\
                 if len == 0.0 {\n\
                     return 0.0\n\
                 }\n\
                 return len\n\
             }\n\
         }\n",
    );
    // The local binding gets `let`, the `if` block survives, statements are
    // terminated — not jammed onto one line.
    assert!(
        rust.contains("let len = self.x;"),
        "missing `let len`:\n{rust}"
    );
    assert!(rust.contains("if len == 0.0"), "missing if block:\n{rust}");
    assert!(
        rust.contains("return len;"),
        "missing terminated return:\n{rust}"
    );
}

#[test]
fn bare_self_receiver_borrows() {
    let rust = transpile(
        "struct V { x: f64 }\n\
         impl V {\n    func f64 get(self) { return self.x }\n}\n",
    );
    assert!(
        rust.contains("fn get(&self)"),
        "bare self should borrow:\n{rust}"
    );
}

#[test]
fn struct_literal_field_is_not_a_typed_declaration() {
    // `x: self.x` inside a struct literal must NOT become `let x: self; .x`.
    let rust = transpile(
        "struct V { x: f64, y: f64 }\n\
         impl V {\n\
             func V add(self, o: V) {\n\
                 return V { x: self.x + o.x, y: self.y + o.y }\n\
             }\n\
         }\n",
    );
    assert!(
        !rust.contains("let x:"),
        "struct field became a let:\n{rust}"
    );
    assert!(
        rust.contains("self.x + o.x"),
        "field value mangled:\n{rust}"
    );
}

#[test]
fn multiline_struct_literal_has_no_stray_semicolon() {
    let rust = transpile(
        "struct P { x: f64, y: f64 }\n\
         impl P {\n\
             func P make(a: f64, b: f64) {\n\
                 return P {\n\
                     x: a,\n\
                     y: b\n\
                 }\n\
             }\n\
         }\n",
    );
    // The newline after the last field must not inject `;` (`y: b;` is invalid).
    assert!(
        !rust.contains("y: b;"),
        "stray `;` in struct literal:\n{rust}"
    );
}

#[test]
fn for_loop_over_identifier_uses_semicolons_not_commas() {
    // `for x in items { stmt() }` must terminate the body statement with `;`.
    // The loop body `{` follows an identifier (`items`), but it's a block, not a
    // struct literal — so it must NOT get `,` separators.
    let rust = transpile(
        "func void run(items: Vec<i32>) {\n\
             for x in items {\n\
                 println!(\"{}\", x)\n\
             }\n\
         }\n",
    );
    assert!(
        !rust.contains("println!(\"{}\", x),") && !rust.contains("x),"),
        "for-body got a comma separator:\n{rust}"
    );
}

#[test]
fn operator_continued_lines_do_not_get_semicolons() {
    // A line ending in `&&` is a continuation, not a statement end.
    let rust = transpile(
        "struct B { v: bool }\n\
         impl B {\n\
             func bool both(self, o: B) {\n\
                 return self.v &&\n\
                        o.v\n\
             }\n\
         }\n",
    );
    assert!(!rust.contains("&&;"), "operator line got a `;`:\n{rust}");
    assert!(
        rust.contains("self.v && o.v") || rust.contains("self.v &&\n"),
        "expr mangled:\n{rust}"
    );
}
