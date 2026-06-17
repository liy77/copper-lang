use crate::{tokenizer::tokens::*, ConsumedTrait};

use super::{kind::TokenKind, tokens::Token};
use crate::utils::Consumed;
use once_cell::sync::Lazy;
use regex::Regex;

/// Remove `/* ... */` block comments from `source`, leaving everything else
/// (including embedded newlines) intact so subsequent tokenization keeps the
/// same line numbers. Block comments inside string literals are preserved.
/// Block comments do not nest in Copper, mirroring Rust's older rules — the
/// first `*/` ends the comment.
fn strip_block_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_string = false;
    let mut escape = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        if c == '"' {
            in_string = true;
            out.push(c);
            continue;
        }

        // Line comment `// …`: copy it verbatim to end of line. A `/*` that
        // appears INSIDE a line comment (e.g. a glob path like `ui/*.mui` in a
        // doc comment) must NOT open a block comment — without this guard the
        // scan ran to the next `*/`/EOF and silently swallowed real code after
        // the comment (a `view` after such a line vanished -> "no view").
        if c == '/' && chars.peek() == Some(&'/') {
            out.push(c);
            out.push(chars.next().unwrap()); // the second '/'
            for next in chars.by_ref() {
                out.push(next);
                if next == '\n' {
                    break;
                }
            }
            continue;
        }

        if c == '/' && chars.peek() == Some(&'*') {
            chars.next(); // consume `*`
                          // Skip until matching `*/`, preserving any newlines so token
                          // location data stays aligned with the source file.
            let mut prev = '\0';
            for next in chars.by_ref() {
                if prev == '*' && next == '/' {
                    break;
                }
                if next == '\n' {
                    out.push('\n');
                }
                prev = next;
            }
            continue;
        }

        out.push(c);
    }

    out
}

pub(super) type EndToken = Token;

impl EndToken {
    pub fn new_end(kind: TokenKind, value: String, length: usize, data: Data) -> Self {
        Self {
            kind,
            value,
            length,
            data,
            generated: false,
            struct_brace: false,
            origin: None,
            location_data: None,
        }
    }
}

// Operator categories + boolean literals now live in the shared lexicon so the
// tokenizer, the transpiler's type conversion, and the typed expression AST all
// read one definition (see crate::lexicon). The iteration ORDER/strategy in
// symbol_token (compound -> compare -> arithmetic -> range with `.rev()` ->
// symbol) is unchanged — only the data moved.
use crate::lexicon::{
    ARITHMETIC_SIGNS, BOOL, COMPARE_SIGNS, COMPOUND_SIGNS, RANGE_SIGNS,
    SYMBOL_OPERATORS as OPERATORS,
};

const TRAILING_SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+$").unwrap());
const COMMA_SEPARATORS: [&str; 2] = [",", ";"];
const BOM: u32 = 65279;
const RUST_KEYWORDS: &[&str] = &[
    // Control Flow Keywords
    "if",
    "else",
    "match",
    "loop",
    "while",
    "for",
    "break",
    "continue",
    "return",
    // Visibility and Access Modifiers
    "pub",
    "crate",
    "self",
    "super",
    "mod",
    // Declaration Keywords
    "let",
    "const",
    "static",
    "mut",
    // Types and Traits Keywords
    "struct",
    "enum",
    "union",
    "trait",
    "impl",
    "type",
    // Memory and Safety Control Keywords
    "unsafe",
    "async",
    "await",
    "move",
    "dyn",
    // Module Handling Keywords
    "extern",
    "use",
    // Function Definition and Implementation Keywords
    "fn",
    "self",
    "Self",
    // Data Manipulation Keywords
    "ref",
    "match",
    "in",
    "as",
    "Box",
    // Reliability and Testing Control Keywords
    "where",
    "macro",
    "macro_rules",
    "proc",
    // Result and Error Handling Keywords
    "Result",
    "Option",
    "Some",
    "None",
    "Ok",
    "Err",
    // Standard Data and System Types Keywords
    "bool",
    "char",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "isize",
    "usize",
    "f32",
    "f64",
    "str",
    // Property and Pattern Keywords
    "true",
    "false",
    // Contextual Keywords
    "abstract",
    "become",
    "box",
    "do",
    "final",
    "macro",
    "override",
    "priv",
    "typeof",
    "unsized",
    "virtual",
    "yield",
    "try",
];

const COPPER_KEYWORDS: &[&str] = &[
    // Function Definition and Implementation Keywords
    "func", "$init", "$child", // Module Handling Keywords
    "import", "from", // Class Keywords
    "class", "extends", // Struct and Impl Keywords
    "struct", "impl", "trait", "for", // Data Format Types
    "json", "xml", "toml", // Visibility (alias for `pub`)
    "public",
];

