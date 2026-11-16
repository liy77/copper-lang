use copper_syntax::tokenizer::tokenizer::Tokenizer;
use copper_syntax::tokenizer::kind::TokenKind;

fn show(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    tokens.iter()
        .filter(|t| t.kind != TokenKind::Whitespace && t.kind != TokenKind::Newline && t.kind != TokenKind::Comment)
        .map(|t| format!("{:?}='{}'", t.kind, t.value))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[test]
fn division_after_field_access() {
    let src = "func f64 c(self) { return self.size.x / 2.0 }\n";
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    assert!(tokens.iter().any(|t| t.kind == TokenKind::Operator && t.value == "/"), "{}", show(src));
}

#[test]
fn division_in_class_method() {
    let src = r#"
class AABB {
    pos: f64
    size: f64
    f64 center() {
        return self.pos + self.size / 2.0
    }
}
"#;
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    let s = show(src);
    assert!(tokens.iter().any(|t| t.kind == TokenKind::Operator && t.value == "/"), "{s}");
    assert!(!tokens.iter().any(|t| matches!(t.kind, TokenKind::Regex)), "{s}");
}

#[test]
fn division_in_vec2_new_call() {
    let src = r#"
class AABB {
    pos: Vec2
    size: Vec2
    Vec2 center() {
        return Vec2::new(
            self.pos.x + self.size.x / 2.0,
            self.pos.y + self.size.y / 2.0,
        )
    }
}
"#;
    let mut tok = Tokenizer::new(src.to_string());
    let tokens = tok.tokenize();
    let s = show(src);
    assert!(
        tok.errors.is_empty(),
        "unterminated regex on field div: {:?} | {s}",
        tok.errors
    );
    assert!(!tokens.iter().any(|t| matches!(t.kind, TokenKind::Regex)), "regex: {s}");
    assert!(
        tokens.iter().any(|t| t.kind == TokenKind::Operator && t.value == "/"),
        "missing / operator: {s}"
    );
}
