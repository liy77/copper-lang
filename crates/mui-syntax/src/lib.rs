//! MUI parser — turns `.mui` / `.crm` source into the component [`ast`].
//!
//! This first increment parses the **structural layer**: `view` declarations,
//! their parameters, and the nested element/children tree (plus `let` / `if` /
//! `for` / `match` / `effect` node shapes). Element arguments and prop
//! expressions are recognised structurally; lowering them into
//! [`copper_syntax::expr`] trees is the next increment (the dependency is
//! already wired — the AST carries `Expr`/`Block` from copper-syntax).
//!
//! Lexing reuses Copper's own tokenizer so MUI and Copper stay in lockstep.

pub mod ast;
pub mod loader;
pub mod style;

use ast::{Document, Element, Handler, MuiError, MuiValue, Node, Param, Prop, PropValue, View};
use copper_syntax::ast::Span;
use copper_syntax::expr::Expr;
use copper_syntax::tokenizer::kind::TokenKind;
use copper_syntax::tokenizer::tokenizer::Tokenizer;
use copper_syntax::tokenizer::tokens::Token;

/// Parse a MUI document. Never panics; recoverable errors land in
/// [`Document::errors`].
pub fn parse(source: &str) -> Document {
    // The Copper tokenizer silently drops `#` and splits the hex body, so a
    // `#rrggbb` color is unrecoverable post-lex. Rewrite each color literal to
    // a placeholder identifier (`__mui_color_RRGGBBAA`) that survives lexing
    // intact; `parse_arg_value` decodes it back into a `MuiValue::Color`.
    let source = rewrite_color_literals(source);
    let toks: Vec<Token> = Tokenizer::new(source.clone())
        .tokenize()
        .into_iter()
        .filter(|t| {
            let trivia = matches!(
                t.kind,
                TokenKind::Whitespace | TokenKind::Comment | TokenKind::DocComment | TokenKind::Eof
            );
            // Keep Newline tokens (the tokenizer's `;\n` / `,\n`) — they are the
            // statement separators we need to end a `let`. Drop only real
            // whitespace / comments / empty tokens.
            let empty_non_newline = t.kind != TokenKind::Newline && t.value.trim().is_empty();
            !(trivia || empty_non_newline)
        })
        .collect();
    let mut p = Parser {
        toks,
        pos: 0,
        errors: Vec::new(),
    };
    let mut views = Vec::new();
    let mut app = None;
    let mut imports = Vec::new();
    while p.pos < p.toks.len() {
        match p.peek_val() {
            Some("view") => {
                if let Some(v) = p.parse_view() {
                    views.push(v);
                }
            }
            // Component import: `import { Card } from "./card.mui"`.
            Some("import") if p.peek2_val() == Some("{") => {
                if let Some(imp) = p.parse_import() {
                    imports.push(imp);
                }
            }
            // Top-level app config: the preferred `App() { ... }` (capital,
            // empty parens) or the legacy `app { ... }`. Only the first kept.
            Some("App") | Some("app") if matches!(p.peek2_val(), Some("{") | Some("(")) => {
                let cfg = p.parse_app_config();
                if app.is_none() {
                    app = Some(cfg);
                }
            }
            _ => {
                // Skip anything else (Copper items in .crm, bare statements).
                p.pos += 1;
            }
        }
    }
    Document {
        views,
        imports,
        app,
        errors: p.errors,
    }
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    errors: Vec<MuiError>,
}

/// Placeholder identifier prefix for a color literal, injected by
/// [`rewrite_color_literals`] before lexing and decoded in `parse_arg_value`.
const COLOR_TAG: &str = "__mui_color_";

/// Rewrite `#rrggbb` / `#rrggbbaa` color literals to `__mui_color_RRGGBBAA`
/// identifiers (always 8 hex digits, alpha defaulted to `ff`) so they survive
/// Copper lexing as a single token. Skips `#` inside string literals and
/// comments. Everything else is passed through untouched.
fn rewrite_color_literals(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len() + 16);
    let mut i = 0; // byte position
    let n = bytes.len();

    while i < n {
        // Decode the current char from a valid UTF-8 slice — never split multi-byte sequences.
        let ch = source[i..].chars().next().unwrap_or('\0');
        let ch_len = ch.len_utf8();

        // Skip string literals verbatim, preserving their UTF-8 content intact.
        if ch == '"' {
            out.push(ch);
            i += ch_len;
            while i < n {
                let d = source[i..].chars().next().unwrap_or('\0');
                let d_len = d.len_utf8();
                out.push(d);
                i += d_len;
                if d == '\\' && i < n {
                    let e = source[i..].chars().next().unwrap_or('\0');
                    out.push(e);
                    i += e.len_utf8();
                } else if d == '"' {
                    break;
                }
            }
            continue;
        }

        // Skip line comments verbatim (content after `//` is always ASCII-safe to
        // copy char by char, but we stay UTF-8 correct just in case).
        if ch == '/' && i + 1 < n && bytes[i + 1] == b'/' {
            while i < n && bytes[i] != b'\n' {
                let c = source[i..].chars().next().unwrap_or('\0');
                out.push(c);
                i += c.len_utf8();
            }
            continue;
        }

        // A `#` starts a color literal when followed by exactly 6 or 8 ASCII hex
        // digits. The hex digits are always single-byte so `bytes[j]` is safe here.
        if ch == '#' {
            let mut j = i + 1;
            while j < n && bytes[j].is_ascii_hexdigit() {
                j += 1;
            }
            let len = j - (i + 1);
            if len == 6 || len == 8 {
                let hex = &source[i + 1..j];
                let with_alpha = if len == 6 {
                    format!("{hex}ff")
                } else {
                    hex.to_string()
                };
                out.push_str(COLOR_TAG);
                out.push_str(&with_alpha.to_lowercase());
                i = j;
                continue;
            }
        }

        out.push(ch);
        i += ch_len;
    }
    out
}