pub struct Tokenizer {
    source: String,
    index: usize,
    chunk: String,
    chunk_line: isize,
    chunk_column: usize,
    chunk_offset: usize,
    tokens: Vec<Token>,
    ends: Vec<EndToken>,
    seen_for: bool,
    seen_func: bool,
    seen_import: bool,
    seen_public: bool,
    import_specifier_list: bool,
    /// Tracks whether each currently-open `{` is the body of a `match`
    /// expression. Used by line_break_token to pick the right separator
    /// (`,` for match arms, `;` for everything else).
    brace_is_match: Vec<bool>,
    /// We saw a `match` keyword and are still scanning its head expression;
    /// the next `{` (at paren depth 0) opens the match body.
    expect_match_brace: bool,
    /// Tracks whether each currently-open `{` opens a **struct/enum literal**
    /// (`Vec2 { x: 1, y: 2 }`) rather than a code block. Inside one,
    /// line_break_token separates fields with `,` (not `;`), so a multi-line
    /// literal whose last field has no trailing comma stays valid.
    brace_is_struct: Vec<bool>,
    /// Set when the most recently tokenized `}` closed a **struct/enum
    /// literal** (`Point { x: 1, y: 2 }`). A struct literal in value position
    /// is an expression that DOES need a `;` terminator — unlike a block close
    /// (`if {...}`, fn body) which doesn't. `line_break_token` reads this to
    /// emit the `;` after `mut p = Point { ... }`. Cleared once any non-`}`,
    /// non-whitespace token is seen.
    last_closed_brace_was_struct: bool,
    /// We just emitted a control-flow keyword (`if`/`else`/`while`/`for`/
    /// `loop`); the next `{` opens its block, NOT a struct literal — even when
    /// the condition ends in an identifier (`if ready {`, `for x in xs {`).
    expect_block_brace: bool,
    /// Parenthesis / bracket depth since `expect_match_brace` was set, so
    /// `match foo({}) { ... }` doesn't mistake the inner `{}` for the match
    /// body.
    match_paren_depth: i32,
    location_data_compensations: Vec<usize>,
    /// Collected tokenizer errors. The CLI driver (`cforge`) checks this
    /// and exits non-zero; the LSP reads them as diagnostics. Either way,
    /// `tokenize()` itself never aborts the process — that would kill the
    /// language server on the first malformed edit.
    pub errors: Vec<String>,
}

impl Tokenizer {
    pub fn new(source: String) -> Self {
        // Strip `/* ... */` block comments before chunked tokenization.
        // The line-by-line scanner can't see across newlines, so the
        // simplest correct treatment is to remove block comments up front
        // while preserving each newline they contained — that keeps later
        // line/column reporting honest.
        let source = strip_block_comments(&source);

        let mut s = Self {
            source,
            index: 0,
            chunk: String::new(),
            chunk_line: 0,
            chunk_column: 0,
            chunk_offset: 0,
            tokens: Vec::new(),
            ends: Vec::new(),
            seen_for: false,
            seen_func: false,
            seen_import: false,
            seen_public: false,
            import_specifier_list: false,
            brace_is_match: vec![],
            expect_match_brace: false,
            brace_is_struct: vec![],
            last_closed_brace_was_struct: false,
            expect_block_brace: false,
            match_paren_depth: 0,
            location_data_compensations: vec![],
            errors: Vec::new(),
        };

        s.clean_source();

        s
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        while self.index < self.source.len() {
            // Find the end of current line
            let line_end = self.source[self.index..]
                .find('\n')
                .map(|pos| self.index + pos + 1)
                .unwrap_or(self.source.len());

            self.chunk = self.source[self.index..line_end].to_string();
            self.chunk_line += 1;
            self.chunk_column = 0;
            self.chunk_offset = self.index;

            while self.chunk_column < self.chunk.len() {
                let prev_col = self.chunk_column;
                let consumed = self
                    .identifier_token()
                    .or(|| self.number_token())
                    .or(|| self.string_token())
                    .or(|| self.comment_token())
                    .or(|| self.regex_token())
                    .or(|| self.operator_token())
                    .or(|| self.symbol_token())
                    .or(|| self.whitespace_token())
                    .or(|| self.line_break_token());

                if !consumed.is_consumed() {
                    self.next_char();
                }
                // Belt-and-suspenders: if a token method claimed it
                // consumed but didn't actually advance the chunk cursor,
                // force-advance one char rather than spin forever. This
                // is defensive — a properly-advancing token method
                // should make this branch unreachable.
                if self.chunk_column == prev_col {
                    self.next_char();
                }
            }

            // Move to next line
            self.index = line_end;
        }

        if !self.ends.is_empty() {
            let last = self.ends.last().unwrap();
            let location = last.origin.as_ref().unwrap().location_data.clone().unwrap();

            self.error(&format!(
                "Unterminated token {}\nOrigin: {}:{}",
                last.value, location.last_line, location.last_column
            ));
        }

        if self.kind() != Some(TokenKind::Newline) {
            self.token(TokenKind::Newline, ";\n".to_string());
        }

        self.token(TokenKind::Eof, "END_OF_FILE".to_string());
        self.tokens.clone()
    }

    pub fn operator_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let mut kind = TokenKind::Operator;

        for sign in COMPOUND_SIGNS.iter() {
            if self
                .chunk
                .get(self.chunk_column..)
                .unwrap_or_default()
                .starts_with(sign)
            {
                value.push_str(sign);
                consumed += sign.len();
                self.chunk_column += sign.len();
                break;
            }
        }

        if consumed == 0 {
            for sign in COMPARE_SIGNS.iter() {
                if self
                    .chunk
                    .get(self.chunk_column..)
                    .unwrap_or_default()
                    .starts_with(sign)
                {
                    value.push_str(sign);
                    consumed += sign.len();
                    self.chunk_column += sign.len();
                    break;
                }
            }
        }

        if consumed == 0 {
            for sign in ARITHMETIC_SIGNS.iter() {
                if self
                    .chunk
                    .get(self.chunk_column..)
                    .unwrap_or_default()
                    .starts_with(sign)
                {
                    value.push_str(sign);
                    consumed += sign.len();
                    self.chunk_column += sign.len();
                    break;
                }
            }
        }

        if consumed == 0 {
            for sign in RANGE_SIGNS.iter().rev() {
                if self
                    .chunk
                    .get(self.chunk_column..)
                    .unwrap_or_default()
                    .starts_with(sign)
                {
                    value.push_str(sign);
                    consumed += sign.len();
                    self.chunk_column += sign.len();
                    kind = TokenKind::Range;
                    break;
                }
            }
        }

