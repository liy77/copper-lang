//! Curated table of Rust prelude items that Copper users routinely write
//! (because Copper transpiles directly to Rust). Drives completion, hover,
//! and signature help for `Some`, `None`, `Ok`, `Err`, `Option`, `Result`,
//! `String`, `Vec`, `HashMap`, the print/format/vec macros, etc.
//!
//! Hand-curated rather than parsed from `std` because:
//! 1. Parsing rustdoc would dwarf the rest of the LSP.
//! 2. The Copper user only needs the prelude surface, not all of std.
//!
//! Add an entry whenever a new Rust API becomes idiomatic in Copper.

use tower_lsp::lsp_types::CompletionItemKind;

pub struct RustItem {
    pub label: &'static str,
    /// Snippet inserted on completion. Use `${N:placeholder}` for tab stops.
    pub insert: &'static str,
    pub kind: CompletionItemKind,
    /// One-line signature shown in completion `detail` and hover preamble.
    pub detail: &'static str,
    /// Markdown body shown on hover.
    pub doc: &'static str,
    /// `Some` if this item is callable (variants, macros, constructors) and
    /// should drive signature help. The slice gives parameter labels in
    /// `name: type` form.
    pub params: Option<&'static [&'static str]>,
}

/// Lookup by exact name. Returns `None` if the name isn't a known prelude
/// item — caller falls back to other sources (cstd, file-local symbols).
pub fn lookup(name: &str) -> Option<&'static RustItem> {
    PRELUDE.iter().find(|i| i.label == name)
}

pub fn all() -> &'static [RustItem] {
    PRELUDE
}