impl Parser {
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
    fn err(&mut self, message: impl Into<String>) {
        let span = self
            .toks
            .get(self.pos)
            .map(span_of)
            .unwrap_or(Span::new(0, 0));
        self.errors.push(MuiError {
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
        )
    }
    fn is_newline(&self) -> bool {
        self.peek_kind() == Some(TokenKind::Newline)
    }

    /// Parse `import { Name, Other } from "path"`. The names are the views
    /// pulled into scope as elements; the path is a string literal resolved
    /// relative to the importing file by the loader.
    fn parse_import(&mut self) -> Option<ast::Import> {
        let start = span_of(self.toks.get(self.pos)?);
        self.eat("import");
        if !self.eat("{") {
            return None;
        }
        let mut names = Vec::new();
        while let Some(v) = self.peek_val() {
            if v == "}" {
                break;
            }
            if v == "," || self.is_newline() {
                self.bump();
                continue;
            }
            // Inside `import { ... }` the tokenizer tags names as ModuleVar
            // (not Identifier), so accept those too.
            if self.is_ident() || self.peek_kind() == Some(TokenKind::ModuleVar) {
                names.push(self.bump().unwrap().value);
            } else {
                self.bump();
            }
        }
        self.eat("}");
        self.eat("from");
        // The path: collect the remaining tokens on this entry and pull the
        // string out (Copper makes a `Literal::Str` / `format("...")`).
        let (raw, _) = self.collect_arg_tokens();
        let path = match copper_syntax::expr::parse_expr(&raw).0 {
            Some(e) => prop_string(&PropValue::Expr(e)).unwrap_or(raw),
            None => raw,
        };
        let end = self.prev_span(start);
        let path = path.trim_matches('"').to_string();
        let kind = ast::ImportKind::from_path(&path);
        Some(ast::Import {
            names,
            path,
            kind,
            span: Span::merge(start, end),
        })
    }

    /// Parse a top-level `app { key: value, ... }` configuration block. Each
    /// entry's value is read with the same `parse_arg_value` classifier as
    /// element props, then coerced to the field's type.
    fn parse_app_config(&mut self) -> ast::AppConfig {
        let start = self
            .toks
            .get(self.pos)
            .map(span_of)
            .unwrap_or(Span::new(0, 0));
        // Keyword: `App` (preferred) or legacy `app`.
        if matches!(self.peek_val(), Some("App") | Some("app")) {
            self.bump();
        }
        // Optional empty parens of the `App()` form.
        if self.peek_val() == Some("(") {
            self.bump();
            if self.peek_val() == Some(")") {
                self.bump();
            }
        }
        // Tolerate a newline between `App()` and its `{` body.
        while self.is_newline() {
            self.bump();
        }
        let mut cfg = ast::AppConfig::default();
        if !self.eat("{") {
            cfg.span = start;
            return cfg;
        }
        while let Some(v) = self.peek_val() {
            if v == "}" {
                break;
            }
            if v == "," || v == ";" || self.is_newline() {
                self.bump();
                continue;
            }
            // `key :` pair.
            if self.is_ident() && self.peek2_val() == Some(":") {
                let key = self.bump().unwrap().value;
                self.bump(); // `:`
                let value = self.parse_arg_value();
                self.apply_app_field(&mut cfg, &key, value);
            } else {
                self.bump(); // tolerate stray tokens
            }
        }
        self.eat("}");
        cfg.span = Span::merge(start, self.prev_span(start));
        cfg
    }

    /// Coerce a parsed prop value into the matching [`ast::AppConfig`] field.
    fn apply_app_field(&self, cfg: &mut ast::AppConfig, key: &str, value: PropValue) {
        match key {
            "name" => cfg.name = prop_string(&value),
            "id" => cfg.id = prop_string(&value),
            "title" => cfg.title = prop_string(&value),
            // `mainContent` is the user-facing name for the entry component;
            // `entry` is the legacy alias.
            "entry" | "mainContent" => cfg.entry = prop_ident_or_string(&value),
            "host" => cfg.host = prop_ident_or_string(&value),
            "width" => cfg.width = prop_int(&value),
            "height" => cfg.height = prop_int(&value),
            "minWidth" => cfg.min_width = prop_int(&value),
            "minHeight" => cfg.min_height = prop_int(&value),
            "maxWidth" => cfg.max_width = prop_int(&value),
            "maxHeight" => cfg.max_height = prop_int(&value),
            "renderer" => cfg.renderer = prop_ident_or_string(&value),
            "msaa" => cfg.msaa = prop_int(&value),
            "aa" | "antialias" => cfg.aa = prop_ident_or_string(&value),
            "renderQuality" => cfg.render_quality = prop_ident_or_string(&value),
            "taaBlend" => cfg.taa_blend = prop_float(&value),
            "background" | "bg" => {
                if let PropValue::Mui(MuiValue::Color { r, g, b, a }) = value {
                    cfg.background = Some((r, g, b, a));
                }
            }
            _ => {} // unknown key — ignored (forward-compatible)
        }
    }

    fn parse_view(&mut self) -> Option<View> {
        let start = self
            .toks
            .get(self.pos)
            .map(span_of)
            .unwrap_or(Span::new(0, 0));
        self.eat("view");
        let name = self.expect_ident("view name");
        let params = self.parse_params();
        let body = if self.peek_val() == Some("{") {
            self.parse_block()
        } else {
            self.err("expected `{` to open the view body");
            Vec::new()
        };
        let end = self
            .toks
            .get(self.pos.saturating_sub(1))
            .map(span_of)
            .unwrap_or(start);
        Some(View {
            name,
            params,
            body,
            span: Span::merge(start, end),
        })
    }

    fn parse_params(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        if !self.eat("(") {
            return params;
        }
        while let Some(v) = self.peek_val() {
            if v == ")" {
                break;
            }
            if !self.is_ident() {
                self.bump();
                continue;
            }
            let start = span_of(self.toks.get(self.pos).unwrap());
            let name = self.expect_ident("parameter name");
            // Type annotation. The tokenizer may or may not surface the `:`
            // (param types can come through as ParamType directly), so eat an
            // optional colon and then collect the type tokens regardless.
            let mut ty = None;
            self.eat(":");
            let mut parts = Vec::new();
            while let Some(v) = self.peek_val() {
                if v == "," || v == ")" || v == "=" || self.is_newline() {
                    break;
                }
                parts.push(self.bump().unwrap().value);
            }
            if !parts.is_empty() {
                ty = Some(parts.join(""));
            }
            // Default value `= EXPR`, lowered to a Copper expression. Collect
            // the raw tokens up to the next top-level `,` / `)` and parse them.
            let mut default = None;
            if self.eat("=") {
                let (raw, span) = self.collect_arg_tokens();
                if !raw.is_empty() {
                    default = Some(copper_syntax::expr::parse_expr(&raw).0.unwrap_or_else(|| {
                        copper_syntax::expr::Expr::new(
                            copper_syntax::expr::ExprKind::Ident(raw),
                            span,
                        )
                    }));
                }
            }
            let end = self
                .toks
                .get(self.pos.saturating_sub(1))
                .map(span_of)
                .unwrap_or(start);
            params.push(Param {
                name,
                ty,
                default,
                span: Span::merge(start, end),
            });
            if !self.eat(",") {
                // tolerate missing comma
            }
        }
        self.eat(")");
        params
    }

    /// Parse a `{ ... }` body into a list of nodes. Assumes the next token is
    /// `{`.
    fn parse_block(&mut self) -> Vec<Node> {
        let mut nodes = Vec::new();
        if !self.eat("{") {
            return nodes;
        }
        while let Some(v) = self.peek_val() {
            if v == "}" {
                break;
            }
            // statement separators the tokenizer may have left in
            if v == ";" || v == "," || self.is_newline() {
                self.bump();
                continue;
            }
            let before = self.pos;
            if let Some(n) = self.parse_node() {
                nodes.push(n);
            }
            if self.pos == before {
                self.bump(); // guarantee progress
            }
        }
        self.eat("}");
        nodes
    }

    fn parse_node(&mut self) -> Option<Node> {
        match self.peek_val() {
            // Copper has no `let`. State is `mut name = expr` (mutable) or a
            // bare `name = expr` (immutable). The mutable form is keyword-led.
            Some("mut") => self.parse_bind_node(),
            Some("effect") => self.parse_effect_node(),
            Some("if") => self.parse_if_node(),
            Some("for") => self.parse_for_node(),
            Some("match") => self.parse_match_node(),
            // An identifier is ambiguous: `name = ...` is an immutable binding,
            // while `Name(...)` / `Name { ... }` is an element. Disambiguate by
            // looking at the token right after the identifier.
            _ if self.is_ident() => {
                if self.next_is_binding() {
                    self.parse_bind_node()
                } else {
                    self.parse_element().map(Node::Element)
                }
            }
            _ => {
                self.bump();
                None
            }
        }
    }

    /// True when the current identifier begins an immutable binding
    /// (`name = expr`): the next non-trivia token is a lone `=` (not `==`).
    fn next_is_binding(&self) -> bool {
        let next = self.toks.get(self.pos + 1);
        match next {
            Some(t) if t.value == "=" => true,
            // `=` may arrive glued to the start of `==`; a real binding has a
            // standalone `=` token, so `==` (comparison) is correctly excluded.
            _ => false,
        }
    }

    fn parse_element(&mut self) -> Option<Element> {
        let start = span_of(self.toks.get(self.pos)?);
        let name = self.expect_ident("element name");
        // Arguments `( positional?, name: value, ... )`.
        let mut positional = None;
        let mut props = Vec::new();
        let mut key = None;
        if self.peek_val() == Some("(") {
            self.parse_args(&mut positional, &mut props, &mut key);
        }
        // Children `{ ... }`.
        let children = if self.peek_val() == Some("{") {
            self.parse_block()
        } else {
            Vec::new()
        };
        let end = self
            .toks
            .get(self.pos.saturating_sub(1))
            .map(span_of)
            .unwrap_or(start);
        Some(Element {
            name,
            positional,
            props,
            children,
            key,
            span: Span::merge(start, end),
        })
    }

    /// Parse an element argument list `( ... )`. The first bare argument (no
    /// `name:`) is the positional content; `name: value` pairs are props.
    /// `key:` is lifted out for reconciliation. Each value is classified into
    /// a [`PropValue`] (handler, MUI color/enum literal, or Copper expression).
    fn parse_args(
        &mut self,
        positional: &mut Option<Expr>,
        props: &mut Vec<Prop>,
        key: &mut Option<Expr>,
    ) {
        if !self.eat("(") {
            return;
        }
        let mut seen_positional = false;
        loop {
            match self.peek_val() {
                Some(")") | None => break,
                Some(",") => {
                    self.bump();
                    continue;
                }
                _ => {}
            }
            if self.is_newline() {
                self.bump();
                continue;
            }
            let arg_start = span_of(self.toks.get(self.pos).unwrap());
            // A prop is `ident :` where `:` is the *next* token (not `::`).
            if self.is_ident() && self.peek2_val() == Some(":") {
                let name = self.bump().unwrap().value; // ident
                self.bump(); // `:`
                let value = self.parse_arg_value();
                let end = self.prev_span(arg_start);
                let span = Span::merge(arg_start, end);
                if name == "key" {
                    if let PropValue::Expr(e) = &value {
                        *key = Some(e.clone());
                    }
                }
                props.push(Prop { name, value, span });
            } else {
                // Positional argument (only the first one counts).
                let value = self.parse_arg_value();
                if !seen_positional {
                    if let PropValue::Expr(e) = value {
                        *positional = Some(e);
                    }
                    seen_positional = true;
                }
            }
        }
        self.eat(")");
    }

    /// Parse a single argument value up to the next top-level `,` or the
    /// closing `)`. Classifies it as a handler block, a MUI color/enum
    /// literal, or a lowered Copper expression.
    fn parse_arg_value(&mut self) -> PropValue {
        // Event handler: `{ |a, b| ... }` or `{ ... }`.
        if self.peek_val() == Some("{") {
            return PropValue::Handler(self.parse_handler());
        }
        // Collect the raw token run for this argument (balanced, stops at a
        // top-level `,` or `)`), then classify it.
        let (raw, exprs_span) = self.collect_arg_tokens();
        // Color literal: rewritten to `__mui_color_RRGGBBAA` before lexing.
        if let Some(color) = decode_color_tag(&raw) {
            return PropValue::Mui(color);
        }
        // `rgba(r, g, b, a)` / `rgb(r, g, b)` color function.
        if let Some(color) = parse_rgba_call(&raw) {
            return PropValue::Mui(color);
        }
        // Enum access `Type.Member`.
        if let Some(v) = classify_mui_value(&raw) {
            return PropValue::Mui(v);
        }
        // Fall back to a lowered Copper expression.
        match copper_syntax::expr::parse_expr(&raw).0 {
            Some(e) => PropValue::Expr(e),
            None => PropValue::Expr(copper_syntax::expr::Expr::new(
                copper_syntax::expr::ExprKind::Ident(raw),
                exprs_span,
            )),
        }
    }

    /// `{ |params| body }` / `{ body }` — recognised structurally. The body is
    /// captured by skipping the balanced braces (lowering the statements into a
    /// Copper [`Block`] is the evaluator's job, in M3).
    fn parse_handler(&mut self) -> Handler {
        let mut params = Vec::new();
        self.eat("{");
        // Optional `| a, b |` parameter list.
        if self.peek_val() == Some("|") {
            self.bump();
            while let Some(v) = self.peek_val() {
                if v == "|" {
                    break;
                }
                if v == "," {
                    self.bump();
                    continue;
                }
                if self.is_ident() {
                    params.push(self.bump().unwrap().value);
                } else {
                    self.bump();
                }
            }
            self.eat("|");
        }
        // Capture the body up to the matching `}` (one `{` already consumed),
        // joining the token text so the runtime/codegen can interpret simple
        // statements like `count = count + 1`. Spaces are inserted between
        // word-like tokens so `count`/`=`/`count`/`+`/`1` don't fuse.
        let mut depth = 1i32;
        let mut parts: Vec<String> = Vec::new();
        while let Some(v) = self.peek_val() {
            if v == "{" {
                depth += 1;
            } else if v == "}" {
                depth -= 1;
                if depth == 0 {
                    self.bump();
                    break;
                }
            }
            let tok = self.bump().unwrap().value;
            if !tok.trim().is_empty() {
                parts.push(tok);
            }
        }
        Handler {
            params,
            body: empty_block(),
            raw: parts.join(" "),
        }
    }

    /// Collect the raw source of one argument value: every token until a
    /// top-level `,` or the closing `)` (respecting nested brackets). Returns
    /// the joined source and its span.
    fn collect_arg_tokens(&mut self) -> (String, Span) {
        let start = self
            .toks
            .get(self.pos)
            .map(span_of)
            .unwrap_or(Span::new(0, 0));
        let mut parts: Vec<String> = Vec::new();
        let mut depth = 0i32;
        while let Some(v) = self.peek_val() {
            // A value ends at a top-level separator: a comma (element args), the
            // closing `)` of an arg list, the closing `}` of an `app {}` /
            // children block, or a newline (statement / app-entry boundary).
            if depth == 0 && (v == "," || v == ")" || v == "}") {
                break;
            }
            if depth == 0 && self.is_newline() {
                break;
            }
            // Inside nested brackets a newline is just whitespace — drop it.
            if self.is_newline() {
                self.bump();
                continue;
            }
            match v {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                _ => {}
            }
            parts.push(self.bump().unwrap().value);
        }
        let end = self.prev_span(start);
        // Join token text, inserting a space ONLY between two word-like tokens so
        // keywords don't fuse: `if true` was joining to `iftrue`, which then
        // misparsed as a struct literal `iftrue { ... }` (breaking conditional
        // props/text like `if ok { "[ok]" } else { "[x]" }`). Symbols, quotes and
        // braces still join tight, matching the source closely enough for Copper.
        let mut joined = String::new();
        for part in &parts {
            if let (Some(l), Some(f)) = (joined.chars().last(), part.chars().next()) {
                let wordy = |c: char| c.is_alphanumeric() || c == '_';
                if wordy(l) && wordy(f) {
                    joined.push(' ');
                }
            }
            joined.push_str(part);
        }
        (joined, Span::merge(start, end))
    }

    fn peek2_val(&self) -> Option<&str> {
        self.toks.get(self.pos + 1).map(|t| t.value.as_str())
    }

    fn prev_span(&self, fallback: Span) -> Span {
        self.toks
            .get(self.pos.saturating_sub(1))
            .map(span_of)
            .unwrap_or(fallback)
    }

    /// Parse a Copper binding node: `mut name = expr` (mutable) or
    /// `name = expr` (immutable). There is no `let` keyword in Copper.
    fn parse_bind_node(&mut self) -> Option<Node> {
        let start = span_of(self.toks.get(self.pos)?);
        let mutable = self.peek_val() == Some("mut");
        if mutable {
            self.bump(); // `mut`
        }
        let name = self.expect_ident("binding name");
        // Consume the `=` that introduces the initializer.
        self.eat("=");
        // Collect the initializer's tokens up to the statement boundary — a
        // Newline (`;\n`) or the block's `}` — and lower them to a Copper
        // expression (e.g. `signal(0)`, `computed { ... }`, a literal).
        let mut value = None;
        {
            let val_start = self
                .toks
                .get(self.pos)
                .map(span_of)
                .unwrap_or(Span::new(0, 0));
            let mut parts: Vec<String> = Vec::new();
            let mut depth = 0i32;
            while let Some(v) = self.peek_val() {
                if depth == 0 && (v == ";" || v == "}" || self.is_newline()) {
                    break;
                }
                match v {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {}
                }
                parts.push(self.bump().unwrap().value);
            }
            let raw = parts.join("");
            if !raw.is_empty() {
                value = Some(copper_syntax::expr::parse_expr(&raw).0.unwrap_or_else(|| {
                    copper_syntax::expr::Expr::new(
                        copper_syntax::expr::ExprKind::Ident(raw),
                        val_start,
                    )
                }));
            }
        }
        let end = self
            .toks
            .get(self.pos.saturating_sub(1))
            .map(span_of)
            .unwrap_or(start);
        Some(Node::Let {
            name,
            mutable,
            value,
            span: Span::merge(start, end),
        })
    }

    fn parse_effect_node(&mut self) -> Option<Node> {
        let start = span_of(self.toks.get(self.pos)?);
        self.eat("effect");
        // Capture the body text (space-joined token values, like a handler) so
        // the runtime/codegen can interpret its statements.
        let mut raw = String::new();
        if self.eat("{") {
            let mut depth = 1i32;
            let mut parts: Vec<String> = Vec::new();
            while let Some(v) = self.peek_val() {
                if v == "{" {
                    depth += 1;
                } else if v == "}" {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        break;
                    }
                }
                let tok = self.bump().unwrap().value;
                if !tok.trim().is_empty() {
                    parts.push(tok);
                }
            }
            raw = parts.join(" ");
        }
        let end = self
            .toks
            .get(self.pos.saturating_sub(1))
            .map(span_of)
            .unwrap_or(start);
        Some(Node::Effect {
            body: empty_block(),
            raw,
            span: Span::merge(start, end),
        })
    }