        if consumed == 0 {
            for sign in OPERATORS.iter() {
                if self
                    .chunk
                    .get(self.chunk_column..)
                    .unwrap_or_default()
                    .starts_with(sign)
                {
                    if *sign == ":" && self.kind() == Some(TokenKind::Param) {
                        value.push(':');
                        consumed += sign.len() + 1;
                        self.chunk_column += sign.len() + 1;
                        kind = TokenKind::Colon;
                        break;
                    } else if *sign == "?" {
                        // Optional chaining: `?.` is an atomic operator that
                        // means "if the LHS is Some, run the chain on its
                        // inner value; otherwise short-circuit to None". A
                        // bare `?` (no following `.`) stays the Rust try
                        // operator.
                        let rest = self.chunk.get(self.chunk_column..).unwrap_or_default();
                        if rest.starts_with("?.") {
                            value.push_str("?.");
                            consumed += 2;
                            self.chunk_column += 2;
                            kind = TokenKind::OptionalChain;
                            break;
                        }
                    } else if *sign == "." {
                        kind = TokenKind::Dot;

                        if self.seen_import {
                            value.push_str(sign);
                            consumed += sign.len();
                            self.chunk_column += sign.len();
                            self.add_to_value(sign);

                            return Consumed::Consumed(consumed as isize);
                        }
                    }

                    value.push_str(sign);
                    consumed += sign.len();
                    self.chunk_column += sign.len();
                    break;
                }
            }
        }

        if consumed > 0 {
            self.token(kind, value);
        }

