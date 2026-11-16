use copper_syntax::tokenizer::tokenizer::Tokenizer;
use copper_syntax::tokenizer::kind::TokenKind;

fn show(src: &str) {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    for t in tokens.iter().filter(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline | TokenKind::Comment | TokenKind::Eof)) {
        println!("  {:?} = {:?}", t.kind, t.value);
    }
}

#[test]
fn dump_import_forms() {
    println!("=== bare ===");
    show("import { Vec2 } from vec2\n");
    println!("=== relative string ===");
    show("import { Vec2 } from \"./math/vec2.crs\"\n");
    println!("=== crate path ===");
    show("import { Vec2 } from crate::math::vec2\n");
    println!("=== super path ===");
    show("import { World } from super::ecs::world\n");
}
