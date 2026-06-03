//! Whole-program AST for Copper — top-level [`Item`]s with **real bodies**.
//!
//! Where the other modules sit
//! ---------------------------
//! * [`crate::ast`] is a span-based, declaration-level tree used by the LSP for
//!   document symbols; bodies are opaque spans.
//! * [`crate::expr`] is the typed expression/statement layer ([`Expr`],
//!   [`Stmt`], [`Block`]) — the surface MUI bindings and the JIT need.
//! * **This module** ties them together: it parses a `.crs` file into a
//!   [`Program`] of [`Item`]s (`func` / `struct` / `class` / `impl` / `trait`
//!   / `enum` / `import`), each carrying a fully-parsed [`Block`] body. It is
//!   the faithful program AST a backend (Cranelift, or a re-emitter) can walk.
//!
//! It reuses the [`expr`](crate::expr) parser for bodies via
//! [`expr::parse_stmts`], so item bodies and embedded expressions share one
//! grammar and cannot drift from each other.

use crate::ast::Span;
use crate::expr::{self, Block, Type};
use crate::tokenizer::kind::TokenKind;
use crate::tokenizer::tokenizer::Tokenizer;
use crate::tokenizer::tokens::Token;

/// A parsed Copper source file.
#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>,
    pub errors: Vec<ProgramError>,
}

#[derive(Debug, Clone)]
pub struct ProgramError {
    pub span: Span,
    pub message: String,
}

/// A function / method parameter: `name: Type`. A `self` / `&self` / `&mut
/// self` receiver is captured as a param named `self` with no type.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Option<Type>,
    pub span: Span,
}

/// A `struct` / `class` field: `name: Type`.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Option<Type>,
    pub span: Span,
}

/// A member inside a `class` body.
#[derive(Debug, Clone)]
pub enum ClassMember {
    Field(Field),
    /// `ClassName(params) { body }` — the constructor.
    Constructor {
        params: Vec<Param>,
        body: Block,
        span: Span,
    },
    /// `RetType name(params) { body }`.
    Method {
        name: String,
        params: Vec<Param>,
        return_type: Option<Type>,
        body: Block,
        span: Span,
    },
}

/// One import binding form.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportKind {
    /// `import name from path` → bring `path` in under alias `name`.
    Alias(String),
    /// `import { a, b } from path`.
    Items(Vec<String>),
    /// `import * from path`.
    Glob,
}

/// A top-level item.
#[derive(Debug, Clone)]
pub enum Item {
    /// `func RetType name(params) { body }` (or `unsafe func ...`).
    Function {
        name: String,
        params: Vec<Param>,
        return_type: Option<Type>,
        is_unsafe: bool,
        body: Block,
        span: Span,
    },
    /// `struct Name { fields }`. `generics` keeps the raw `<...>` source.
    Struct {
        name: String,
        generics: Option<String>,
        fields: Vec<Field>,
        span: Span,
    },
    /// `class Name { members }` (Copper-specific; lowers to struct + impl).
    Class {
        name: String,
        members: Vec<ClassMember>,
        span: Span,
    },
    /// `impl Target { items }` / `impl Trait for Target { items }`.
    Impl {
        trait_name: Option<String>,
        target: String,
        items: Vec<Item>,
        span: Span,
    },
    /// `trait Name { items }`.
    Trait {
        name: String,
        items: Vec<Item>,
        span: Span,
    },
    /// `enum Name { Variant, Variant(T), ... }` — variants kept as raw source.
    Enum {
        name: String,
        variants: Vec<String>,
        span: Span,
    },
    /// `import ... from path`.
    Import {
        kind: ImportKind,
        path: String,
        span: Span,
    },
    /// A top-level statement (Copper allows free-standing statements, which the
    /// transpiler wraps into `fn main`). Carries the expr-layer statement.
    Stmt(expr::Stmt),
}

