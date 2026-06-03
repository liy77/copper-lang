//! Copper language syntax: tokenizer + AST.
//!
//! Shared between `cforge` (the transpiler) and `copper-lsp` (the language
//! server). The transpiler ignores `ast` and walks tokens directly; the LSP
//! ignores the transpiler's emit pass and walks the AST.

#![allow(
    clippy::borrow_interior_mutable_const,
    clippy::declare_interior_mutable_const,
    clippy::doc_lazy_continuation,
    clippy::if_same_then_else,
    clippy::manual_strip,
    clippy::module_inception,
    clippy::needless_late_init,
    clippy::redundant_locals,
    clippy::to_string_trait_impl,
    clippy::unnecessary_unwrap,
    clippy::while_let_loop
)]

pub mod ast;
pub mod expr;
pub mod lexicon;
pub mod program;
pub mod tokenizer;
pub mod utils;

// Re-exports so existing tokenizer code that wrote `crate::ConsumedTrait`
// (originally from the cforge crate root) keeps compiling.
pub use utils::*;
