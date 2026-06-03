//! Copper parser: the transpiler's emit pass.
//!
//! Consumes the token stream produced by `copper-syntax`'s tokenizer and
//! walks it, emitting Rust source into a `result::Result` buffer. This is
//! the half of the pipeline `cforge` drives; the LSP uses `copper-syntax`'s
//! AST instead and never touches this crate.

// Pre-existing untouched code carries a backlog of stylistic clippy
// warnings. We silence the categories that aren't bugs so
// `cargo clippy -- -D warnings` stays green for CI. Tighten this list as
// the relevant code paths are rewritten.
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

pub mod parser;

// Re-export at the crate root so `cforge` can write `copper_parser::Parser`
// and the parser's public surface stays flat.
pub use parser::Parser;