const PRELUDE: &[RustItem] = &[
    // ---- Option / Result variants ----
    RustItem {
        label: "Some",
        insert: "Some(${1:value})",
        kind: CompletionItemKind::ENUM_MEMBER,
        detail: "Some<T>(value: T) -> Option<T>",
        doc: "**`Some(value)`** — `Option` variant carrying a value.\n\n```rust\nlet x: Option<i32> = Some(42);\nif let Some(v) = x { /* ... */ }\n```",
        params: Some(&["value: T"]),
    },
    RustItem {
        label: "None",
        insert: "None",
        kind: CompletionItemKind::ENUM_MEMBER,
        detail: "None: Option<T>",
        doc: "**`None`** — `Option` variant representing absence of a value.\n\n```rust\nlet x: Option<i32> = None;\n```",
        params: None,
    },
    RustItem {
        label: "Ok",
        insert: "Ok(${1:value})",
        kind: CompletionItemKind::ENUM_MEMBER,
        detail: "Ok<T, E>(value: T) -> Result<T, E>",
        doc: "**`Ok(value)`** — successful `Result` variant.\n\n```rust\nfn parse() -> Result<i32, String> {\n    Ok(42)\n}\n```",
        params: Some(&["value: T"]),
    },
    RustItem {
        label: "Err",
        insert: "Err(${1:error})",
        kind: CompletionItemKind::ENUM_MEMBER,
        detail: "Err<T, E>(error: E) -> Result<T, E>",
        doc: "**`Err(error)`** — failure `Result` variant.\n\n```rust\nfn parse(s: &str) -> Result<i32, String> {\n    s.parse().map_err(|e| e.to_string())\n}\n```",
        params: Some(&["error: E"]),
    },
    // ---- Generic types ----
    RustItem {
        label: "Option",
        insert: "Option<${1:T}>",
        kind: CompletionItemKind::ENUM,
        detail: "enum Option<T> { Some(T), None }",
        doc: "**`Option<T>`** — value that may be present (`Some(T)`) or absent (`None`). Use `?` to propagate `None`, `unwrap_or(default)` for fallback.",
        params: None,
    },
    RustItem {
        label: "Result",
        insert: "Result<${1:T}, ${2:E}>",
        kind: CompletionItemKind::ENUM,
        detail: "enum Result<T, E> { Ok(T), Err(E) }",
        doc: "**`Result<T, E>`** — fallible computation: `Ok(T)` on success, `Err(E)` on failure. Use `?` to propagate.",
        params: None,
    },
    RustItem {
        label: "String",
        insert: "String",
        kind: CompletionItemKind::STRUCT,
        detail: "struct String",
        doc: "**`String`** — owned, growable UTF-8 string. Build with `String::new()`, `String::from(\"...\")`, or `format!(...)`.",
        params: None,
    },
    RustItem {
        label: "Vec",
        insert: "Vec<${1:T}>",
        kind: CompletionItemKind::STRUCT,
        detail: "struct Vec<T>",
        doc: "**`Vec<T>`** — heap-allocated, growable array. Build with `Vec::new()`, `vec![a, b, c]`, or `Vec::with_capacity(n)`.",
        params: None,
    },
    RustItem {
        label: "HashMap",
        insert: "HashMap<${1:K}, ${2:V}>",
        kind: CompletionItemKind::STRUCT,
        detail: "struct HashMap<K, V>",
        doc: "**`HashMap<K, V>`** — hash table from `K` to `V`. Requires `use std::collections::HashMap;`.",
        params: None,
    },
    RustItem {
        label: "HashSet",
        insert: "HashSet<${1:T}>",
        kind: CompletionItemKind::STRUCT,
        detail: "struct HashSet<T>",
        doc: "**`HashSet<T>`** — unique-element set backed by a hash table. Requires `use std::collections::HashSet;`.",
        params: None,
    },
    RustItem {
        label: "BTreeMap",
        insert: "BTreeMap<${1:K}, ${2:V}>",
        kind: CompletionItemKind::STRUCT,
        detail: "struct BTreeMap<K, V>",
        doc: "**`BTreeMap<K, V>`** — ordered map (B-tree). Requires `use std::collections::BTreeMap;`.",
        params: None,
    },
    RustItem {
        label: "Box",
        insert: "Box<${1:T}>",
        kind: CompletionItemKind::STRUCT,
        detail: "struct Box<T>",
        doc: "**`Box<T>`** — owning pointer to a heap-allocated `T`. Build with `Box::new(value)`.",
        params: None,
    },
    RustItem {
        label: "Rc",
        insert: "Rc<${1:T}>",
        kind: CompletionItemKind::STRUCT,
        detail: "struct Rc<T>",
        doc: "**`Rc<T>`** — single-threaded reference-counted pointer. Requires `use std::rc::Rc;`.",
        params: None,
    },
    RustItem {
        label: "Arc",
        insert: "Arc<${1:T}>",
        kind: CompletionItemKind::STRUCT,
        detail: "struct Arc<T>",
        doc: "**`Arc<T>`** — atomic reference-counted pointer (thread-safe). Requires `use std::sync::Arc;`.",
        params: None,
    },
    RustItem {
        label: "RefCell",
        insert: "RefCell::new(${1:value})",
        kind: CompletionItemKind::STRUCT,
        detail: "struct RefCell<T>",
        doc: "**`RefCell<T>`** — runtime-checked interior mutability. Requires `use std::cell::RefCell;`.",
        params: Some(&["value: T"]),
    },
    RustItem {
        label: "Iterator",
        insert: "Iterator",
        kind: CompletionItemKind::INTERFACE,
        detail: "trait Iterator",
        doc: "**`Iterator`** — sequence of values. Get one via `iter()`/`into_iter()` then chain `.map`, `.filter`, `.collect`, etc.",
        params: None,
    },
    // ---- Macros ----
    RustItem {
        label: "println!",
        insert: "println!(\"${1:fmt}\"$0)",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! println — print line to stdout",
        doc: "**`println!(fmt, args...)`** — print formatted text + newline to stdout.\n\n```rust\nprintln!(\"x = {}\", x);\n```",
        params: Some(&["fmt: &str", "args: ..."]),
    },
    RustItem {
        label: "print!",
        insert: "print!(\"${1:fmt}\"$0)",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! print — print to stdout (no newline)",
        doc: "**`print!(fmt, args...)`** — print formatted text to stdout without a trailing newline.",
        params: Some(&["fmt: &str", "args: ..."]),
    },
    RustItem {
        label: "eprintln!",
        insert: "eprintln!(\"${1:fmt}\"$0)",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! eprintln — print line to stderr",
        doc: "**`eprintln!(fmt, args...)`** — like `println!` but writes to stderr.",
        params: Some(&["fmt: &str", "args: ..."]),
    },
    RustItem {
        label: "eprint!",
        insert: "eprint!(\"${1:fmt}\"$0)",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! eprint — print to stderr (no newline)",
        doc: "**`eprint!(fmt, args...)`** — like `print!` but writes to stderr.",
        params: Some(&["fmt: &str", "args: ..."]),
    },
    RustItem {
        label: "format!",
        insert: "format!(\"${1:fmt}\"$0)",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! format -> String",
        doc: "**`format!(fmt, args...) -> String`** — build a `String` using the same formatting rules as `println!`.\n\n```rust\nlet s = format!(\"hello {}\", name);\n```",
        params: Some(&["fmt: &str", "args: ..."]),
    },
    RustItem {
        label: "vec!",
        insert: "vec![${1:elements}]",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! vec -> Vec<T>",
        doc: "**`vec![a, b, c]`** or **`vec![elem; n]`** — build a `Vec<T>` literal.\n\n```rust\nlet v = vec![1, 2, 3];\nlet zeros = vec![0; 100];\n```",
        params: None,
    },
    RustItem {
        label: "panic!",
        insert: "panic!(\"${1:msg}\"$0)",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! panic — abort current thread",
        doc: "**`panic!(fmt, args...)`** — abort the current thread with a formatted message.",
        params: Some(&["fmt: &str", "args: ..."]),
    },
    RustItem {
        label: "dbg!",
        insert: "dbg!(${1:expr})",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! dbg — print and forward",
        doc: "**`dbg!(expr)`** — print `file:line expr = value` to stderr **and** return the value. Great for ad-hoc tracing.",
        params: Some(&["expr"]),
    },
    RustItem {
        label: "assert!",
        insert: "assert!(${1:cond})",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! assert — panic if false",
        doc: "**`assert!(cond)`** — panic if `cond` is false.",
        params: Some(&["cond: bool", "msg?: &str"]),
    },
    RustItem {
        label: "assert_eq!",
        insert: "assert_eq!(${1:left}, ${2:right})",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! assert_eq",
        doc: "**`assert_eq!(left, right)`** — panic if `left != right`, showing both.",
        params: Some(&["left", "right"]),
    },
    RustItem {
        label: "assert_ne!",
        insert: "assert_ne!(${1:left}, ${2:right})",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! assert_ne",
        doc: "**`assert_ne!(left, right)`** — panic if `left == right`.",
        params: Some(&["left", "right"]),
    },
    RustItem {
        label: "todo!",
        insert: "todo!()",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! todo — placeholder that panics",
        doc: "**`todo!()`** — typed hole that panics at runtime. Useful for stubbing out branches.",
        params: None,
    },
    RustItem {
        label: "unimplemented!",
        insert: "unimplemented!()",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! unimplemented",
        doc: "**`unimplemented!()`** — same as `todo!()` but signals the omission is intentional and unlikely to ever be filled in.",
        params: None,
    },
    RustItem {
        label: "unreachable!",
        insert: "unreachable!()",
        kind: CompletionItemKind::FUNCTION,
        detail: "macro_rules! unreachable",
        doc: "**`unreachable!()`** — assert that this branch is dead code; panic if reached.",
        params: None,
    },
    // ---- Primitive type aliases (also covered by the keyword cheatsheet,
    // but listing here gets them into completion at all positions) ----
    RustItem {
        label: "i8",
        insert: "i8",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 8-bit signed integer",
        doc: "Signed 8-bit integer. Range: −128..=127.",
        params: None,
    },
    RustItem {
        label: "i16",
        insert: "i16",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 16-bit signed integer",
        doc: "Signed 16-bit integer. Range: −32_768..=32_767.",
        params: None,
    },
    RustItem {
        label: "i32",
        insert: "i32",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 32-bit signed integer",
        doc: "Signed 32-bit integer (Rust default `int`).",
        params: None,
    },
    RustItem {
        label: "i64",
        insert: "i64",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 64-bit signed integer",
        doc: "Signed 64-bit integer (Copper `int`).",
        params: None,
    },
    RustItem {
        label: "u8",
        insert: "u8",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 8-bit unsigned",
        doc: "Unsigned 8-bit integer. Range: 0..=255.",
        params: None,
    },
    RustItem {
        label: "u16",
        insert: "u16",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 16-bit unsigned",
        doc: "Unsigned 16-bit integer.",
        params: None,
    },
    RustItem {
        label: "u32",
        insert: "u32",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 32-bit unsigned",
        doc: "Unsigned 32-bit integer.",
        params: None,
    },
    RustItem {
        label: "u64",
        insert: "u64",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 64-bit unsigned",
        doc: "Unsigned 64-bit integer.",
        params: None,
    },
    RustItem {
        label: "usize",
        insert: "usize",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: pointer-sized unsigned",
        doc: "Pointer-sized unsigned integer (`u32` on 32-bit targets, `u64` on 64-bit). Used for indexing.",
        params: None,
    },
    RustItem {
        label: "isize",
        insert: "isize",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: pointer-sized signed",
        doc: "Pointer-sized signed integer counterpart of `usize`.",
        params: None,
    },
    RustItem {
        label: "f32",
        insert: "f32",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 32-bit float",
        doc: "IEEE-754 single-precision float.",
        params: None,
    },
    RustItem {
        label: "f64",
        insert: "f64",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 64-bit float",
        doc: "IEEE-754 double-precision float (Copper `float`).",
        params: None,
    },
    RustItem {
        label: "char",
        insert: "char",
        kind: CompletionItemKind::TYPE_PARAMETER,
        detail: "primitive: 32-bit Unicode scalar value",
        doc: "Single Unicode scalar value, 4 bytes wide. Literals like `'a'`, `'λ'`.",
        params: None,
    },
];
