//! Copper AST.
//!
//! Built from a token stream by [`parse`]. The builder is **recoverable**:
//! syntactic errors land in [`ParsedFile::errors`] but the builder keeps
//! going so the LSP can render diagnostics over a partial tree while the
//! user is mid-edit.
//!
//! Coverage today (Phase 1):
//! * Top-level items: `import`/`use`, `func`, `class`, `struct`, `impl`,
//!   `let`/`mut` declarations.
//! * Bodies are captured by their *span* but not yet recursively parsed
//!   into expression trees — enough to power document symbols and
//!   brace-balance diagnostics.

use crate::tokenizer::kind::TokenKind;
use crate::tokenizer::tokenizer::Tokenizer;
use crate::tokenizer::tokens::Token;

/// Byte-offset range into the original source. `start` is inclusive,
/// `end` is exclusive.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    pub fn empty(at: u32) -> Self {
        Self { start: at, end: at }
    }

    pub fn merge(a: Span, b: Span) -> Span {
        Span {
            start: a.start.min(b.start),
            end: a.end.max(b.end),
        }
    }

    pub fn contains(&self, offset: u32) -> bool {
        offset >= self.start && offset < self.end
    }
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub type_name: Option<String>,
    pub span: Span,
}

/// One declaration inside a `class` body — a field, method, or constructor.
/// Powers member completion (`obj.|`) and `self.` lookup in the LSP.
#[derive(Debug, Clone)]
pub enum ClassMember {
    Field {
        name: String,
        type_name: Option<String>,
        span: Span,
    },
    Method {
        name: String,
        params: Vec<Param>,
        return_type: Option<String>,
        span: Span,
        body: Option<Span>,
    },
    /// `ClassName(args) { ... }` — same name as the enclosing class.
    Constructor {
        params: Vec<Param>,
        span: Span,
        body: Option<Span>,
    },
}

impl ClassMember {
    pub fn name(&self) -> &str {
        match self {
            ClassMember::Field { name, .. } => name,
            ClassMember::Method { name, .. } => name,
            ClassMember::Constructor { .. } => "<constructor>",
        }
    }
}

#[derive(Debug, Clone)]
pub enum AstNode {
    Use {
        path: String,
        items: Vec<String>,
        alias: Option<String>,
        span: Span,
    },
    Function {
        name: String,
        return_type: Option<String>,
        params: Vec<Param>,
        body: Option<Span>,
        span: Span,
    },
    Struct {
        name: String,
        fields: Vec<Param>,
        span: Span,
    },
    Class {
        name: String,
        body: Option<Span>,
        members: Vec<ClassMember>,
        span: Span,
    },
    Impl {
        target: String,
        body: Option<Span>,
        span: Span,
    },
    Let {
        name: String,
        mutable: bool,
        type_hint: Option<String>,
        span: Span,
    },
}