        Consumed::consume(consumed as isize)
    }

    pub fn symbol_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let mut kind = TokenKind::Symbol;
        // Set when this token is the `{`/`}` of a struct/enum literal; copied
        // onto the pushed token so the parser can coerce string fields (Bug B).
        let mut is_struct_brace = false;

        // `#[...]` Rust attribute — capture the full `#[...]` as one Attribute token
        // so the parser doesn't mis-parse `[...]` as a vec literal.
        if self.current_char() == '#' {
            let rest = &self.chunk[self.chunk_column..];
            if rest.starts_with("#[") {
                let mut depth = 0usize;
                let mut end = 0usize;
                for (i, c) in rest.char_indices() {
                    match c {
                        '[' => depth += 1,
                        ']' => {
                            depth -= 1;
                            if depth == 0 {
                                end = i + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if end > 0 {
                    let attr = rest[..end].to_string();
                    let len = attr.len();
                    self.chunk_column += len;
                    self.token(TokenKind::Attribute, attr);
                    return Consumed::Consumed(len as isize);
                }
            }
        }

        // Copper attributes symbol
        if self.current_char() == '$' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = TokenKind::CurrencySign;
        } else if self.current_char() == '<' || self.current_char() == '>' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = match value.as_str() {
                "<" => TokenKind::AngleStart,
                ">" => TokenKind::AngleEnd,
                _ => TokenKind::Symbol,
            };
        } else if self.current_char() == '(' && self.seen_func {
            // After `func`, `(` opens a tuple return type like `(i32, str)`.
            // Collect the full `(...)` as a single ReturnType token so the
            // parser doesn't mistake it for the start of the parameter list.
            self.seen_func = false;
            let rest = &self.chunk[self.chunk_column..];
            let mut depth = 0usize;
            let mut end = 0usize;
            for (i, c) in rest.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let type_str = rest[..end].to_string();
            let len = type_str.len();
            self.chunk_column += len;
            self.token(TokenKind::ReturnType, type_str);
            return Consumed::Consumed(len as isize);
        } else if self.current_char() == '(' || self.current_char() == ')' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = match value.as_str() {
                "(" => {
                    if self.expect_match_brace {
                        self.match_paren_depth += 1;
                    }
                    let last_sig = self.last_token();
                    let last_is_ident = last_sig.map(|t| t.kind) == Some(TokenKind::Identifier);
                    let last_is_angle_end = last_sig.map(|t| t.kind) == Some(TokenKind::AngleEnd);
                    let last_is_op_gt = last_sig
                        .map(|t| t.kind == TokenKind::Operator && t.value == ">")
                        .unwrap_or(false);
                    if last_is_ident || last_is_angle_end || last_is_op_gt {
                        self.end(TokenKind::ParametersEnd, ")".to_string());
                        TokenKind::ParenthesesStart
                    } else {
                        self.end(TokenKind::ParenthesesEnd, ")".to_string());
                        TokenKind::ParenthesesStart
                    }
                }
                ")" => {
                    if self.expect_match_brace && self.match_paren_depth > 0 {
                        self.match_paren_depth -= 1;
                    }
                    if self.end_kind() == Some(TokenKind::ParametersEnd) {
                        self.skip_end();
                        TokenKind::ParametersEnd
                    } else {
                        self.skip_end();
                        TokenKind::ParenthesesEnd
                    }
                }
                _ => TokenKind::Symbol,
            };
        } else if self.current_char() == '[' || self.current_char() == ']' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = match value.as_str() {
                "[" => {
                    if self.expect_match_brace {
                        self.match_paren_depth += 1;
                    }
                    self.end(TokenKind::BracketEnd, "]".to_string());
                    TokenKind::BracketStart
                }
                "]" => {
                    if self.expect_match_brace && self.match_paren_depth > 0 {
                        self.match_paren_depth -= 1;
                    }
                    self.skip_end();
                    TokenKind::BracketEnd
                }
                _ => TokenKind::Symbol,
            };
        } else if self.current_char() == '{' || self.current_char() == '}' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = match value.as_str() {
                "{" => {
                    if self.seen_import && self.value(true) == Some("import".to_string()) {
                        self.import_specifier_list = true;
                    }

                    // Track whether this `{` opens the body of a `match`.
                    // Only count when we're not inside the matched
                    // expression's parens (paren depth 0).
                    let opens_match_body = self.expect_match_brace && self.match_paren_depth == 0;
                    self.brace_is_match.push(opens_match_body);
                    if opens_match_body {
                        self.expect_match_brace = false;
                    }

                    // A `{` opens a struct/enum literal when it directly follows
                    // a type name (`Vec2 {`) — i.e. the previous token is an
                    // identifier or a closing generic `>` — and it isn't a
                    // control-flow block (`if x {`, `for i in xs {`) or a match
                    // body. The `expect_block_brace` flag disambiguates the
                    // `Identifier {` case where the identifier is a condition.
                    // A trailing `>` can close a generic type (`Vec<T> {`) OR be
                    // the tail of a `=>` fat arrow (a match arm body `=> {`).
                    // Only the former is a struct literal; the latter opens a
                    // normal block whose statements need `;`. `=>` lexes as two
                    // tokens (`=` then `>`), so detect the fat arrow by looking
                    // one token further back.
                    let prev_gt_closes_generic = matches!(
                        self.last_token().map(|t| (t.kind, t.value.clone())),
                        Some((TokenKind::Operator, v)) if v == ">"
                    ) && self.second_last_nonws_value().as_deref() != Some("=");
                    let prev_is_type_name = matches!(
                        self.last_token().map(|t| (t.kind, t.value.clone())),
                        Some((TokenKind::Identifier, _)) | Some((TokenKind::ReturnType, _))
                    ) || prev_gt_closes_generic;
                    let opens_struct =
                        prev_is_type_name && !opens_match_body && !self.expect_block_brace;
                    self.brace_is_struct.push(opens_struct);
                    is_struct_brace = opens_struct;
                    self.expect_block_brace = false;

                    self.end(TokenKind::BraceEnd, "}".to_string());
                    TokenKind::BraceStart
                }
                "}" => {
                    if self.import_specifier_list {
                        self.import_specifier_list = false;
                    }
                    self.brace_is_match.pop();
                    // Remember whether this `}` closed a struct literal: a
                    // struct literal in value position is an expression and
                    // needs a `;` after it (handled in line_break_token).
                    self.last_closed_brace_was_struct =
                        matches!(self.brace_is_struct.pop(), Some(true));
                    is_struct_brace = self.last_closed_brace_was_struct;

                    self.skip_end();
                    TokenKind::BraceEnd
                }
                _ => TokenKind::Symbol,
            };
        } else if COMMA_SEPARATORS.contains(&self.current_char().to_string().as_str()) {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = match value.as_str() {
                "," => TokenKind::Comma,
                ";" => {
                    if self.peek() == '\n' {
                        self.next_char();
                        consumed += 1;

                        if self.seen_import {
                            self.set_kind(TokenKind::ModulePath);
                            self.seen_import = false;
                        }

                        value.push('\n');

                        TokenKind::Newline
                    } else {
                        TokenKind::Semicolon
                    }
                }
                _ => TokenKind::Symbol,
            };
        } else if self.current_char() == '.' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = TokenKind::Dot;
        }

        // Only stamp `struct_brace` when we actually produced a brace token.
        // `symbol_token` is also called speculatively for non-symbol chars and
        // ends with a zero-length `self.token(Symbol, "")`, which (length 0)
        // returns the PREVIOUS token instead of pushing — writing here would
        // clobber a real struct-brace flag on whatever token came before.
        if matches!(kind, TokenKind::BraceStart | TokenKind::BraceEnd) {
            self.token(kind, value).struct_brace = is_struct_brace;
        } else {
            self.token(kind, value);
        }

        Consumed::consume(consumed)
    }

    pub fn string_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::String;
        let mut escape = false;

        if self.current_char() == '"' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            while self.current_char() != '"' || escape {
                if self.current_char() == '\\' {
                    escape = !escape;
                } else {
                    escape = false;
                }

                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }

            value.push(self.current_char());
            self.next_char();
            consumed += 1;
        }

        // Detect `$ident` / `${expr}` interpolation. If present, swap the
        // token's kind/value/data so downstream code can render either as a
        // `format!(...)` expression or as raw macro args, depending on
        // context.
        if let Some(interp) = super::interpolation::parse(&value) {
            let rendered = super::interpolation::render_format_call(&interp);
            let token = self.token(TokenKind::InterpolatedString, rendered);
            token.data = Data::Interpolation {
                placeholder: interp.placeholder,
                args: interp.args,
            };
        } else {
            self.token(kind, value);
        }

        Consumed::consume(consumed)
    }

    pub fn number_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::Number;

        if self.current_char().is_numeric() {
            while self.current_char().is_numeric() {
                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }

            if self.current_char() == '.' {
                if self.peek().is_numeric() {
                    value.push(self.current_char());
                    self.next_char();
                    consumed += 1;

                    while self.current_char().is_numeric() {
                        value.push(self.current_char());
                        self.next_char();
                        consumed += 1;
                    }
                } else if self.peek() != '.' {
                    self.error(&format!("Invalid float or range: {}", self.peek()));
                }
            }
        }

        self.token(kind, value);

        Consumed::consume(consumed)
    }

    pub fn identifier_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let mut kind = TokenKind::Unknown;

        if self.current_char() == '\'' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            // Continue capturing the lifetime name
            while self.current_char().is_alphanumeric() || self.current_char() == '_' {
                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }

            kind = TokenKind::Lifetime;
        } else if self.current_char().is_alphabetic() || self.current_char() == '_' {
            while self.current_char().is_alphanumeric() || self.current_char() == '_' {
                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }

            if BOOL.contains(&value.as_str()) {
                kind = TokenKind::Keyword;
            } else if COPPER_KEYWORDS.contains(&value.as_str())
                // `from` is contextual: it is only the import keyword while an
                // import statement is in progress. Anywhere else (`From` trait,
                // `X::from`, `.from(`, a variable named `from`) it must fall
                // through to the normal identifier-resolution path below.
                && (value != "from" || self.seen_import)
                // `json`/`xml`/`toml` are data-type keywords only in type
                // position. As a method/field name (`resp.json(...)`) or a
                // function name / call (`func String json(...)`, `json(...)`)
                // they must be plain identifiers — fall through like `from`.
                && !(matches!(value.as_str(), "json" | "xml" | "toml")
                    && (matches!(self.last_token().map(|t| t.kind), Some(TokenKind::Dot))
                        || self.current_char() == '('))
            {
                match value.as_str() {
                    "import" => {
                        self.seen_import = true;
                        kind = TokenKind::Import;
                    }
                    "from" => {
                        kind = TokenKind::From;
                    }
                    "as" => {
                        kind = TokenKind::As;
                    }
                    "public" => {
                        // `public` is an alias for `pub`. Normalize the token so
                        // the whole pipeline (dispatch + `pub func` handling)
                        // treats `public` and `pub` identically.
                        self.seen_public = true;
                        value = "pub".to_string();
                        kind = TokenKind::Keyword;
                    }
                    "for" => {
                        self.seen_for = true;
                        kind = TokenKind::For;
                        // The next `{` opens the loop body, not a struct literal,
                        // even though `for x in items {` ends in an identifier.
                        self.expect_block_brace = true;
                    }
                    "struct" => {
                        kind = TokenKind::Struct;
                    }
                    "impl" => {
                        kind = TokenKind::Impl;
                    }
                    "trait" => {
                        kind = TokenKind::Trait;
                    }
                    "json" => {
                        kind = TokenKind::Json;
                    }
                    "xml" => {
                        kind = TokenKind::Xml;
                    }
                    "toml" => {
                        kind = TokenKind::Toml;
                    }
                    "func" => {
                        self.seen_func = true;
                        kind = TokenKind::Keyword;
                    }
                    _ => {
                        kind = TokenKind::Keyword;
                    }
                }
            } else if self.seen_func
                && self
                    .peek_significant_char()
                    .map(|c| c == '(')
                    .unwrap_or(false)
            {
                // `func name(...)` with no declared return type: the first
                // identifier after `func` is immediately followed by `(`, so
                // it's the function NAME (void return), not a return type.
                // Without this the void `main` (and any void function) would
                // be mislabeled as a return type, leaving the function nameless
                // (`fn (...)`). Emit it as a plain Identifier — the parser's
                // name handling (incl. the `main` → `__copper_main` rename)
                // then applies.
                self.seen_func = false;
                kind = TokenKind::Identifier;
            } else if self.seen_func {
                // First identifier-like token after `func` is the declared
                // return type. Win against RUST_KEYWORDS so things like
                // `func Result<T, E> name(...)` route the whole `Result<...>`
                // to the parser's ReturnType handler.
                self.seen_func = false;
                // Optional-return sugar: `func Type? name(...)`. Fold a
                // trailing `?` into the ReturnType token (mirrors the ParamType
                // path below) so `convert_type` lowers `Type?` → `Option<Type>`.
                // Generic returns (`Result<T, E>`) keep the `?`-less base name;
                // their `<...>` is still gobbled by the parser's ReturnType arm.
                if self.current_char() == '?' {
                    self.next_char();
                    value.push('?');
                }
                kind = TokenKind::ReturnType;
            } else if RUST_KEYWORDS.contains(&value.as_str()) {
                // Specialise loop-related keywords so the parser can route
                // them through the keyword-spacing path without re-checking
                // strings.
                kind = match value.as_str() {
                    "loop" => TokenKind::Loop,
                    "while" => TokenKind::While,
                    "break" => TokenKind::Break,
                    "continue" => TokenKind::Continue,
                    "in" => TokenKind::In,
                    _ => TokenKind::Keyword,
                };
                if value == "match" {
                    // Arm the brace tracker: the next `{` we open while not
                    // inside parens belongs to this match expression.
                    self.expect_match_brace = true;
                    self.match_paren_depth = 0;
                }
                // Control-flow keywords open a block (not a struct literal) at
                // the next `{`, even when the condition ends in an identifier.
                if matches!(value.as_str(), "if" | "else" | "while" | "for" | "loop") {
                    self.expect_block_brace = true;
                }
            } else if self.import_specifier_list {
                kind = TokenKind::ModuleVar;
            } else if self.seen_import {
                if self.value(true) == Some("from".to_string()) {
                    kind = TokenKind::ModulePath;
                } else if self.value(true) != Some(" ".to_string()) {
                    self.add_to_value(&value);
                    return Consumed::Consumed(consumed);
                } else {
                    kind = TokenKind::ModuleVar;
                }
            } else {
                match self.last_token() {
                    Some(token) if token.value == "func" => kind = TokenKind::ReturnType,
                    Some(_) if self.end_kind() == Some(TokenKind::ParametersEnd) => {
                        if self.value(true) == Some(":".to_string()) {
                            if self.current_char() == '?' {
                                self.next_char();
                                value.push('?');
                            }

                            kind = TokenKind::ParamType
                        } else {
                            kind = TokenKind::Param
                        }
                    }
                    _ => kind = TokenKind::Identifier,
                }
            }
        }

        if consumed > 0 {
            self.token(kind, value);
        }

        Consumed::consume(consumed)
    }

    pub fn whitespace_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::Whitespace;

        if self.current_char() == ' ' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            while self.current_char() == ' ' {
                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }
        }

        self.token(kind, value);

        Consumed::consume(consumed)
    }

    pub fn line_break_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::Newline;

        if self.current_char() == '\n' && self.kind() != Some(TokenKind::Newline) {
            // A line ending in a binary/infix operator (`a &&`, `x +`, `obj.`)
            // or `::` is a *continuation*, not a statement end — appending `;`
            // would produce `a &&;`. Postfix operators (`?`, `++`, `--`) DO end
            // a statement, so they keep the `;`.
            let ends_with_continuation = match self.last_token() {
                Some(t) if t.kind == TokenKind::Dot => true,
                Some(t) if t.kind == TokenKind::Operator => {
                    // Postfix `++` / `--` end a statement (keep the `;`). The
                    // tokenizer emits them as two single-char `+`/`-` operator
                    // tokens, so the last token alone looks like a binary `+`
                    // (a continuation). Detect the pair so `count++\n if ...`
                    // gets its terminator instead of fusing into the next line.
                    let is_postfix_pair = (t.value == "+" || t.value == "-")
                        && self.second_last_nonws_value().as_deref()
                            == Some(t.value.as_str());
                    !is_postfix_pair && !matches!(t.value.as_str(), "?" | "++" | "--")
                }
                _ => false,
            };
            match self.kind() {
                _ if ends_with_continuation => {
                    value.push(self.current_char());
                }
                // A `}` that closed a struct literal in value position
                // (`mut p = Point { x: 1, y: 2 }`) is an EXPRESSION and needs a
                // terminator, unlike a block close (`if {...}`, fn body). Pick
                // `,` when that literal is itself a field value inside an
                // enclosing match arm / struct literal, else `;`.
                Some(TokenKind::BraceEnd) if self.last_closed_brace_was_struct => {
                    let separator = if matches!(self.brace_is_match.last(), Some(&true))
                        || matches!(self.brace_is_struct.last(), Some(&true))
                    {
                        ","
                    } else {
                        ";"
                    };
                    value.push_str(&format!("{}{}", separator, self.current_char()));
                }
                Some(TokenKind::BraceStart) |
                Some(TokenKind::BracketStart) |
                Some(TokenKind::ParenthesesStart) |
                Some(TokenKind::ParametersStart) |
                Some(TokenKind::BraceEnd) |
                // `,` already separates items (function args, match arms,
                // collection literals); appending a `;` would produce `,;`
                // which Rust rejects inside match blocks.
                Some(TokenKind::Comma) => {
                    value.push(self.current_char());
                },
                _ => {
                    // Inside a `match` body, line endings between arms must
                    // be `,` not `;` — `_ => 2;` is a parse error in Rust,
                    // while `_ => 2,` is correct (and a trailing comma is
                    // fine right before `}`). The same holds for the fields of
                    // a multi-line struct literal (`Vec2 {\n x: 1,\n y: 2\n}`):
                    // a `;` after the last field would break it, a `,` is fine.
                    let separator = if matches!(self.brace_is_match.last(), Some(&true))
                        || matches!(self.brace_is_struct.last(), Some(&true))
                    {
                        ","
                    } else {
                        ";"
                    };
                    value.push_str(&format!("{}{}", separator, self.current_char()));
                }
            }
            self.next_char();
            consumed += 1;

            if self.seen_import {
                self.seen_import = false;
                self.set_kind(TokenKind::ModulePath);
            }
        } else if self.current_char() == '\n' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;
        }

        self.token(kind, value);

        Consumed::consume(consumed)
    }

    pub fn comment_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();

        if self.current_char() == '/' && self.peek() == '/' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            // Doc comment (like Rust): `///` (outer) or `//!` (inner), but NOT
            // `////` (which is a plain comment). Doc comments are preserved
            // through transpilation and surfaced by the LSP; `//` is dropped.
            let is_doc =
                (self.current_char() == '/' && self.peek() != '/') || self.current_char() == '!';
            let kind = if is_doc {
                TokenKind::DocComment
            } else {
                TokenKind::Comment
            };

            while self.current_char() != '\n' && self.current_char() != '\0' {
                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }

            self.token(kind, value);
        }

        Consumed::consume(consumed)
    }

    pub fn regex_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::Regex;

        // Only consider regex if it starts with '/'
        if self.current_char() != '/' {
            return Consumed::consume(0);
        }

        // Check if it's not a comment (//)
        if self.peek() == '/' {
            return Consumed::consume(0);
        }

        // Division vs. regex disambiguation (JS-style): a `/` is the DIVISION
        // operator when it follows an operand — an identifier, number, string,
        // or a closing `)` / `]`. A regex literal `/.../ ` is only recognised
        // when a value is expected (start of expression: after `=`, `(`, `,`,
        // an operator, `return`, etc.). Without this, `a / b` lexed `/ b /...`
        // as a regex and arithmetic division was broken.
        let last_significant = self.tokens.iter().rev().find(|t| {
            !matches!(
                t.kind,
                TokenKind::Whitespace
                    | TokenKind::Newline
                    | TokenKind::Comment
                    | TokenKind::DocComment
            )
        });
        if let Some(prev) = last_significant {
            if matches!(
                prev.kind,
                TokenKind::Identifier
                    | TokenKind::Number
                    | TokenKind::String
                    | TokenKind::InterpolatedString
                    | TokenKind::ParenthesesEnd
                    | TokenKind::BracketEnd
            ) {
                // Division — let operator_token handle the `/`.
                return Consumed::consume(0);
            }
        }

        // Process only from current position in chunk. Cloned so we can
        // re-borrow self mutably for `self.error(...)` if the regex is
        // unterminated.
        let remaining_chunk: String = self.chunk[self.chunk_column..].to_string();

        // Check again in chunk if it's a comment
        if remaining_chunk.starts_with("//") {
            return Consumed::consume(0);
        }

        let r = Regex::new(r"^(/)([^/]+)(/)?").unwrap(); // Added ^ for string start

        if let Some(cap) = r.captures(&remaining_chunk) {
            let start = cap.get(1).unwrap().start();
            let end_cap = cap.get(3);

            if end_cap.is_none() {
                self.error("Unterminated regex literal");
                return Consumed::consume(0);
            }

            let end = end_cap.unwrap().end();
            value.push_str(&remaining_chunk[start..end]);
            consumed = end - start;

            // Advance chunk_column instead of draining
            for _ in 0..consumed {
                self.next_char();
            }
        }

        self.token(kind, value);

        Consumed::consume(consumed as isize)
    }

    pub fn error(&mut self, message: &str) {
        // Collect errors instead of aborting the process. The cforge CLI
        // surfaces these and exits; the LSP turns them into diagnostics.
        let (line, column, _) = self.get_line_and_column(self.chunk_column);
        let formatted = format!("{}\nLine: {}:{}", message, line + 1, column + 1);
        self.errors.push(formatted);
    }

    pub fn value(&self, use_origin: bool) -> Option<String> {
        let token = self.tokens.last();

        token?;

        let token = token.unwrap();
        let value = token.value.clone();
        let origin = token.origin.clone();

        if use_origin && origin.is_some() {
            return Some(origin.unwrap().value.clone());
        }

        Some(value)
    }

    pub fn add_to_value(&mut self, value: &str) {
        if self.last_token().is_none() {
            return;
        }

        let token = self.tokens.last_mut().unwrap();
        token.value.push_str(value);
    }

    pub fn last_token(&self) -> Option<&Token> {
        self.tokens
            .iter()
            .rfind(|t| t.kind != TokenKind::Whitespace)
    }

    /// Value of the second-to-last non-whitespace token, if any. Used to
    /// detect postfix `++` / `--`, which the tokenizer emits as two
    /// single-char `+` / `-` operators rather than one token.
    fn second_last_nonws_value(&self) -> Option<String> {
        self.tokens
            .iter()
            .rev()
            .filter(|t| t.kind != TokenKind::Whitespace)
            .nth(1)
            .map(|t| t.value.clone())
    }

    pub fn token(&mut self, kind: TokenKind, value: String) -> &mut Token {
        let length = value.len();

        // If the length is 0, we want to modify the last token
        if length == 0 {
            if self.tokens.is_empty() {
                return self.token(TokenKind::Unknown, " ".to_string());
            }

            return self.tokens.last_mut().unwrap();
        }

        // The `last_closed_brace_was_struct` flag is set when a `}` closes a
        // struct literal and read by the very next `line_break_token`. Keep it
        // alive across the closing `}` itself, intervening whitespace, and the
        // terminating newline; clear it as soon as any other token appears so a
        // later, unrelated `}`-less statement doesn't pick up a stale `;`.
        if !matches!(
            kind,
            TokenKind::BraceEnd | TokenKind::Newline | TokenKind::Whitespace
        ) {
            self.last_closed_brace_was_struct = false;
        }

        let mut token = Token::new(kind, value, length, Data::None, false);
        token.set_location_data(self.create_location_data(self.chunk_offset, length));

        match kind {
            TokenKind::BraceStart
            | TokenKind::BracketStart
            | TokenKind::ParametersStart
            | TokenKind::ParenthesesStart => {
                self.last_end().unwrap().set_origin(token.clone());
            }
            _ => {}
        }

        self.tokens.push(token.clone());
        self.tokens.last_mut().unwrap()
    }

    pub fn skip_end(&mut self) {
        if self.ends.is_empty() {
            return;
        }

        self.ends.remove(self.ends.len() - 1);
    }

    pub fn last_end(&mut self) -> Option<&mut Token> {
        self.ends.last_mut()
    }

    pub fn end(&mut self, kind: TokenKind, value: String) -> &mut EndToken {
        self.ends.push(EndToken::new_end(
            kind,
            value.clone(),
            value.len(),
            Data::None,
        ));

        self.ends.last_mut().unwrap()
    }

    pub fn set_kind(&mut self, kind: TokenKind) {
        let token = self.tokens.last_mut();

        if token.is_some() {
            token.unwrap().kind = kind;
        }
    }

    pub fn kind(&self) -> Option<TokenKind> {
        let token = &self.tokens.last();

        if token.is_some() {
            return Some(token.unwrap().clone().kind);
        }

        None
    }

    pub fn end_kind(&self) -> Option<TokenKind> {
        let token = &self.ends.last();

        if token.is_some() {
            return Some(token.unwrap().clone().kind);
        }

        None
    }

    pub fn create_location_data(&self, offset_in_chunk: usize, length: usize) -> LocationData {
        let last_char = if length > 0 { length - 1 } else { 0 };

        let (first_line, first_column, range_start) = self.get_line_and_column(offset_in_chunk);
        let (last_line, last_column, end_offset) =
            self.get_line_and_column(offset_in_chunk + last_char);

        let range = (
            range_start,
            if length > 0 {
                end_offset + 1
            } else {
                end_offset
            },
        );

        LocationData {
            first_line,
            first_column,
            last_line,
            last_column,
            range,
        }
    }

    pub fn get_location_data_compensation(&self, start: usize, mut end: usize) -> usize {
        let mut total_compensation = 0;
        let initial_end = end;
        let mut current = start;
        let mut iterations = 0;

        while start >= end && iterations < 1000 {
            iterations += 1;

            if current == end && start != initial_end {
                break;
            }

            let compensation = self.location_data_compensations.get(current);

            if compensation.is_some() {
                let compensation = compensation.unwrap();
                total_compensation += compensation;
                end += compensation;
            }

            current += 1;

            // Additional safety condition
            if current >= self.source.len() + 1000 {
                break;
            }
        }

        total_compensation
    }

    #[allow(unused_assignments)]
    pub fn get_line_and_column(&self, offset: usize) -> (isize, usize, usize) {
        let compensation =
            self.get_location_data_compensation(self.chunk_offset, self.chunk_offset + offset);
        let mut s = String::new();
        let mut line_count: usize = 0;
        let mut column = 0;
        let mut previous_lines_compensation = 0;
        let mut column_compensation = 0;

        if offset == 0 {
            return (
                self.chunk_line,
                self.chunk_column + compensation,
                self.chunk_offset + compensation,
            );
        }

        if offset >= self.chunk.len() {
            s = self.chunk.clone();
        } else {
            let end = match offset {
                o if o > 0 => o,
                _ => self.chunk.len(),
            };

            // Floor `end` to the nearest valid UTF-8 char boundary so the slice
            // never panics on source with multi-byte characters.
            let raw_end = end.min(self.chunk.len());
            let safe_end = (0..=raw_end)
                .rev()
                .find(|&k| self.chunk.is_char_boundary(k))
                .unwrap_or(0);
            s = self.chunk[0..safe_end].to_owned();
        }

        line_count = self.count_occurrences(&s, "\n");
        column = self.chunk_column;

        if line_count > 0 {
            let r = s.split("\n").collect::<Vec<&str>>();
            column = r.last().unwrap().len();
            previous_lines_compensation = self.get_location_data_compensation(
                self.chunk_offset,
                self.chunk_offset + offset - column,
            );

            column_compensation = self.get_location_data_compensation(
                self.chunk_offset + offset - previous_lines_compensation - column,
                self.chunk_offset + offset + previous_lines_compensation,
            );
        } else {
            column += s.len();
            column_compensation = compensation;
        }

        (
            self.chunk_line + line_count as isize,
            column + column_compensation,
            self.chunk_offset + offset + compensation,
        )
    }

    pub fn count_occurrences(&self, string: &str, substr: &str) -> usize {
        if substr.is_empty() {
            return usize::MAX; // Use usize::MAX to represent infinity
        }
        let mut num = 0;
        let mut pos = 0;

        while let Some(p) = string[pos..].find(substr) {
            num += 1;
            pos += p + 1; // Increment position to continue search
        }

        num
    }

    // pub fn next_chunk(&self) -> String {
    //     self.source.get(self.index..).unwrap_or_default().to_owned()
    // }

    pub fn clean_source(&mut self) {
        let re = Regex::new(r"\r").unwrap();
        let mut thus_far = 0;
        let mut source = &self.source;

        if !source.is_empty() && source.chars().nth(0).unwrap() as u32 == BOM {
            self.source = source.chars().skip(1).collect::<String>();
            source = &self.source;
            self.location_data_compensations[0] = 1;
            thus_far += 1;
        }

        if TRAILING_SPACES.is_match(source) {
            self.source = TRAILING_SPACES.replace_all(source, "").to_string();
            source = &self.source;
            self.chunk_line -= 1;
            self.location_data_compensations.insert(
                0,
                self.location_data_compensations.first().unwrap_or(&1) - 1,
            );
        }

        for mat in re.find_iter(source) {
            let offset = mat.start();

            while self.location_data_compensations.len() < (thus_far + offset) + 1 {
                self.location_data_compensations.push(0);
            }

            self.location_data_compensations[thus_far + offset] = 1;
            thus_far += offset;
        }

        self.source = re.replace_all(source, "").to_string();
    }

    fn current_char(&self) -> char {
        // Use proper UTF-8 indexing.
        if self.chunk_column >= self.chunk.len() {
            // Past end of chunk — return NUL so identifier/number loops
            // terminate. Earlier code returned `chunk[0]` here, which made
            // a source ending in `[A-Za-z_]` re-enter the identifier
            // scanner forever (`current_char()` kept yielding the file's
            // first char while `next_char()` clamped at `chunk.len()`).
            return '\0';
        }
        if let Some((_, ch)) = self
            .chunk
            .char_indices()
            .find(|(pos, _)| *pos == self.chunk_column)
        {
            ch
        } else {
            // Misaligned UTF-8 position (shouldn't normally happen).
            '\0'
        }
    }

    /// Peek the next non-space/tab character from the current chunk position
    /// without advancing. Used to disambiguate `func name(...)` (void return)
    /// from `func Type name(...)`: if a `(` immediately follows the first
    /// identifier after `func`, that identifier is the function name.
    fn peek_significant_char(&self) -> Option<char> {
        self.chunk
            .char_indices()
            .filter(|(pos, _)| *pos >= self.chunk_column)
            .map(|(_, ch)| ch)
            .find(|ch| *ch != ' ' && *ch != '\t')
    }

    fn next_char(&mut self) {
        // Find the next character boundary
        if let Some((next_pos, _)) = self
            .chunk
            .char_indices()
            .find(|(pos, _)| *pos > self.chunk_column)
        {
            self.chunk_column = next_pos;
        } else {
            // End of string
            self.chunk_column = self.chunk.len();
        }
    }

    fn peek(&self) -> char {
        self.chunk
            .chars()
            .nth(self.chunk_column + 1)
            .unwrap_or_default()
    }
}