    fn parse_if_node(&mut self) -> Option<Node> {
        let start = span_of(self.toks.get(self.pos)?);
        self.eat("if");
        // Capture the condition tokens up to the opening `{` (space-joined), so
        // the runtime/codegen can evaluate which branch to render.
        let mut cond_parts: Vec<String> = Vec::new();
        while let Some(v) = self.peek_val() {
            if v == "{" {
                break;
            }
            let tok = self.bump().unwrap().value;
            if !tok.trim().is_empty() {
                cond_parts.push(tok);
            }
        }
        let cond_raw = cond_parts.join(" ");
        let then = self.parse_block();
        let els = if self.eat("else") {
            if self.peek_val() == Some("if") {
                self.parse_if_node().map(|n| vec![n])
            } else {
                Some(self.parse_block())
            }
        } else {
            None
        };
        let end = self
            .toks
            .get(self.pos.saturating_sub(1))
            .map(span_of)
            .unwrap_or(start);
        Some(Node::If {
            cond: raw_expr(),
            cond_raw,
            then,
            els,
            span: Span::merge(start, end),
        })
    }

    fn parse_for_node(&mut self) -> Option<Node> {
        let start = span_of(self.toks.get(self.pos)?);
        self.eat("for");
        let pattern = self.expect_ident("loop variable");
        self.eat("in");
        // Capture the iterable tokens up to the opening `{` (space-joined),
        // then lower them to a real Copper expression so codegen/runtime can
        // emit `for <pat> in <iter>`.
        let mut iter_parts: Vec<String> = Vec::new();
        while let Some(v) = self.peek_val() {
            if v == "{" {
                break;
            }
            let tok = self.bump().unwrap().value;
            if !tok.trim().is_empty() {
                iter_parts.push(tok);
            }
        }
        let iter_raw = iter_parts.join(" ");
        let iter = copper_syntax::expr::parse_expr(&iter_raw)
            .0
            .unwrap_or_else(raw_expr);
        let body = self.parse_block();
        let end = self
            .toks
            .get(self.pos.saturating_sub(1))
            .map(span_of)
            .unwrap_or(start);
        Some(Node::For {
            pattern,
            iter,
            body,
            span: Span::merge(start, end),
        })
    }