impl AstNode {
    pub fn span(&self) -> Span {
        match self {
            AstNode::Use { span, .. }
            | AstNode::Function { span, .. }
            | AstNode::Struct { span, .. }
            | AstNode::Class { span, .. }
            | AstNode::Impl { span, .. }
            | AstNode::Let { span, .. } => *span,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            AstNode::Use { path, .. } => path,
            AstNode::Function { name, .. } => name,
            AstNode::Struct { name, .. } => name,
            AstNode::Class { name, .. } => name,
            AstNode::Impl { target, .. } => target,
            AstNode::Let { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxErrorKind {
    UnbalancedBrace,
    UnbalancedParen,
    UnbalancedBracket,
    UnexpectedToken,
}

#[derive(Debug, Clone)]
pub struct SyntaxError {
    pub span: Span,
    pub message: String,
    pub kind: SyntaxErrorKind,
}

#[derive(Debug, Default, Clone)]
pub struct ParsedFile {
    pub nodes: Vec<AstNode>,
    pub errors: Vec<SyntaxError>,
}

/// Parse `source` into a [`ParsedFile`]. Never panics on malformed input.
pub fn parse(source: &str) -> ParsedFile {
    let mut tokens = Tokenizer::new(source.to_string()).tokenize();
    fix_spans(&mut tokens, source);
    Builder::new(tokens, source.len()).build()
}

/// The tokenizer's `location_data.range` collapses to `(0, length)` because
/// the position machinery is line-relative and doesn't track absolute byte
/// offsets. The LSP needs absolute spans for ranges/diagnostics, so we
/// recompute them by walking the source forward and matching each token's
/// raw text against it. O(n) for well-formed input.
fn fix_spans(tokens: &mut [Token], source: &str) {
    use crate::tokenizer::tokens::LocationData;
    let bytes = source.as_bytes();
    let mut cursor = 0usize;
    for t in tokens.iter_mut() {
        if t.generated || matches!(t.kind, TokenKind::Eof) || t.value.is_empty() {
            continue;
        }
        // Newline tokens carry ";\n" / ",\n" (synthetic separator + real newline);
        // anchor on the trailing '\n' which actually appears in the source.
        let needle: &[u8] = if matches!(t.kind, TokenKind::Newline) {
            b"\n"
        } else {
            t.value.as_bytes()
        };
        if needle.is_empty() {
            continue;
        }
        let mut found = None;
        let mut i = cursor;
        while i + needle.len() <= bytes.len() {
            if &bytes[i..i + needle.len()] == needle {
                found = Some(i);
                break;
            }
            i += 1;
        }
        if let Some(s) = found {
            t.location_data = Some(LocationData {
                first_line: 0,
                first_column: 0,
                last_line: 0,
                last_column: 0,
                range: (s, s + needle.len()),
            });
            cursor = s + needle.len();
        }
    }
}

struct Builder {
    tokens: Vec<Token>,
    source_len: usize,
    cursor: usize,
    out: ParsedFile,
}

impl Builder {
    fn new(tokens: Vec<Token>, source_len: usize) -> Self {
        let filtered: Vec<Token> = tokens
            .into_iter()
            .filter(|t| {
                !matches!(
                    t.kind,
                    TokenKind::Whitespace | TokenKind::Newline | TokenKind::Comment
                )
            })
            .collect();
        Self {
            tokens: filtered,
            source_len,
            cursor: 0,
            out: ParsedFile::default(),
        }
    }

    fn build(mut self) -> ParsedFile {
        self.scan_brace_balance();

        while self.cursor < self.tokens.len() {
            let progressed = self.parse_top_level();
            if !progressed {
                self.cursor += 1;
            }
        }

        self.collect_nested_lets();

        self.out
    }

    /// Second pass: lift `let X` / `mut X` declarations from anywhere in
    /// the token stream — including function and class-method bodies that
    /// the top-level walker skipped over via `consume_braced_span`. The
    /// LSP needs these for hover/completion of locals.
    ///
    /// Dedups by span so top-level Lets that the main pass already added
    /// don't double-count.
    fn collect_nested_lets(&mut self) {
        let existing_starts: std::collections::HashSet<u32> = self
            .out
            .nodes
            .iter()
            .filter_map(|n| match n {
                AstNode::Let { span, .. } => Some(span.start),
                _ => None,
            })
            .collect();

        let mut i = 0usize;
        while i + 1 < self.tokens.len() {
            let t = &self.tokens[i];
            let is_let =
                matches!(t.kind, TokenKind::Keyword) && (t.value == "let" || t.value == "mut");
            if !is_let {
                i += 1;
                continue;
            }
            let next = &self.tokens[i + 1];
            // Bare `mut` modifier in a borrow / parameter context (e.g.
            // `&mut x`, `(mut p: …)`) — the next token isn't an
            // identifier in a declaration position. Skip if the prior
            // token was `&` or `(` (`,` for second param onward).
            let prev_value = if i > 0 {
                Some(self.tokens[i - 1].value.as_str())
            } else {
                None
            };
            if t.value == "mut"
                && matches!(prev_value, Some("&") | Some("(") | Some(",") | Some("*"))
            {
                i += 1;
                continue;
            }
            if !matches!(next.kind, TokenKind::Identifier) {
                i += 1;
                continue;
            }
            let span = span_of(t);
            if existing_starts.contains(&span.start) {
                i += 2;
                continue;
            }
            // The binding name span gives a more useful hover-target range
            // than the keyword position.
            let name_span = span_of(next);
            self.out.nodes.push(AstNode::Let {
                name: next.value.clone(),
                mutable: t.value == "mut",
                type_hint: None,
                span: Span::new(span.start, name_span.end),
            });
            i += 2;
        }
    }

    fn scan_brace_balance(&mut self) {
        let mut stack: Vec<(char, Span)> = Vec::new();
        for tok in &self.tokens {
            match tok.value.as_str() {
                "{" | "(" | "[" => {
                    let open = tok.value.chars().next().unwrap();
                    stack.push((open, span_of(tok)));
                }
                "}" => Self::pop_match(&mut self.out, &mut stack, '{', tok),
                ")" => Self::pop_match(&mut self.out, &mut stack, '(', tok),
                "]" => Self::pop_match(&mut self.out, &mut stack, '[', tok),
                _ => {}
            }
        }
        for (open, span) in stack {
            let (kind, msg) = match open {
                '{' => (
                    SyntaxErrorKind::UnbalancedBrace,
                    "Unmatched `{` — missing closing brace",
                ),
                '(' => (
                    SyntaxErrorKind::UnbalancedParen,
                    "Unmatched `(` — missing closing paren",
                ),
                '[' => (
                    SyntaxErrorKind::UnbalancedBracket,
                    "Unmatched `[` — missing closing bracket",
                ),
                _ => unreachable!(),
            };
            self.out.errors.push(SyntaxError {
                span,
                message: msg.to_string(),
                kind,
            });
        }
    }

    fn pop_match(out: &mut ParsedFile, stack: &mut Vec<(char, Span)>, expected: char, tok: &Token) {
        match stack.pop() {
            Some((open, _)) if open == expected => {}
            Some((open, span)) => {
                let (kind, msg) = match open {
                    '{' => (
                        SyntaxErrorKind::UnbalancedBrace,
                        format!("Expected `}}` to close this brace, found `{}`", tok.value),
                    ),
                    '(' => (
                        SyntaxErrorKind::UnbalancedParen,
                        format!("Expected `)` to close this paren, found `{}`", tok.value),
                    ),
                    '[' => (
                        SyntaxErrorKind::UnbalancedBracket,
                        format!("Expected `]` to close this bracket, found `{}`", tok.value),
                    ),
                    _ => unreachable!(),
                };
                out.errors.push(SyntaxError {
                    span,
                    message: msg,
                    kind,
                });
            }
            None => {
                let kind = match expected {
                    '{' => SyntaxErrorKind::UnbalancedBrace,
                    '(' => SyntaxErrorKind::UnbalancedParen,
                    _ => SyntaxErrorKind::UnbalancedBracket,
                };
                out.errors.push(SyntaxError {
                    span: span_of(tok),
                    message: format!("Stray `{}` with no matching opener", tok.value),
                    kind,
                });
            }
        }
    }

    fn parse_top_level(&mut self) -> bool {
        let tok = match self.tokens.get(self.cursor) {
            Some(t) => t.clone(),
            None => return false,
        };

        // Keywords that introduce expression-level constructs we don't (yet)
        // recurse into. Skip them — and any immediately-following `let` —
        // so we don't misinterpret patterns like `if let Some(x) = ...` as
        // a top-level binding declaration.
        if matches!(tok.kind, TokenKind::Keyword)
            && matches!(
                tok.value.as_str(),
                "if" | "else" | "return" | "yield" | "as" | "ref" | "move"
            )
        {
            self.cursor += 1;
            // Lookahead: if `let` or `mut` follows, swallow it too (and
            // don't enter parse_let, which would expect a plain ident).
            if let Some(next) = self.tokens.get(self.cursor) {
                if matches!(next.kind, TokenKind::Keyword)
                    && (next.value == "let" || next.value == "mut")
                {
                    self.cursor += 1;
                }
            }
            return true;
        }
        if matches!(
            tok.kind,
            TokenKind::Loop
                | TokenKind::While
                | TokenKind::For
                | TokenKind::Break
                | TokenKind::Continue
        ) {
            self.cursor += 1;
            if let Some(next) = self.tokens.get(self.cursor) {
                if matches!(next.kind, TokenKind::Keyword)
                    && (next.value == "let" || next.value == "mut")
                {
                    self.cursor += 1;
                }
            }
            return true;
        }

        match tok.kind {
            TokenKind::Keyword if tok.value == "func" => self.parse_function(),
            TokenKind::Keyword if tok.value == "struct" => self.parse_struct(),
            TokenKind::Struct => self.parse_struct(),
            TokenKind::Keyword if tok.value == "class" => self.parse_class(),
            TokenKind::Keyword if tok.value == "impl" => self.parse_impl(),
            TokenKind::Impl => self.parse_impl(),
            TokenKind::Import | TokenKind::ImportAll => self.parse_use(),
            TokenKind::Keyword if tok.value == "import" || tok.value == "use" => self.parse_use(),
            TokenKind::Keyword if tok.value == "mut" || tok.value == "let" => self.parse_let(),
            TokenKind::Identifier => self.parse_implicit_let(),
            _ => false,
        }
    }

    fn parse_function(&mut self) -> bool {
        let start = span_of(&self.tokens[self.cursor]).start;
        self.cursor += 1; // skip `func`

        // Optional return type (TokenKind::ReturnType from the tokenizer).
        let mut return_type = None;
        if let Some(t) = self.tokens.get(self.cursor) {
            if matches!(t.kind, TokenKind::ReturnType) {
                return_type = Some(t.value.clone());
                self.cursor += 1;
                // Optional generic args `<T, E>` after the return type.
                if matches!(
                    self.tokens.get(self.cursor).map(|t| t.value.as_str()),
                    Some("<")
                ) {
                    let mut depth = 0i32;
                    let mut buf = String::new();
                    while let Some(t) = self.tokens.get(self.cursor) {
                        let v = &t.value;
                        if v == "<" {
                            depth += 1;
                        } else if v == ">" {
                            depth -= 1;
                        }
                        buf.push_str(v);
                        self.cursor += 1;
                        if depth <= 0 {
                            break;
                        }
                    }
                    if let Some(rt) = return_type.as_mut() {
                        rt.push_str(&buf);
                    }
                }
            }
        }

        let name = match self.tokens.get(self.cursor) {
            Some(t) if matches!(t.kind, TokenKind::Identifier) => {
                let n = t.value.clone();
                self.cursor += 1;
                n
            }
            _ => {
                self.error_here("Expected function name after `func`");
                return true;
            }
        };

        let params = self.consume_param_list();
        let body = self.consume_braced_span();

        let end = body
            .map(|s| s.end)
            .or_else(|| self.last_consumed_end())
            .unwrap_or(start);
        self.out.nodes.push(AstNode::Function {
            name,
            return_type,
            params,
            body,
            span: Span::new(start, end),
        });
        true
    }

    fn parse_struct(&mut self) -> bool {
        let start = span_of(&self.tokens[self.cursor]).start;
        self.cursor += 1;
        let name = match self.tokens.get(self.cursor) {
            Some(t) if matches!(t.kind, TokenKind::Identifier) => {
                let n = t.value.clone();
                self.cursor += 1;
                n
            }
            _ => {
                self.error_here("Expected struct name after `struct`");
                return true;
            }
        };
        let mut fields = Vec::new();
        let body = self.consume_braced_span();
        if let Some(body_span) = body {
            self.collect_fields_from_span(body_span, &mut fields);
        }
        let end = body
            .map(|s| s.end)
            .unwrap_or_else(|| start + name.len() as u32);
        self.out.nodes.push(AstNode::Struct {
            name,
            fields,
            span: Span::new(start, end),
        });
        true
    }

    fn parse_class(&mut self) -> bool {
        let start = span_of(&self.tokens[self.cursor]).start;
        self.cursor += 1;
        let name = match self.tokens.get(self.cursor) {
            Some(t) if matches!(t.kind, TokenKind::Identifier) => {
                let n = t.value.clone();
                self.cursor += 1;
                n
            }
            _ => {
                self.error_here("Expected class name after `class`");
                return true;
            }
        };
        let body = self.consume_braced_span();
        let members = body
            .map(|b| self.collect_class_members(&name, b))
            .unwrap_or_default();
        let end = body.map(|s| s.end).unwrap_or(start + name.len() as u32);
        self.out.nodes.push(AstNode::Class {
            name,
            body,
            members,
            span: Span::new(start, end),
        });
        true
    }

    /// Walk tokens inside a class body and lift fields, constructors, and
    /// methods. Strict-enough patterns to avoid swallowing the bodies of
    /// other declarations:
    ///
    /// * `Ident :`            → field (read type until `,`/`;`/newline)
    /// * `ClassName (`        → constructor
    /// * `Type Ident (`       → method (Type may be `void`, an identifier,
    ///   or `ReturnType<...>` glued back together)
    ///
    /// We do a depth-aware scan so nested braces inside a method body
    /// don't produce phantom members.
    fn collect_class_members(&self, class_name: &str, body: Span) -> Vec<ClassMember> {
        let inside: Vec<&Token> = self
            .tokens
            .iter()
            .filter(|t| {
                let s = span_of(t);
                s.start > body.start && s.end <= body.end
            })
            .collect();

        let mut members = Vec::new();
        let mut i = 0usize;
        // Strip the closing `}` if it landed in the slice.
        let end_i = inside.len();

        while i < end_i {
            let t = inside[i];
            if !matches!(t.kind, TokenKind::Identifier) {
                i += 1;
                continue;
            }

            // ---- field: Ident `:` Type ... ----
            if let Some(colon) = inside.get(i + 1) {
                if colon.value == ":" {
                    let name_tok = t;
                    let mut j = i + 2;
                    let mut type_buf = String::new();
                    let mut depth = 0i32;
                    while j < end_i {
                        let cur = inside[j];
                        let v = &cur.value;
                        if depth == 0 {
                            // Hard terminators that always end the type.
                            if v == "," || v == ";" || v == "{" || v == "}" {
                                break;
                            }
                            // Lookahead: another `Ident :` (next field) or
                            // `Ident (` (next method/constructor) means our
                            // type ended at the previous token. The token
                            // stream is whitespace-filtered so adjacency is
                            // semantic.
                            if matches!(
                                cur.kind,
                                TokenKind::Identifier | TokenKind::ReturnType | TokenKind::Keyword
                            ) && !type_buf.is_empty()
                            {
                                // Pattern A: another field starting (`Ident :`).
                                // Pattern B: a constructor starting (`ClassName (`).
                                // Pattern C: a method starting (`ReturnType Ident (`).
                                let next1 = inside.get(j + 1).map(|t| t.value.as_str());
                                let next2 = inside.get(j + 2).map(|t| t.value.as_str());
                                if next1 == Some(":") || next1 == Some("(") {
                                    break;
                                }
                                if matches!(
                                    inside.get(j + 1).map(|t| t.kind),
                                    Some(TokenKind::Identifier) | Some(TokenKind::ReturnType)
                                ) && next2 == Some("(")
                                {
                                    break;
                                }
                            }
                        }
                        if v == "<" || v == "(" || v == "[" {
                            depth += 1;
                        } else if v == ">" || v == ")" || v == "]" {
                            depth -= 1;
                        }
                        append_type_token(&mut type_buf, v);
                        j += 1;
                    }
                    let type_name = if type_buf.trim().is_empty() {
                        None
                    } else {
                        Some(type_buf.trim().to_string())
                    };
                    let span = Span::merge(span_of(name_tok), span_of(inside[j.min(end_i) - 1]));
                    members.push(ClassMember::Field {
                        name: name_tok.value.clone(),
                        type_name,
                        span,
                    });
                    i = j;
                    continue;
                }
            }

            // ---- constructor: ClassName `(` ... ----
            if t.value == class_name {
                if let Some(paren) = inside.get(i + 1) {
                    if paren.value == "(" {
                        let (params, body_span, consumed) = parse_paren_then_body(&inside, i + 1);
                        let span = Span::merge(
                            span_of(t),
                            body_span
                                .unwrap_or_else(|| span_of(inside[(i + consumed).min(end_i) - 1])),
                        );
                        members.push(ClassMember::Constructor {
                            params,
                            span,
                            body: body_span,
                        });
                        i += consumed;
                        continue;
                    }
                }
            }

            // ---- method: ReturnType Ident `(` ... ----
            // The first token is a type-like (Identifier/Keyword/ReturnType),
            // followed by an Identifier name, followed by `(`.
            let is_type_like = matches!(
                t.kind,
                TokenKind::Identifier | TokenKind::ReturnType | TokenKind::Keyword
            );
            // Allow generic args after the return type: collect `Ret<...>`
            // by scanning for the next plain identifier whose successor is `(`.
            if is_type_like {
                let mut k = i + 1;
                let mut return_buf = t.value.clone();
                // Pull in `<...>` if present.
                if inside.get(k).map(|x| x.value.as_str()) == Some("<") {
                    let mut depth = 0i32;
                    while k < end_i {
                        let v = &inside[k].value;
                        if v == "<" {
                            depth += 1;
                        } else if v == ">" {
                            depth -= 1;
                        }
                        append_type_token(&mut return_buf, v);
                        k += 1;
                        if depth <= 0 {
                            break;
                        }
                    }
                }
                if let (Some(name_tok), Some(paren)) = (inside.get(k), inside.get(k + 1)) {
                    if matches!(name_tok.kind, TokenKind::Identifier) && paren.value == "(" {
                        let (params, body_span, consumed) = parse_paren_then_body(&inside, k + 1);
                        let span =
                            Span::merge(span_of(t), body_span.unwrap_or_else(|| span_of(name_tok)));
                        members.push(ClassMember::Method {
                            name: name_tok.value.clone(),
                            params,
                            return_type: Some(return_buf),
                            span,
                            body: body_span,
                        });
                        i = k + 1 + consumed;
                        continue;
                    }
                }
            }

            i += 1;
        }

        members
    }

    fn parse_impl(&mut self) -> bool {
        let start = span_of(&self.tokens[self.cursor]).start;
        self.cursor += 1;
        let target = match self.tokens.get(self.cursor) {
            Some(t) if matches!(t.kind, TokenKind::Identifier) => {
                let n = t.value.clone();
                self.cursor += 1;
                n
            }
            _ => {
                self.error_here("Expected target type after `impl`");
                return true;
            }
        };
        let body = self.consume_braced_span();
        let end = body.map(|s| s.end).unwrap_or(start + target.len() as u32);
        self.out.nodes.push(AstNode::Impl {
            target,
            body,
            span: Span::new(start, end),
        });
        true
    }

    fn parse_use(&mut self) -> bool {
        let start_tok = self.tokens[self.cursor].clone();
        let start = span_of(&start_tok).start;
        self.cursor += 1;

        let mut items = Vec::new();
        let mut alias = None;

        if matches!(
            self.tokens.get(self.cursor).map(|t| t.value.as_str()),
            Some("{")
        ) {
            self.cursor += 1;
            while let Some(t) = self.tokens.get(self.cursor) {
                if t.value == "}" {
                    self.cursor += 1;
                    break;
                }
                if matches!(t.kind, TokenKind::Identifier) {
                    items.push(t.value.clone());
                }
                self.cursor += 1;
            }
        } else if let Some(t) = self.tokens.get(self.cursor) {
            if matches!(t.kind, TokenKind::Identifier) {
                alias = Some(t.value.clone());
                self.cursor += 1;
            }
        }

        let mut path = String::new();
        if matches!(
            self.tokens.get(self.cursor).map(|t| t.value.as_str()),
            Some("from")
        ) {
            self.cursor += 1;
            while let Some(t) = self.tokens.get(self.cursor) {
                if t.value == ";" {
                    self.cursor += 1;
                    break;
                }
                path.push_str(&t.value);
                self.cursor += 1;
            }
        }

        let end = self
            .last_consumed_end()
            .unwrap_or(start + start_tok.value.len() as u32);
        self.out.nodes.push(AstNode::Use {
            path,
            items,
            alias,
            span: Span::new(start, end),
        });
        true
    }

    fn parse_let(&mut self) -> bool {
        let start_tok = self.tokens[self.cursor].clone();
        let start = span_of(&start_tok).start;
        let mutable = start_tok.value == "mut";
        self.cursor += 1;

        // `let mut x` — swallow the inner `mut` so the name captured
        // below is the actual binding.
        if let Some(t) = self.tokens.get(self.cursor) {
            if matches!(t.kind, TokenKind::Keyword) && t.value == "mut" {
                self.cursor += 1;
            }
        }

        // Only treat `let X = ...` / `mut X = ...` as a binding declaration
        // when X is a plain identifier. Patterns like `let Some(v) = ...`,
        // `let (a, b) = ...`, `let _ = ...` are valid but not Let nodes —
        // skip silently rather than emit a false-positive diagnostic. The
        // parser is recoverable; the LSP just doesn't surface a symbol
        // for the destructured form yet.
        let name = match self.tokens.get(self.cursor) {
            Some(t) if matches!(t.kind, TokenKind::Identifier) => {
                let n = t.value.clone();
                self.cursor += 1;
                n
            }
            _ => return true,
        };
        let end = self
            .last_consumed_end()
            .unwrap_or(start + start_tok.value.len() as u32 + 1);
        self.out.nodes.push(AstNode::Let {
            name,
            mutable,
            type_hint: None,
            span: Span::new(start, end),
        });
        true
    }

    fn parse_implicit_let(&mut self) -> bool {
        let ident_tok = self.tokens[self.cursor].clone();
        let next = self.tokens.get(self.cursor + 1);
        let is_assign = matches!(next.map(|t| t.value.as_str()), Some("="));
        if !is_assign {
            return false;
        }
        let start = span_of(&ident_tok).start;
        let end = start + ident_tok.value.len() as u32;
        self.out.nodes.push(AstNode::Let {
            name: ident_tok.value.clone(),
            mutable: false,
            type_hint: None,
            span: Span::new(start, end),
        });
        self.cursor += 1;
        true
    }

    fn consume_param_list(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        if !matches!(
            self.tokens.get(self.cursor).map(|t| t.value.as_str()),
            Some("(")
        ) {
            return params;
        }
        self.cursor += 1;
        let mut depth = 1i32;
        let mut current_name: Option<(String, Span)> = None;
        let mut after_colon = false;
        let mut current_type = String::new();

        while let Some(t) = self.tokens.get(self.cursor).cloned() {
            self.cursor += 1;
            match t.value.as_str() {
                "(" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some((name, span)) = current_name.take() {
                            let type_name = if current_type.is_empty() {
                                None
                            } else {
                                Some(current_type.trim().to_string())
                            };
                            params.push(Param {
                                name,
                                type_name,
                                span,
                            });
                        }
                        return params;
                    }
                }
                "," if depth == 1 => {
                    if let Some((name, span)) = current_name.take() {
                        let type_name = if current_type.is_empty() {
                            None
                        } else {
                            Some(current_type.trim().to_string())
                        };
                        params.push(Param {
                            name,
                            type_name,
                            span,
                        });
                    }
                    current_type.clear();
                    after_colon = false;
                }
                ":" if depth == 1 => {
                    after_colon = true;
                    // Skip the rest of this iteration so the colon doesn't
                    // accidentally fall into the identifier-recognition arm
                    // below (it's `_`, so it won't, but keep the intent
                    // explicit).
                    continue;
                }
                _ => {
                    // The tokenizer marks identifiers inside `()` as
                    // `Param` (name) or `ParamType` (after `:`); plain
                    // `Identifier` shows up only outside that context.
                    // `self`/`Self` are tagged Keyword but are legal as
                    // param names — accept them too.
                    let is_self = matches!(t.kind, TokenKind::Keyword)
                        && (t.value == "self" || t.value == "Self");
                    let is_name_like =
                        matches!(t.kind, TokenKind::Identifier | TokenKind::Param) || is_self;
                    if is_name_like && !after_colon && current_name.is_none() {
                        current_name = Some((t.value.clone(), span_of(&t)));
                    } else if after_colon {
                        // Type side: anything contributes (Identifier,
                        // ParamType, Operator like `&`, `*`, generics).
                        append_type_token(&mut current_type, &t.value);
                    }
                }
            }
        }
        params
    }

    fn collect_fields_from_span(&self, body_span: Span, fields: &mut Vec<Param>) {
        let mut current_name: Option<(String, Span)> = None;
        let mut current_type = String::new();
        let mut after_colon = false;
        for t in &self.tokens {
            let s = span_of(t);
            if s.start <= body_span.start || s.end > body_span.end {
                continue;
            }
            match t.value.as_str() {
                "," | ";" => {
                    if let Some((name, span)) = current_name.take() {
                        let type_name = if current_type.is_empty() {
                            None
                        } else {
                            Some(current_type.trim().to_string())
                        };
                        fields.push(Param {
                            name,
                            type_name,
                            span,
                        });
                    }
                    current_type.clear();
                    after_colon = false;
                }
                ":" => after_colon = true,
                _ => {
                    if matches!(t.kind, TokenKind::Identifier)
                        && !after_colon
                        && current_name.is_none()
                    {
                        current_name = Some((t.value.clone(), s));
                    } else if after_colon {
                        append_type_token(&mut current_type, &t.value);
                    }
                }
            }
        }
        if let Some((name, span)) = current_name {
            let type_name = if current_type.is_empty() {
                None
            } else {
                Some(current_type.trim().to_string())
            };
            fields.push(Param {
                name,
                type_name,
                span,
            });
        }
    }

    fn consume_braced_span(&mut self) -> Option<Span> {
        if !matches!(
            self.tokens.get(self.cursor).map(|t| t.value.as_str()),
            Some("{")
        ) {
            return None;
        }
        let start = span_of(&self.tokens[self.cursor]).start;
        self.cursor += 1;
        let mut depth = 1i32;
        while let Some(t) = self.tokens.get(self.cursor) {
            match t.value.as_str() {
                "{" => depth += 1,
                "}" => {
                    depth -= 1;
                    if depth == 0 {
                        let end = span_of(t).end;
                        self.cursor += 1;
                        return Some(Span::new(start, end));
                    }
                }
                _ => {}
            }
            self.cursor += 1;
        }
        None
    }

    fn last_consumed_end(&self) -> Option<u32> {
        if self.cursor == 0 {
            return None;
        }
        let t = self.tokens.get(self.cursor - 1)?;
        Some(span_of(t).end)
    }

    fn error_here(&mut self, msg: &str) {
        let span = self
            .tokens
            .get(self.cursor)
            .map(span_of)
            .unwrap_or_else(|| Span::empty(self.source_len as u32));
        self.out.errors.push(SyntaxError {
            span,
            message: msg.to_string(),
            kind: SyntaxErrorKind::UnexpectedToken,
        });
    }
}

/// Append `value` to `buf`, inserting a single space when both adjacent
/// chars are word-chars (or a comma is on the left). Keeps complex types
/// like `*const i32`, `Vec<i32>`, `Result<T, E>` printable without
/// running tokens together (`*consti32` was the symptom).
fn append_type_token(buf: &mut String, value: &str) {
    if let (Some(last), Some(first)) = (buf.chars().last(), value.chars().next()) {
        let last_word = last.is_alphanumeric() || last == '_';
        let first_word = first.is_alphanumeric() || first == '_';
        let needs_space = (last_word || last == ',') && first_word;
        if needs_space {
            buf.push(' ');
        }
    }
    buf.push_str(value);
}

/// Starting at `inside[start]` which must be `(`, parse the param list and
/// the optional `{ ... }` body that follows. Returns `(params, body_span,
/// tokens_consumed_past_start)`.
///
/// Used for class methods + constructors so we don't reinvent param/body
/// scanning per member kind.
fn parse_paren_then_body(inside: &[&Token], start: usize) -> (Vec<Param>, Option<Span>, usize) {
    if inside.get(start).map(|t| t.value.as_str()) != Some("(") {
        return (Vec::new(), None, 1);
    }
    let mut params = Vec::new();
    let mut depth = 1i32;
    let mut current_name: Option<(String, Span)> = None;
    let mut after_colon = false;
    let mut current_type = String::new();
    let mut i = start + 1;
    while i < inside.len() {
        let t = inside[i];
        match t.value.as_str() {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth == 0 {
                    if let Some((name, span)) = current_name.take() {
                        let type_name = if current_type.is_empty() {
                            None
                        } else {
                            Some(current_type.trim().to_string())
                        };
                        params.push(Param {
                            name,
                            type_name,
                            span,
                        });
                    }
                    i += 1;
                    break;
                }
            }
            "," if depth == 1 => {
                if let Some((name, span)) = current_name.take() {
                    let type_name = if current_type.is_empty() {
                        None
                    } else {
                        Some(current_type.trim().to_string())
                    };
                    params.push(Param {
                        name,
                        type_name,
                        span,
                    });
                }
                current_type.clear();
                after_colon = false;
            }
            ":" if depth == 1 => {
                after_colon = true;
                i += 1;
                continue;
            }
            _ => {
                // `self`/`Self` are keywords but legal param names in
                // methods. Accept Identifier, Param, and the small set of
                // keywords that act as identifiers in param position.
                let is_self = matches!(t.kind, TokenKind::Keyword)
                    && (t.value == "self" || t.value == "Self");
                let is_name_like =
                    matches!(t.kind, TokenKind::Identifier | TokenKind::Param) || is_self;
                if is_name_like && !after_colon && current_name.is_none() {
                    current_name = Some((t.value.clone(), span_of(t)));
                } else if after_colon {
                    append_type_token(&mut current_type, &t.value);
                }
            }
        }
        i += 1;
    }

    // Now consume an optional `{ ... }` body.
    let mut body = None;
    if inside.get(i).map(|t| t.value.as_str()) == Some("{") {
        let body_start = span_of(inside[i]).start;
        let mut bd = 1i32;
        i += 1;
        while i < inside.len() {
            let v = &inside[i].value;
            if v == "{" {
                bd += 1;
            } else if v == "}" {
                bd -= 1;
                if bd == 0 {
                    let body_end = span_of(inside[i]).end;
                    body = Some(Span::new(body_start, body_end));
                    i += 1;
                    break;
                }
            }
            i += 1;
        }
    }

    (params, body, i - start)
}

fn span_of(t: &Token) -> Span {
    let (start, end) = t
        .location_data
        .as_ref()
        .map(|l| (l.range.0 as u32, l.range.1 as u32))
        .unwrap_or((0, t.length as u32));
    Span::new(start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_function() {
        // Copper functions always carry an explicit return type: `func void name(...) {}`.
        let p = parse("func void name() {}\n");
        assert_eq!(p.errors.len(), 0, "errors: {:?}", p.errors);
        assert_eq!(p.nodes.len(), 1);
        match &p.nodes[0] {
            AstNode::Function {
                name, return_type, ..
            } => {
                assert_eq!(name, "name");
                assert_eq!(return_type.as_deref(), Some("void"));
            }
            other => panic!("expected function, got {:?}", other),
        }
    }

    #[test]
    fn parses_function_with_generic_return() {
        let p = parse("func Result<i32, String> parse(s: str) {}\n");
        assert_eq!(p.nodes.len(), 1);
        if let AstNode::Function {
            name, return_type, ..
        } = &p.nodes[0]
        {
            assert_eq!(name, "parse");
            assert!(
                return_type.as_deref().unwrap_or("").starts_with("Result"),
                "return_type = {:?}",
                return_type
            );
        } else {
            panic!("expected function");
        }
    }

    #[test]
    fn raw_pointer_param_keeps_spaces() {
        // Regression: `*const i32` was being concatenated as `*consti32`
        // because tokens were appended without word-boundary spaces.
        let p = parse("func i32 deref(p: *const i32) {}\n");
        let AstNode::Function { params, .. } = &p.nodes[0] else {
            panic!("expected function");
        };
        assert_eq!(params[0].type_name.as_deref(), Some("*const i32"));
    }

    #[test]
    fn captures_function_params() {
        let p = parse("func String input(prompt: &str) {}\n");
        assert!(p.errors.is_empty(), "errors: {:?}", p.errors);
        let AstNode::Function { params, .. } = &p.nodes[0] else {
            panic!("expected function, got {:?}", p.nodes[0]);
        };
        assert_eq!(params.len(), 1, "params: {:?}", params);
        assert_eq!(params[0].name, "prompt");
        assert_eq!(params[0].type_name.as_deref(), Some("&str"));
    }

    #[test]
    fn parses_class_with_field_and_method() {
        let src = "class Greeter {\n  name: String\n\n  Greeter(name: String) {\n    self.name = name\n  }\n\n  void hello(self) {\n    println!(\"hi\")\n  }\n}\n";
        let p = parse(src);
        assert!(p.errors.is_empty(), "errors: {:?}", p.errors);
        let AstNode::Class { name, members, .. } = &p.nodes[0] else {
            panic!("expected class, got {:?}", p.nodes[0]);
        };
        assert_eq!(name, "Greeter");
        assert_eq!(members.len(), 3, "members: {:?}", members);

        let field = &members[0];
        let ClassMember::Field {
            name: fname,
            type_name,
            ..
        } = field
        else {
            panic!("expected Field, got {:?}", field);
        };
        assert_eq!(fname, "name");
        assert_eq!(type_name.as_deref(), Some("String"));

        let ctor = &members[1];
        let ClassMember::Constructor { params, .. } = ctor else {
            panic!("expected Constructor, got {:?}", ctor);
        };
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "name");

        let method = &members[2];
        let ClassMember::Method {
            name: mname,
            params,
            ..
        } = method
        else {
            panic!("expected Method, got {:?}", method);
        };
        assert_eq!(mname, "hello");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "self");
    }

