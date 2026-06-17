//! The Copper **lexicon** — the single source of truth for the language's
//! lexical facts: type aliases and operator sets.
//!
//! Why this module exists
//! ----------------------
//! These facts were previously duplicated: the type-alias table lived in
//! `copper-parser/src/parser/utils.rs`, and the operator category arrays lived
//! as private `const`s inside the tokenizer. Anything that re-implements a
//! slice of Copper (the LSP, the new typed expression AST in [`crate::expr`],
//! the MUI parser) had to copy them and would silently drift.
//!
//! Now there is ONE definition. The tokenizer, the transpiler's `convert_type`,
//! and the expression AST all read from here, so adding an operator or a type
//! alias updates every consumer at once. A drift test in [`crate::expr`]
//! asserts the expression parser recognises every operator listed here.

// ===========================================================================
// Type aliases  (Copper spelling -> Rust spelling)
// ===========================================================================

/// Copper's built-in type aliases, in the exact order the transpiler applies
/// them. The first match wins (there are no overlaps, so order is cosmetic).
///
/// This is the authoritative table — `copper-parser`'s `convert_type` and the
/// typed AST's [`crate::expr::Type`] resolution both read it.
pub const COPPER_TYPES: &[(&str, &str)] = &[
    ("int", "i64"),
    ("float", "f64"),
    ("uint", "u64"),
    ("int8", "i8"),
    ("int16", "i16"),
    ("int32", "i32"),
    ("int64", "i64"),
    ("uint8", "u8"),
    ("uint16", "u16"),
    ("uint32", "u32"),
    ("uint64", "u64"),
    ("float32", "f32"),
    ("float64", "f64"),
    ("string", "String"),
    ("void", "()"),
    ("json", "JsonValue"),
    ("xml", "XmlValue"),
    ("toml", "TomlValue"),
];

/// Resolve a Copper type spelling to its Rust spelling, honouring the trailing
/// `?` optional suffix (`int?` -> `Option<i64>`). Handles tuple types
/// recursively: `(int, str)` → `(i64, String)`. Pure and allocation-light;
/// shared by the transpiler and the typed AST so they never disagree.
pub fn convert_type(value: &str) -> String {
    let (base, optional) = match value.strip_suffix('?') {
        Some(b) => (b, true),
        None => (value, false),
    };

    let kind = if base.starts_with('(') && base.ends_with(')') {
        // Tuple type — recursively convert each element.
        let inner = &base[1..base.len() - 1];
        let parts = split_at_top_level_commas(inner);
        let converted: Vec<String> = parts
            .iter()
            .map(|p| convert_type(p.trim()))
            .filter(|p| !p.is_empty())
            .collect();
        format!("({})", converted.join(", "))
    } else if base.ends_with('>') && base.contains('<') {
        // Generic type — convert the head alias and recurse into each arg so
        // Copper aliases inside `<...>` are lowered too:
        // `Rc<RefCell<int>>` -> `Rc<RefCell<i64>>`, `Vec<string>` -> `Vec<String>`,
        // `Reflected<int>` -> `Reflected<i64>`.
        let lt = base.find('<').unwrap();
        let head = base[..lt].trim();
        let inner = &base[lt + 1..base.len() - 1];
        let mut head_kind = head.to_string();
        for (copper, rust) in COPPER_TYPES {
            if head == *copper {
                head_kind = (*rust).to_string();
                break;
            }
        }
        let args: Vec<String> = split_at_top_level_commas(inner)
            .iter()
            .map(|p| convert_type(p.trim()))
            .filter(|p| !p.is_empty())
            .collect();
        format!("{}<{}>", head_kind, args.join(", "))
    } else {
        let mut kind = base.to_string();
        for (copper, rust) in COPPER_TYPES {
            if base == *copper {
                kind = (*rust).to_string();
                break;
            }
        }
        kind
    };

    if optional {
        format!("Option<{kind}>")
    } else {
        kind
    }
}

/// Split `s` at commas that are not inside angle brackets, parens, or
/// square brackets. Used to parse tuple-type component lists.
fn split_at_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => {
                depth = depth.saturating_sub(1);
            }
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

// ===========================================================================
// Operators
// ===========================================================================
//
// These four arrays are consumed by the tokenizer's `symbol_token` (in the
// order it checks them: compound -> compare -> arithmetic -> range -> symbol)
// AND are exposed so the expression parser can verify it handles each one.
//
// The iteration *order/strategy* (e.g. range checked with `.rev()` so `..=`
// beats `..`) stays in the tokenizer; only the data lives here.

/// Multi-char operators that fuse into a single token: `::`, the compound
/// assignments, and the logical `&&` / `||`.
pub const COMPOUND_SIGNS: &[&str] = &["::", "-=", "+=", "/=", "*=", "%=", "&=", "^=", "|="];

/// Comparison operators (and the bare `<` / `>` that double as angle brackets).
pub const COMPARE_SIGNS: &[&str] = &["==", "!=", "<=", ">=", ">", "<"];

/// Arithmetic operators.
pub const ARITHMETIC_SIGNS: &[&str] = &["+", "-", "*", "/", "%"];

/// Range operators. Check `..=` before `..` when scanning.
pub const RANGE_SIGNS: &[&str] = &["..", "..="];

/// Single-char symbol operators (bitwise, logical-not, deref/ref, ternary `?`,
/// member `.`, assign `=`, type `:`).
pub const SYMBOL_OPERATORS: &[&str] = &["!", "&", "|", "^", "~", ":", "?", ".", "="];

/// Boolean literals.
pub const BOOL: &[&str] = &["true", "false"];

/// Logical operators that the tokenizer may emit either fused (`&&`) or, in
/// some contexts, as two single tokens (`&` `&`). Parsers must coalesce.
pub const LOGICAL_PAIRS: &[(&str, &str)] = &[("&", "&"), ("|", "|")];

#[cfg(test)]
mod generic_type_tests {
    use super::convert_type;

    #[test]
    fn converts_aliases_inside_generic_args() {
        assert_eq!(convert_type("Vec<int>"), "Vec<i64>");
        assert_eq!(convert_type("Rc<RefCell<int>>"), "Rc<RefCell<i64>>");
        assert_eq!(convert_type("Result<int, string>"), "Result<i64, String>");
        assert_eq!(convert_type("HashMap<string, int>"), "HashMap<String, i64>");
        // user types and Rust-native names pass through unchanged
        assert_eq!(convert_type("Reflected<Obj>"), "Reflected<Obj>");
        assert_eq!(convert_type("Signal<i32>"), "Signal<i32>");
    }
}