    fn parse_match_node(&mut self) -> Option<Node> {
        let start = span_of(self.toks.get(self.pos)?);
        self.eat("match");
        // For this increment, capture the match structurally by skipping its
        // body; arm-by-arm node parsing is the next step.
        while let Some(v) = self.peek_val() {
            if v == "{" {
                break;
            }
            self.bump();
        }
        if self.peek_val() == Some("{") {
            self.skip_balanced("{", "}");
        }
        let end = self
            .toks
            .get(self.pos.saturating_sub(1))
            .map(span_of)
            .unwrap_or(start);
        Some(Node::Match {
            scrutinee: raw_expr(),
            arms: Vec::new(),
            span: Span::merge(start, end),
        })
    }

    fn skip_balanced(&mut self, open: &str, close: &str) {
        if !self.eat(open) {
            return;
        }
        let mut depth = 1i32;
        while let Some(v) = self.peek_val() {
            if v == open {
                depth += 1;
            } else if v == close {
                depth -= 1;
                if depth == 0 {
                    self.bump();
                    return;
                }
            }
            self.bump();
        }
    }

    fn expect_ident(&mut self, what: &str) -> String {
        if self.is_ident() {
            return self.bump().unwrap().value;
        }
        self.err(format!("expected {what}"));
        String::new()
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

fn empty_block() -> copper_syntax::expr::Block {
    copper_syntax::expr::Block {
        stmts: Vec::new(),
        tail: None,
        span: Span::new(0, 0),
    }
}

/// Placeholder expression used where the structural pass records a node but
/// the embedded Copper expression isn't lowered yet (next increment).
fn raw_expr() -> copper_syntax::expr::Expr {
    copper_syntax::expr::parse_expr("0")
        .0
        .expect("placeholder literal parses")
}

/// Extract a string from a prop value (a string literal, lowered by Copper
/// into a `format("...")` call or a `Literal::Str`).
fn prop_string(value: &PropValue) -> Option<String> {
    use copper_syntax::expr::{ExprKind, Literal, StrPart};
    let PropValue::Expr(e) = value else {
        return None;
    };
    // Bare string literal.
    if let ExprKind::Literal(Literal::Str(t)) = &e.kind {
        let s: String = t
            .parts
            .iter()
            .filter_map(|p| match p {
                StrPart::Lit(s) => Some(s.clone()),
                StrPart::Expr(_) => None,
            })
            .collect();
        return Some(s);
    }
    // Copper lowers `"My App"` (no interpolation) to `format("My App")`.
    if let ExprKind::Call { callee, args, .. } = &e.kind {
        if matches!(&callee.kind, ExprKind::Ident(n) if n == "format" || n == "format!") {
            if let Some(first) = args.first() {
                if let ExprKind::Literal(Literal::Str(t)) = &first.kind {
                    let s: String = t
                        .parts
                        .iter()
                        .filter_map(|p| match p {
                            StrPart::Lit(s) => Some(s.clone()),
                            StrPart::Expr(_) => None,
                        })
                        .collect();
                    return Some(s);
                }
            }
        }
    }
    None
}

/// Extract an integer from a numeric-literal prop value.
fn prop_int(value: &PropValue) -> Option<i32> {
    use copper_syntax::expr::{ExprKind, Literal};
    let PropValue::Expr(e) = value else {
        return None;
    };
    match &e.kind {
        ExprKind::Literal(Literal::Int(i)) => Some(*i as i32),
        ExprKind::Literal(Literal::Float(f)) => Some(*f as i32),
        _ => None,
    }
}

fn prop_float(value: &PropValue) -> Option<f32> {
    use copper_syntax::expr::{ExprKind, Literal};
    let PropValue::Expr(e) = value else {
        return None;
    };
    match &e.kind {
        ExprKind::Literal(Literal::Int(i)) => Some(*i as f32),
        ExprKind::Literal(Literal::Float(f)) => Some(*f as f32),
        _ => None,
    }
}

/// Extract an identifier name (`entry: Counter`) or a string (`entry: "Counter"`).
fn prop_ident_or_string(value: &PropValue) -> Option<String> {
    use copper_syntax::expr::ExprKind;
    if let PropValue::Expr(e) = value {
        if let ExprKind::Ident(n) = &e.kind {
            return Some(n.clone());
        }
    }
    prop_string(value)
}

/// Decode a `__mui_color_RRGGBBAA` placeholder (8 hex digits) back into a
/// [`MuiValue::Color`]. Returns `None` if `raw` isn't a color placeholder.
fn decode_color_tag(raw: &str) -> Option<MuiValue> {
    let hex = raw.trim().strip_prefix(COLOR_TAG)?;
    parse_hex_color(hex)
}

/// Parse a `rgba(r, g, b, a)` / `rgb(r, g, b)` call into a [`MuiValue::Color`].
/// `r/g/b` are 0-255 ints; `a` is 0.0–1.0 (defaults to 1.0 for `rgb`). The raw
/// token run arrives space-joined (e.g. `rgba ( 15 , 23 , 42 , 0.5 )`), so we
/// strip the `rgb(a)(` prefix + `)` and split on commas.
fn parse_rgba_call(raw: &str) -> Option<MuiValue> {
    let s: String = raw.split_whitespace().collect();
    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let chan = |p: &str| -> Option<u8> {
        let v: f32 = p.parse().ok()?;
        Some(v.round().clamp(0.0, 255.0) as u8)
    };
    let r = chan(parts[0])?;
    let g = chan(parts[1])?;
    let b = chan(parts[2])?;
    let a = if parts.len() == 4 {
        let af: f32 = parts[3].parse().ok()?;
        (af.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        255
    };
    Some(MuiValue::Color { r, g, b, a })
}

/// Parse 6- or 8-digit hex into a [`MuiValue::Color`]. Returns `None` on a
/// non-hex char or wrong length.
fn parse_hex_color(hex: &str) -> Option<MuiValue> {
    let h = hex.trim();
    if h.len() != 6 && h.len() != 8 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    let r = byte(0)?;
    let g = byte(2)?;
    let b = byte(4)?;
    let a = if h.len() == 8 { byte(6)? } else { 255 };
    Some(MuiValue::Color { r, g, b, a })
}

/// Classify a raw argument source into a [`MuiValue`] if it's a MUI literal
/// the Copper grammar doesn't model: an `Enum.Member` access. (Colors are
/// handled earlier, before token collection, because of the `#` sigil.)
/// Returns `None` for anything that should be a normal Copper expression —
/// notably function calls (`signal(0)`) and arithmetic, which contain `(` or
/// operators.
fn classify_mui_value(raw: &str) -> Option<MuiValue> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // `Type.Member` — both sides identifier-ish, exactly one dot, no call/ops.
    if let Some((ty, member)) = s.split_once('.') {
        if is_plain_ident(ty) && is_plain_ident(member) {
            return Some(MuiValue::Enum {
                ty: Some(ty.to_string()),
                member: member.to_string(),
            });
        }
    }
    None
}

/// True for a bare identifier (letters, digits, `_`; not starting with a
/// digit) — i.e. no operators, dots, parens, or whitespace.
fn is_plain_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::Node;

    const HELLO: &str = r#"
view Hello(name: string = "world") {
  Stack(orientation: vertical, gap: 12, padding: 24) {
    Text("Hello, ${name}!", size: 28, color: #0f172a)
    Text("Built with mocida + copper.", size: 14, color: #64748b)
  }
}
"#;

    #[test]
    fn parses_view_header_and_params() {
        let doc = parse(HELLO);
        assert!(doc.errors.is_empty(), "errors: {:?}", doc.errors);
        assert_eq!(doc.views.len(), 1);
        let v = &doc.views[0];
        assert_eq!(v.name, "Hello");
        assert_eq!(v.params.len(), 1);
        assert_eq!(v.params[0].name, "name");
        assert_eq!(v.params[0].ty.as_deref(), Some("string"));
    }

    #[test]
    fn parses_element_tree() {
        let doc = parse(HELLO);
        let v = &doc.views[0];
        assert_eq!(v.body.len(), 1, "body: {:?}", v.body);
        let Node::Element(stack) = &v.body[0] else {
            panic!("expected Stack element, got {:?}", v.body[0]);
        };
        assert_eq!(stack.name, "Stack");
        assert_eq!(stack.children.len(), 2, "children: {:?}", stack.children);
        for child in &stack.children {
            let Node::Element(e) = child else {
                panic!("expected Text element");
            };
            assert_eq!(e.name, "Text");
        }
    }

    #[test]
    fn captures_positional_props_color_and_enum() {
        let doc = parse(HELLO);
        let v = &doc.views[0];
        let Node::Element(stack) = &v.body[0] else {
            panic!("expected Stack");
        };
        // Stack(orientation: vertical, gap: 12, padding: 24)
        // A bare ident (`vertical`) is syntactically a Copper expression; the
        // runtime resolves it to the right enum per prop. Only the explicit
        // dotted `Type.Member` form is a MuiValue::Enum (see the next test).
        assert_eq!(stack.props.len(), 3, "stack props: {:?}", stack.props);
        let orientation = stack
            .props
            .iter()
            .find(|p| p.name == "orientation")
            .unwrap();
        assert!(
            matches!(
                &orientation.value,
                PropValue::Expr(e) if matches!(&e.kind, copper_syntax::expr::ExprKind::Ident(n) if n == "vertical")
            ),
            "orientation `vertical` should be a Copper ident expr, got {:?}",
            orientation.value
        );
        let gap = stack.props.iter().find(|p| p.name == "gap").unwrap();
        assert!(
            matches!(&gap.value, PropValue::Expr(_)),
            "gap should be a Copper expr, got {:?}",
            gap.value
        );

        // First Text: positional "Hello, ${name}!", size: 28, color: #0f172a
        let Node::Element(text) = &stack.children[0] else {
            panic!("expected Text");
        };
        assert!(
            text.positional.is_some(),
            "Text should have a positional arg"
        );
        let color = text.props.iter().find(|p| p.name == "color").unwrap();
        let PropValue::Mui(cv) = &color.value else {
            panic!("color should be a MUI value, got {:?}", color.value);
        };
        assert_eq!(
            *cv,
            MuiValue::Color {
                r: 0x0f,
                g: 0x17,
                b: 0x2a,
                a: 255
            },
            "color #0f172a should parse to channels"
        );
    }

    #[test]
    fn captures_enum_access_and_handler() {
        let src = r#"
view V() {
  Text("hi", fontStyle: FontStyle.Bold)
  Button("Save", onClick: { save() })
  Slider(min: 0, max: 1, value: t, onChange: { |v| log(v) })
}
"#;
        let doc = parse(src);
        assert!(doc.errors.is_empty(), "errors: {:?}", doc.errors);
        let body = &doc.views[0].body;

        let Node::Element(text) = &body[0] else {
            panic!("Text")
        };
        let fs = text.props.iter().find(|p| p.name == "fontStyle").unwrap();
        let PropValue::Mui(fv) = &fs.value else {
            panic!("fontStyle should be a MUI enum value, got {:?}", fs.value);
        };
        assert_eq!(
            *fv,
            MuiValue::Enum {
                ty: Some("FontStyle".into()),
                member: "Bold".into()
            }
        );

        let Node::Element(button) = &body[1] else {
            panic!("Button")
        };
        let on_click = button.props.iter().find(|p| p.name == "onClick").unwrap();
        assert!(
            matches!(&on_click.value, PropValue::Handler(h) if h.params.is_empty()),
            "onClick should be a no-param handler, got {:?}",
            on_click.value
        );

        let Node::Element(slider) = &body[2] else {
            panic!("Slider")
        };
        let on_change = slider.props.iter().find(|p| p.name == "onChange").unwrap();
        assert!(
            matches!(&on_change.value, PropValue::Handler(h) if h.params == ["v"]),
            "onChange should have one param `v`, got {:?}",
            on_change.value
        );
        // `value: t` is a plain Copper expression (signal binding).
        let value = slider.props.iter().find(|p| p.name == "value").unwrap();
        assert!(matches!(&value.value, PropValue::Expr(_)));
    }

    #[test]
    fn parses_control_flow_and_state() {
        let src = r#"
view Counter(start: int = 0) {
  mut count = signal(start)
  Stack {
    Text("Count: ${count}")
    if count > 10 {
      Text("lots")
    } else {
      Text("few")
    }
  }
}
"#;
        let doc = parse(src);
        assert_eq!(doc.views.len(), 1);
        let body = &doc.views[0].body;
        // `mut count` binding + Stack element
        assert!(
            matches!(body[0], Node::Let { .. }),
            "first node: {:?}",
            body[0]
        );
        let Node::Element(stack) = &body[1] else {
            panic!("expected Stack");
        };
        assert!(
            stack.children.iter().any(|n| matches!(n, Node::If { .. })),
            "expected an If node among children"
        );
    }

    #[test]
    fn immutable_binding_vs_element() {
        // `total = ...` is an immutable Copper binding; `Text(...)` is an
        // element. The parser must not confuse the two — both start with an
        // identifier.
        let src = r#"
view V() {
  total = compute(1, 2)
  Text("ok")
}
"#;
        let doc = parse(src);
        assert!(doc.errors.is_empty(), "errors: {:?}", doc.errors);
        let body = &doc.views[0].body;
        assert_eq!(body.len(), 2, "body: {:?}", body);
        match &body[0] {
            Node::Let { name, mutable, .. } => {
                assert_eq!(name, "total");
                assert!(!mutable, "bare binding must be immutable");
            }
            other => panic!("expected immutable binding, got {:?}", other),
        }
        assert!(
            matches!(&body[1], Node::Element(e) if e.name == "Text"),
            "second node should be the Text element, got {:?}",
            body[1]
        );
    }

    #[test]
    fn parses_app_config_block() {
        let src = r#"
app {
  name: "My App"
  id: "net.liy77.myapp"
  width: 1024
  height: 720
  background: #0f172a
  entry: Home
}
view Home() { Text("hi") }
"#;
        let doc = parse(src);
        assert!(doc.errors.is_empty(), "errors: {:?}", doc.errors);
        let app = doc.app.expect("app config");
        assert_eq!(app.name.as_deref(), Some("My App"));
        assert_eq!(app.id.as_deref(), Some("net.liy77.myapp"));
        assert_eq!(app.width, Some(1024));
        assert_eq!(app.height, Some(720));
        assert_eq!(app.background, Some((0x0f, 0x17, 0x2a, 255)));
        assert_eq!(app.entry.as_deref(), Some("Home"));
        // Views still parse alongside the app block.
        assert_eq!(doc.views.len(), 1);
        assert_eq!(doc.views[0].name, "Home");
    }

    #[test]
    fn parses_app_paren_form_with_main_content() {
        // The preferred `App() { ... mainContent: View }` form (capital name,
        // empty parens, `mainContent` alias for `entry`).
        let src = r#"
App() {
  name: "Dashboard"
  width: 720
  height: 520
  background: #f1f5f9
  mainContent: Dashboard
}
view Dashboard() { Text("hi") }
"#;
        let doc = parse(src);
        assert!(doc.errors.is_empty(), "errors: {:?}", doc.errors);
        let app = doc.app.expect("app config");
        assert_eq!(app.name.as_deref(), Some("Dashboard"));
        assert_eq!(app.width, Some(720));
        assert_eq!(app.height, Some(520));
        assert_eq!(app.background, Some((0xf1, 0xf5, 0xf9, 255)));
        assert_eq!(app.entry.as_deref(), Some("Dashboard"));
    }
}