    #[test]
    fn captures_multiple_params() {
        let p = parse("func void greet(name: str, age: i32) {}\n");
        let AstNode::Function { params, .. } = &p.nodes[0] else {
            panic!("expected function");
        };
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "name");
        assert_eq!(params[0].type_name.as_deref(), Some("str"));
        assert_eq!(params[1].name, "age");
        assert_eq!(params[1].type_name.as_deref(), Some("i32"));
    }

    #[test]
    fn if_let_pattern_emits_no_error() {
        // `Some` is a RUST_KEYWORD and tokenizes as Keyword (not Identifier).
        // Our parse_let should silently skip the destructuring form rather
        // than emit "Expected identifier after `let`".
        let p = parse("if let Some(value) = maybe { }\n");
        assert!(
            p.errors.is_empty(),
            "expected no errors, got {:?}",
            p.errors
        );
    }

    #[test]
    fn while_let_pattern_emits_no_error() {
        let p = parse("while let Some(n) = iter.next() { }\n");
        assert!(p.errors.is_empty(), "errors: {:?}", p.errors);
    }

    #[test]
    fn detects_unbalanced_brace() {
        let p = parse("func void a() { x = 1\n");
        assert!(
            p.errors
                .iter()
                .any(|e| e.kind == SyntaxErrorKind::UnbalancedBrace),
            "errors: {:?}",
            p.errors
        );
    }

    #[test]
    fn parses_struct_with_fields() {
        let p = parse("struct Point { x: int, y: int }\n");
        assert_eq!(p.nodes.len(), 1);
        if let AstNode::Struct { name, fields, span } = &p.nodes[0] {
            assert_eq!(name, "Point");
            assert_eq!(fields.len(), 2, "fields: {:?}", fields);
            assert_eq!(fields[0].name, "x");
            assert_eq!(fields[0].type_name.as_deref(), Some("int"));
            assert_eq!(fields[1].name, "y");
            assert!(span.start < span.end);
        } else {
            panic!("expected struct");
        }
    }

    #[test]
    fn tokenizes_simple_import() {
        // Simplest possible import — alias form, no braces.
        let toks = Tokenizer::new("import x from std\n".to_string()).tokenize();
        assert!(!toks.is_empty());
    }

    #[test]
    fn tokenizes_braced_import() {
        // The form that was hanging: braces with multiple items.
        let toks = Tokenizer::new("import { a, b } from std\n".to_string()).tokenize();
        assert!(!toks.is_empty());
    }
}