/// Parse a whole Copper source file into a [`Program`]. Never panics.
pub fn parse_program(source: &str) -> Program {
    let toks: Vec<Token> = Tokenizer::new(source.to_string())
        .tokenize()
        .into_iter()
        .filter(|t| {
            let trivia = matches!(
                t.kind,
                TokenKind::Whitespace | TokenKind::Comment | TokenKind::Eof
            );
            let empty_nl = t.kind != TokenKind::Newline && t.value.trim().is_empty();
            !(trivia || empty_nl)
        })
        .collect();
    let mut p = ItemParser {
        toks,
        pos: 0,
        errors: Vec::new(),
    };
    let items = p.parse_items(usize::MAX);
    Program {
        items,
        errors: p.errors,
    }
}

struct ItemParser {
    toks: Vec<Token>,
    pos: usize,
    errors: Vec<ProgramError>,
}

impl ItemParser {
    // --- cursor helpers ---------------------------------------------------
    fn peek_val(&self) -> Option<&str> {
        self.toks.get(self.pos).map(|t| t.value.as_str())
    }
    fn peek_kind(&self) -> Option<TokenKind> {
        self.toks.get(self.pos).map(|t| t.kind)
    }
    fn bump(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, v: &str) -> bool {
        if self.peek_val() == Some(v) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn at(&self, idx: usize) -> Option<&Token> {
        self.toks.get(idx)
    }
    fn cur_span(&self) -> Span {
        self.toks
            .get(self.pos)
            .map(span_of)
            .or_else(|| self.toks.last().map(span_of))
            .unwrap_or_default()
    }
    fn prev_end_span(&self) -> Span {
        self.toks
            .get(self.pos.saturating_sub(1))
            .map(span_of)
            .unwrap_or_default()
    }
    fn err(&mut self, message: impl Into<String>) {
        let span = self.cur_span();
        self.errors.push(ProgramError {
            span,
            message: message.into(),
        });
    }
    fn is_ident(&self) -> bool {
        matches!(
            self.peek_kind(),
            Some(TokenKind::Identifier)
                | Some(TokenKind::Keyword)
                | Some(TokenKind::Type)
                | Some(TokenKind::Param)
                | Some(TokenKind::ParamType)
                | Some(TokenKind::ReturnType)
        )
    }

    /// Parse items until `}` or end-of-range (`limit` is an absolute token
    /// index ceiling, `usize::MAX` for the whole file).
    fn parse_items(&mut self, limit: usize) -> Vec<Item> {
        let mut items = Vec::new();
        while self.pos < self.toks.len() && self.pos < limit {
            match self.peek_val() {
                Some("}") => break,
                Some(";") | Some(",") => {
                    self.bump();
                    continue;
                }
                _ => {}
            }
            // Newline tokens are statement separators between items.
            if self.peek_kind() == Some(TokenKind::Newline) {
                self.bump();
                continue;
            }
            let before = self.pos;
            self.parse_item_into(&mut items);
            if self.pos == before {
                self.bump(); // guarantee progress
            }
        }
        items
    }

    fn parse_item_into(&mut self, out: &mut Vec<Item>) {
        let item = match self.peek_val() {
            Some("import") | Some("use") => self.parse_import(),
            Some("func") => self.parse_function(false),
            Some("unsafe") if self.at(self.pos + 1).map(|t| t.value.as_str()) == Some("func") => {
                self.bump(); // unsafe
                self.parse_function(true)
            }
            Some("struct") => self.parse_struct(),
            Some("class") => self.parse_class(),
            Some("impl") => self.parse_impl(),
            Some("trait") => self.parse_trait(),
            Some("enum") => self.parse_enum(),
            // Anything else is one or more free-standing statements (the
            // transpiler wraps these into `fn main`). These can yield several
            // `Item::Stmt`s from a single slice, so push directly.
            _ => {
                self.parse_free_stmts(out);
                return;
            }
        };
        if let Some(item) = item {
            out.push(item);
        }
    }

    // --- import -----------------------------------------------------------
    fn parse_import(&mut self) -> Option<Item> {
        let start = self.cur_span();
        self.bump(); // import / use
        let kind;
        if self.eat("{") {
            // Inside an import list the tokenizer tags names as `ModuleVar`,
            // not `Identifier` — collect any token that isn't a delimiter.
            let mut names = Vec::new();
            while let Some(v) = self.peek_val() {
                if v == "}" {
                    break;
                }
                if v == "," {
                    self.bump();
                    continue;
                }
                names.push(self.bump().unwrap().value);
            }
            self.eat("}");
            kind = ImportKind::Items(names);
        } else if self.peek_val() == Some("*") {
            self.bump();
            kind = ImportKind::Glob;
        } else if self.peek_val() != Some("from") {
            // Alias form `import name from path` — the name is a single token
            // (Identifier or ModuleVar).
            kind = ImportKind::Alias(self.bump().map(|t| t.value).unwrap_or_default());
        } else {
            kind = ImportKind::Glob;
        }
        // `from <path>` — path may be dotted (`std.io`) or `::`-pathed.
        let mut path = String::new();
        if self.eat("from") {
            while let Some(t) = self.toks.get(self.pos) {
                if t.kind == TokenKind::Newline || matches!(t.value.as_str(), ";") {
                    break;
                }
                path.push_str(&t.value);
                self.pos += 1;
            }
        }
        let span = Span::merge(start, self.prev_end_span());
        Some(Item::Import { kind, path, span })
    }

    // --- function ---------------------------------------------------------
    fn parse_function(&mut self, is_unsafe: bool) -> Option<Item> {
        let start = self.cur_span();
        self.bump(); // func
                     // Return type: the tokenizer tags it `ReturnType` (possibly followed by
                     // `<...>` generics). `func name(...)` with no return type is allowed.
        let mut return_type = None;
        if self.peek_kind() == Some(TokenKind::ReturnType) {
            let mut buf = self.bump().unwrap().value;
            if self.peek_val() == Some("<") {
                buf.push_str(&self.collect_balanced_angles());
            }
            return_type = Some(expr::type_from_source(&buf));
        }
        let name = self.expect_ident("function name");
        let params = self.parse_params();
        let body = self.parse_braced_block();
        let span = Span::merge(start, self.prev_end_span());
        Some(Item::Function {
            name,
            params,
            return_type,
            is_unsafe,
            body,
            span,
        })
    }

    /// Parse `( ... )` parameters: `name: Type` comma-separated. Tolerates
    /// `self` / `&self` / `&mut self` receivers.
    fn parse_params(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        if !self.eat("(") {
            return params;
        }
        while let Some(v) = self.peek_val() {
            if v == ")" {
                break;
            }
            // Skip receiver markers `&` / `&mut` before `self`.
            if v == "&" {
                self.bump();
                self.eat("mut");
            }
            if !self.is_ident() {
                self.bump();
                continue;
            }
            let p_start = self.cur_span();
            let name = self.bump().unwrap().value;
            let ty = if self.eat(":") {
                Some(self.parse_type_until(&[",", ")"]))
            } else {
                None
            };
            params.push(Param {
                name,
                ty,
                span: Span::merge(p_start, self.prev_end_span()),
            });
            self.eat(",");
        }
        self.eat(")");
        params
    }

    // --- struct -----------------------------------------------------------
    fn parse_struct(&mut self) -> Option<Item> {
        let start = self.cur_span();
        self.bump(); // struct
        let name = self.expect_ident("struct name");
        let generics = if self.peek_val() == Some("<") {
            Some(self.collect_balanced_angles())
        } else {
            None
        };
        let fields = self.parse_fields();
        let span = Span::merge(start, self.prev_end_span());
        Some(Item::Struct {
            name,
            generics,
            fields,
            span,
        })
    }

    /// Parse a `{ name: Type, ... }` field list.
    fn parse_fields(&mut self) -> Vec<Field> {
        let mut fields = Vec::new();
        if !self.eat("{") {
            return fields;
        }
        while let Some(v) = self.peek_val() {
            if v == "}" {
                break;
            }
            if v == "," || v == ";" || self.peek_kind() == Some(TokenKind::Newline) {
                self.bump();
                continue;
            }
            if !self.is_ident() {
                self.bump();
                continue;
            }
            let f_start = self.cur_span();
            let name = self.bump().unwrap().value;
            let ty = if self.eat(":") {
                Some(self.parse_type_until(&[",", ";", "}"]))
            } else {
                None
            };
            fields.push(Field {
                name,
                ty,
                span: Span::merge(f_start, self.prev_end_span()),
            });
            self.eat(",");
        }
        self.eat("}");
        fields
    }

    // --- class ------------------------------------------------------------
    fn parse_class(&mut self) -> Option<Item> {
        let start = self.cur_span();
        self.bump(); // class
        let name = self.expect_ident("class name");
        let mut members = Vec::new();
        if self.eat("{") {
            while let Some(v) = self.peek_val() {
                if v == "}" {
                    break;
                }
                if v == "," || v == ";" || self.peek_kind() == Some(TokenKind::Newline) {
                    self.bump();
                    continue;
                }
                let before = self.pos;
                if let Some(m) = self.parse_class_member(&name) {
                    members.push(m);
                }
                if self.pos == before {
                    self.bump();
                }
            }
            self.eat("}");
        }
        let span = Span::merge(start, self.prev_end_span());
        Some(Item::Class {
            name,
            members,
            span,
        })
    }

    /// One class member. Disambiguates:
    ///   * `Name (`          → constructor (same name as the class)
    ///   * `RetType Name (`  → method
    ///   * `name :`          → field
    fn parse_class_member(&mut self, class_name: &str) -> Option<ClassMember> {
        let start = self.cur_span();
        // Constructor: `ClassName(`.
        if self.peek_val() == Some(class_name)
            && self.at(self.pos + 1).map(|t| t.value.as_str()) == Some("(")
        {
            self.bump(); // class name
            let params = self.parse_params();
            let body = self.parse_braced_block();
            return Some(ClassMember::Constructor {
                params,
                body,
                span: Span::merge(start, self.prev_end_span()),
            });
        }
        // Method: a type-like token (RetType/ident), then `name (`.
        let n1 = self.at(self.pos + 1).map(|t| t.value.as_str());
        let n2 = self.at(self.pos + 2).map(|t| t.value.as_str());
        let looks_method = self.is_ident()
            && self.at(self.pos + 1).map(is_ident_tok).unwrap_or(false)
            && n2 == Some("(");
        if looks_method {
            let return_type = Some(expr::type_from_source(&self.bump().unwrap().value));
            let name = self.bump().unwrap().value;
            let params = self.parse_params();
            let body = self.parse_braced_block();
            return Some(ClassMember::Method {
                name,
                params,
                return_type,
                body,
                span: Span::merge(start, self.prev_end_span()),
            });
        }
        // Field: `name : Type`.
        if self.is_ident() && n1 == Some(":") {
            let name = self.bump().unwrap().value;
            self.eat(":");
            let ty = Some(self.parse_type_until(&[",", ";", "}"]));
            return Some(ClassMember::Field(Field {
                name,
                ty,
                span: Span::merge(start, self.prev_end_span()),
            }));
        }
        None
    }

    // --- impl / trait -----------------------------------------------------
    fn parse_impl(&mut self) -> Option<Item> {
        let start = self.cur_span();
        self.bump(); // impl
                     // Optional generics on the impl itself: `impl<T> ...` — skip them.
        if self.peek_val() == Some("<") {
            self.collect_balanced_angles();
        }
        let first = self.expect_ident("impl target");
        // `impl Trait for Target`.
        let (trait_name, target) = if self.eat("for") {
            (Some(first), self.expect_ident("impl target after `for`"))
        } else {
            (None, first)
        };
        // Skip any generics on the target.
        if self.peek_val() == Some("<") {
            self.collect_balanced_angles();
        }
        let items = self.parse_braced_items();
        let span = Span::merge(start, self.prev_end_span());
        Some(Item::Impl {
            trait_name,
            target,
            items,
            span,
        })
    }

    fn parse_trait(&mut self) -> Option<Item> {
        let start = self.cur_span();
        self.bump(); // trait
        let name = self.expect_ident("trait name");
        if self.peek_val() == Some("<") {
            self.collect_balanced_angles();
        }
        let items = self.parse_braced_items();
        let span = Span::merge(start, self.prev_end_span());
        Some(Item::Trait { name, items, span })
    }

    // --- enum -------------------------------------------------------------
    fn parse_enum(&mut self) -> Option<Item> {
        let start = self.cur_span();
        self.bump(); // enum
        let name = self.expect_ident("enum name");
        if self.peek_val() == Some("<") {
            self.collect_balanced_angles();
        }
        let mut variants = Vec::new();
        if self.eat("{") {
            while let Some(v) = self.peek_val() {
                if v == "}" {
                    break;
                }
                if v == "," || self.peek_kind() == Some(TokenKind::Newline) {
                    self.bump();
                    continue;
                }
                if self.is_ident() {
                    let mut variant = self.bump().unwrap().value;
                    // Tuple/struct variant payload: keep it as raw source.
                    if matches!(self.peek_val(), Some("(") | Some("{")) {
                        let open = self.peek_val().unwrap().to_string();
                        let close = if open == "(" { ")" } else { "}" };
                        variant.push_str(&self.collect_balanced(&open, close));
                    }
                    variants.push(variant);
                } else {
                    self.bump();
                }
                self.eat(",");
            }
            self.eat("}");
        }
        let span = Span::merge(start, self.prev_end_span());
        Some(Item::Enum {
            name,
            variants,
            span,
        })
    }

    // --- free statement ---------------------------------------------------
    /// Parse free-standing statements (outside any item) by handing the token
    /// slice up to the next statement boundary to the expr layer — using the
    /// real tokens (no source reconstruction), so adjacency-sensitive forms
    /// like `x++` survive. Pushes every resulting statement as an `Item::Stmt`.
    fn parse_free_stmts(&mut self, out: &mut Vec<Item>) {
        let start = self.pos;
        let end = self.stmt_slice_end(start);
        if end <= start {
            self.bump();
            return;
        }
        let (block, _errs) = expr::parse_stmts_tokens(&self.toks[start..end]);
        self.pos = end;
        for stmt in block.stmts {
            out.push(Item::Stmt(stmt));
        }
        if let Some(e) = block.tail {
            out.push(Item::Stmt(expr::Stmt::Expr(*e)));
        }
    }

    /// Find the token index that ends a free statement starting at `from`:
    /// the matching close of any opened block, otherwise the next top-level
    /// Newline. Keeps multi-line constructs (if/while/match/closures) whole.
    fn stmt_slice_end(&self, from: usize) -> usize {
        let mut i = from;
        let mut depth = 0i32;
        while i < self.toks.len() {
            match self.toks[i].value.as_str() {
                "{" | "(" | "[" => depth += 1,
                "}" | ")" | "]" => {
                    if depth == 0 {
                        return i; // a closing brace of an enclosing item
                    }
                    depth -= 1;
                }
                _ => {}
            }
            if depth == 0 && self.toks[i].kind == TokenKind::Newline {
                return i + 1; // include the separator
            }
            i += 1;
        }
        self.toks.len()
    }

    // --- shared body helpers ---------------------------------------------
    /// Parse a `{ ... }` body as a [`Block`] by handing the balanced token
    /// range straight to the expr-layer statement parser (no source
    /// reconstruction).
    fn parse_braced_block(&mut self) -> Block {
        let open = self.cur_span();
        if self.peek_val() != Some("{") {
            return empty_block(open);
        }
        let close = self.matching_brace(self.pos + 1);
        let (mut block, _errs) = expr::parse_stmts_tokens(&self.toks[self.pos + 1..close]);
        block.span = Span::merge(open, self.at(close).map(span_of).unwrap_or(open));
        self.pos = close + 1; // past the `}`
        block
    }

    /// Parse a `{ ... }` body as a list of items (impl / trait bodies).
    fn parse_braced_items(&mut self) -> Vec<Item> {
        if !self.eat("{") {
            return Vec::new();
        }
        let close = self.matching_brace(self.pos);
        let items = self.parse_items(close);
        self.pos = close.max(self.pos);
        self.eat("}");
        items
    }

    /// Index of the `}` matching the `{` whose body starts at `from`.
    fn matching_brace(&self, from: usize) -> usize {
        let mut depth = 1i32;
        let mut i = from;
        while i < self.toks.len() {
            match self.toks[i].value.as_str() {
                "{" => depth += 1,
                "}" => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        self.toks.len()
    }

    /// Collect a balanced `<...>` run (generics) as raw text, including the
    /// angle brackets.
    fn collect_balanced_angles(&mut self) -> String {
        self.collect_balanced("<", ">")
    }

    fn collect_balanced(&mut self, open: &str, close: &str) -> String {
        let mut buf = String::new();
        if self.peek_val() != Some(open) {
            return buf;
        }
        let mut depth = 0i32;
        while let Some(t) = self.toks.get(self.pos) {
            let v = t.value.clone();
            if v == open {
                depth += 1;
            } else if v == close {
                depth -= 1;
            }
            buf.push_str(&v);
            self.pos += 1;
            if depth <= 0 {
                break;
            }
        }
        buf
    }

    /// Collect a type from tokens until one of `stops` (at depth 0) is hit, then
    /// resolve it through the shared lexicon.
    fn parse_type_until(&mut self, stops: &[&str]) -> Type {
        let mut buf = String::new();
        let mut depth = 0i32;
        while let Some(t) = self.toks.get(self.pos) {
            let v = t.value.as_str();
            if depth == 0 && (stops.contains(&v) || t.kind == TokenKind::Newline) {
                break;
            }
            match v {
                "<" | "(" | "[" => depth += 1,
                ">" | ")" | "]" => depth -= 1,
                _ => {}
            }
            buf.push_str(v);
            self.pos += 1;
        }
        expr::type_from_source(buf.trim())
    }

    fn expect_ident(&mut self, what: &str) -> String {
        if self.is_ident() {
            return self.bump().unwrap().value;
        }
        self.err(format!("expected {what}"));
        String::new()
    }
}

fn is_ident_tok(t: &Token) -> bool {
    matches!(
        t.kind,
        TokenKind::Identifier
            | TokenKind::Keyword
            | TokenKind::Type
            | TokenKind::Param
            | TokenKind::ParamType
            | TokenKind::ReturnType
    )
}

fn empty_block(span: Span) -> Block {
    Block {
        stmts: Vec::new(),
        tail: None,
        span,
    }
}

fn span_of(t: &Token) -> Span {
    let (s, e) = t
        .location_data
        .as_ref()
        .map(|l| (l.range.0 as u32, l.range.1 as u32))
        .unwrap_or((0, t.length as u32));
    Span::new(s, e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::Stmt;

    #[test]
    fn parses_function_with_body() {
        let p = parse_program("func i32 add(a: i32, b: i32) {\n  return a + b\n}\n");
        assert!(p.errors.is_empty(), "errors: {:?}", p.errors);
        assert_eq!(p.items.len(), 1, "items: {:?}", p.items);
        let Item::Function {
            name,
            params,
            return_type,
            body,
            ..
        } = &p.items[0]
        else {
            panic!("expected function, got {:?}", p.items[0]);
        };
        assert_eq!(name, "add");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "a");
        assert_eq!(return_type.as_ref(), Some(&Type::Int));
        // Body has a real `return a + b` statement.
        assert!(
            matches!(
                body.stmts.first(),
                Some(Stmt::Return { value: Some(_), .. })
            ),
            "body stmts: {:?}",
            body.stmts
        );
    }

    #[test]
    fn parses_struct_fields() {
        let p = parse_program("struct Point { x: int, y: int }\n");
        assert_eq!(p.items.len(), 1);
        let Item::Struct { name, fields, .. } = &p.items[0] else {
            panic!("expected struct");
        };
        assert_eq!(name, "Point");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "x");
        assert_eq!(fields[0].ty.as_ref(), Some(&Type::Int));
    }

    #[test]
    fn parses_class_with_ctor_and_method() {
        let src = "class Greeter {\n  name: string\n  Greeter(name: string) {\n    self.name = name\n  }\n  void hello(self) {\n    println!(\"hi\")\n  }\n}\n";
        let p = parse_program(src);
        let Item::Class { name, members, .. } = &p.items[0] else {
            panic!("expected class, got {:?}", p.items.first());
        };
        assert_eq!(name, "Greeter");
        assert_eq!(members.len(), 3, "members: {:?}", members);
        assert!(matches!(members[0], ClassMember::Field(_)));
        assert!(matches!(members[1], ClassMember::Constructor { .. }));
        assert!(matches!(members[2], ClassMember::Method { .. }));
    }

    #[test]
    fn parses_imports() {
        let p = parse_program(
            "import { input, exit } from cstd\nimport * from std.io\nimport fs from std\n",
        );
        assert_eq!(p.items.len(), 3, "items: {:?}", p.items);
        assert!(
            matches!(&p.items[0], Item::Import { kind: ImportKind::Items(v), .. } if v.len() == 2)
        );
        assert!(matches!(
            &p.items[1],
            Item::Import {
                kind: ImportKind::Glob,
                ..
            }
        ));
        assert!(
            matches!(&p.items[2], Item::Import { kind: ImportKind::Alias(a), .. } if a == "fs")
        );
    }

    #[test]
    fn parses_impl_block() {
        let p = parse_program(
            "impl Point {\n  func int sum(self) {\n    return self.x + self.y\n  }\n}\n",
        );
        let Item::Impl {
            target,
            items,
            trait_name,
            ..
        } = &p.items[0]
        else {
            panic!("expected impl, got {:?}", p.items.first());
        };
        assert_eq!(target, "Point");
        assert!(trait_name.is_none());
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Item::Function { .. }));
    }

    #[test]
    fn parses_control_flow_in_body() {
        let src = "func void run() {\n  mut n = 5\n  while n > 0 {\n    n--\n  }\n  for i in 0..3 {\n    println!(\"{}\", i)\n  }\n}\n";
        let p = parse_program(src);
        let Item::Function { body, .. } = &p.items[0] else {
            panic!("expected function");
        };
        assert!(
            body.stmts.iter().any(|s| matches!(s, Stmt::While { .. })),
            "stmts: {:?}",
            body.stmts
        );
        assert!(body.stmts.iter().any(|s| matches!(s, Stmt::For { .. })));
    }

    #[test]
    fn parses_free_statements() {
        let p = parse_program("mut x = 1\nx++\nprintln!(\"{}\", x)\n");
        // Three top-level statements wrapped as items.
        let stmt_items = p
            .items
            .iter()
            .filter(|i| matches!(i, Item::Stmt(_)))
            .count();
        assert!(stmt_items >= 3, "items: {:?}", p.items);
    }
}
