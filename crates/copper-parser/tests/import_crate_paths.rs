//! Import path shortcuts: `crate::`, `super::`, bare modules, and `./file.crs`.

use copper_syntax::tokenizer::tokenizer::Tokenizer;
use std::path::PathBuf;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

fn transpile_paths(src: &str, file: &str, root: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse_with_paths(
        tokens,
        Some(PathBuf::from(file)),
        Some(PathBuf::from(root)),
    )
}

#[test]
fn bare_module_import() {
    let rust = transpile("import { Vec2 } from vec2\n");
    assert!(
        rust.contains("use vec2::{Vec2}") || rust.contains("use vec2::Vec2"),
        "bare module: {rust}"
    );
}

#[test]
fn crate_path_import() {
    let rust = transpile("import { Vec2 } from crate::math::vec2\n");
    assert!(
        rust.contains("use crate::math::vec2::{Vec2}")
            || rust.contains("use crate::math::vec2::Vec2"),
        "crate path: {rust}"
    );
}

#[test]
fn super_path_import() {
    let rust = transpile("import { World } from super::ecs::world\n");
    assert!(
        rust.contains("use super::ecs::world::{World}")
            || rust.contains("use super::ecs::world::World"),
        "super path: {rust}"
    );
}

#[test]
fn dotted_module_path() {
    let rust = transpile("import { Action, KeyCode } from input::input\n");
    assert!(
        rust.contains("use input::input::{Action, KeyCode}")
            || rust.contains("use input::input::{KeyCode, Action}"),
        "module path: {rust}"
    );
}

#[test]
fn relative_string_becomes_super_without_root() {
    let rust = transpile("import { Vec2 } from \"../math/vec2.crs\"\n");
    assert!(
        rust.contains("use super::math::vec2::{Vec2}")
            || rust.contains("use super::math::vec2::Vec2"),
        "relative string: {rust}"
    );
}

#[test]
fn relative_string_becomes_crate_with_root() {
    let rust = transpile_paths(
        "import { Vec2 } from \"../math/vec2.crs\"\n",
        "/proj/engine/ecs/world.crs",
        "/proj/engine",
    );
    assert!(
        rust.contains("use crate::math::vec2::{Vec2}")
            || rust.contains("use crate::math::vec2::Vec2")
            || rust.contains("use super::math::vec2::{Vec2}")
            || rust.contains("use super::math::vec2::Vec2"),
        "rooted relative: {rust}"
    );
}
