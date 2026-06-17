use copper_syntax::tokenizer::{
    interpolation,
    kind::TokenKind,
    tokenizer::Tokenizer,
    tokens::{Data, Token},
};
use std::vec;

/// The Copper source for the embedded `cstd` standard library, baked into
/// the compiler at build time. Updates to `std/cstd.crs` ship with the
/// next `cargo build`.
const CSTD_SOURCE: &str = include_str!("../../../../std/cstd.crs");

/// Native-Rust helpers for things Copper cannot yet express cleanly
/// (multi-line method chains, `&[T]`, `cfg!(target_os=...)`).
const CSTD_NATIVE: &str = include_str!("../../../../std/cstd_native.rs");

// Additional native std modules, bundled on demand when imported
// (`import { ... } from net` / `from http`). Each has a Copper-written
// `.crs` surface plus a native `.rs` of helpers.
const NET_SOURCE: &str = include_str!("../../../../std/net.crs");
const NET_NATIVE: &str = include_str!("../../../../std/net_native.rs");
const HTTP_SOURCE: &str = include_str!("../../../../std/http.crs");
const HTTP_NATIVE: &str = include_str!("../../../../std/http_native.rs");
const URL_SOURCE: &str = include_str!("../../../../std/url.crs");
const URL_NATIVE: &str = include_str!("../../../../std/url_native.rs");
const JSON_SOURCE: &str = include_str!("../../../../std/json.crs");
const JSON_NATIVE: &str = include_str!("../../../../std/json_native.rs");
const CRYPTO_SOURCE: &str = include_str!("../../../../std/crypto.crs");
const CRYPTO_NATIVE: &str = include_str!("../../../../std/crypto_native.rs");
const TIME_SOURCE: &str = include_str!("../../../../std/time.crs");
const TIME_NATIVE: &str = include_str!("../../../../std/time_native.rs");
const FS_SOURCE: &str = include_str!("../../../../std/fs.crs");
const FS_NATIVE: &str = include_str!("../../../../std/fs_native.rs");
const WS_SOURCE: &str = include_str!("../../../../std/ws.crs");
const WS_NATIVE: &str = include_str!("../../../../std/ws_native.rs");
const REFLECT_SOURCE: &str = include_str!("../../../../std/reflect.crs");
const REFLECT_NATIVE: &str = include_str!("../../../../std/reflect_native.rs");
pub mod result;
pub mod scope;
pub mod scope_manager;
mod ternary;
pub mod utils;

use copper_syntax::utils::Consumed;
use copper_syntax::{ConsumeVar, ConsumedTrait};
use result::Result;
use utils::convert_type;

const COPPER_OPERATORS: [(&str, &str); 2] = [("++", "+= 1"), ("--", "-= 1")];

const RUST_MACROS: [(&str, &str); 1] = [("println", "println!")];

#[derive(Debug)]
pub enum AppendMode {
    Append,
    AppendToMainFunction,
    ForceAppend,
    FFAppend,
    AppendWithSpace,
    AppendToMainFunctionWithSpace,
    ForceAppendWithSpace,
    FFAppendWithSpace,
}

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
    result: Result,
    eof: bool,
    function_start: bool,
    seen_import: bool,
    current_import_vars: Vec<String>,
    is_import_list: bool,
    is_inside_class: bool,
    current_class: Option<String>,
    is_inside_struct: bool,
    /// One entry per currently-open `{` in the main token stream. `Some(d)`
    /// when that `{` opens a **struct/enum literal** (`Point { x: 1, y: 2 }`),
    /// where `d` is the `chain_delim_depth` just inside it; `None` for a code
    /// block. A string literal whose enclosing struct-literal entry is `Some(d)`
    /// AND whose current depth is exactly `d` (i.e. it's a direct field value,
    /// not buried in a nested call/array) is emitted as `"..".into()` so it
    /// coerces to a `String` field (Bug B). Mirrors the tokenizer's
    /// `brace_is_struct`.
    struct_lit_stack: Vec<Option<usize>>,
    is_inside_impl: bool,
    current_struct: Option<String>,
    current_impl_target: Option<String>,
    uses_data_types: bool,
    /// Brace nesting *inside the current function body*, counted from the
    /// `{` that opened it. Used so `parse_function_body` only treats the
    /// matching outer `}` as the end of the function — inner blocks
    /// (`match`, `if`, nested scopes) no longer trip an early exit.
    function_brace_depth: usize,
    /// Paren / bracket depth used to scope optional chaining (`?.`). The
    /// parser opens a chain at the depth where `?.` appears and closes it
    /// (emits the matching `)`) when the depth drops back below that or a
    /// chain-breaking token is hit at the same depth.
    chain_delim_depth: usize,
    /// One entry per currently-open optional chain, holding the
    /// `chain_delim_depth` at which it was opened. Pushed on `?.`, popped
    /// when the chain closes.
    optional_chain_depths: Vec<usize>,
    /// Counter feeding the synthetic closure variable name (`__copt0`,
    /// `__copt1`, ...) so nested chains don't collide.
    optional_chain_counter: usize,
    /// Set when we see `unsafe func ...`: the `unsafe` is consumed
    /// silently and `parse_function` emits `unsafe fn` instead of `fn`.
    pending_unsafe_fn: bool,
    /// Set when we see `pub <item> ...` (or `public <item> ...`, normalized to
    /// `pub`) for a `func` or `struct`: the `pub` is consumed silently and the
    /// item's emitter prepends `pub `, so the visibility lands on the item
    /// instead of leaking onto the next statement (`pub let x = ...`).
    pending_pub: bool,
    pending_async_fn: bool,
}

/// Drop the `;` from a statement-terminating newline when the next
/// significant token is a `.` — i.e. a method-chain continuation written
/// across lines:
///
/// ```text
/// result = vec![1, 2, 3]
///     .iter()
///     .sum()
/// ```
///
/// The tokenizer already suppresses the separator when a line *ends* with a
/// continuation token (`.`, `&&`, `::`, …); this handles the common form
/// where the line ends with `)`/`]` and the *next* line opens with `.`.
fn join_chain_continuations(tokens: Vec<Token>) -> Vec<Token> {
    let next_significant_is_dot = |from: usize| -> bool {
        let mut j = from;
        while j < tokens.len() {
            match tokens[j].kind {
                TokenKind::Newline | TokenKind::Comment | TokenKind::DocComment => j += 1,
                TokenKind::Dot => return true,
                _ => return false,
            }
        }
        false
    };
    let mut out = tokens.clone();
    for i in 0..out.len() {
        if out[i].kind == TokenKind::Newline
            && out[i].value.contains(';')
            && next_significant_is_dot(i + 1)
        {
            // Keep the newline (formatting) but strip the separator so the
            // expression continues onto the chained call.
            out[i].value = out[i].value.replace(';', "");
        }
    }
    out
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        let filtered: Vec<Token> = tokens
            .into_iter()
            .filter(|t| t.kind != TokenKind::Whitespace && t.kind != TokenKind::Comment)
            .collect();
        // Join multi-line method chains: a statement-terminating newline
        // whose next significant token is `.` is a chain continuation
        // (`x\n  .iter()\n  .sum()`), not a statement end — drop its `;`
        // so the chain lowers as one expression instead of three broken
        // statements. The tokenizer only suppresses the `;` when a line
        // *ends* with `.`/operator; this catches the leading-dot form.
        let joined = join_chain_continuations(filtered);
        // Lower `cond ? then : else` ternaries into `if cond { then } else { else }`
        // before the main parser dispatch sees them.
        let lowered = ternary::rewrite(joined);
        Self {
            tokens: lowered,
            current: 0,
            result: Result::new(),
            eof: false,
            function_start: false,
            seen_import: false,
            current_import_vars: vec![],
            is_import_list: false,
            is_inside_class: false,
            current_class: None,
            is_inside_struct: false,
            struct_lit_stack: vec![],
            is_inside_impl: false,
            current_struct: None,
            current_impl_target: None,
            uses_data_types: false,
            function_brace_depth: 0,
            chain_delim_depth: 0,
            optional_chain_depths: vec![],
            optional_chain_counter: 0,
            pending_unsafe_fn: false,
            pending_pub: false,
            pending_async_fn: false,
        }
    }

    pub fn current(&self) -> Option<&Token> {
        self.tokens.get(self.current)
    }

    pub fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.current + 1)
    }

    pub fn peek_kind(&self) -> Option<TokenKind> {
        self.peek().map(|t| t.kind)
    }

    pub fn peek_value(&self) -> Option<String> {
        self.peek().map(|t| t.value.clone())
    }

    pub fn select(&self, index: usize) -> Option<&Token> {
        self.tokens.get(index)
    }

    pub fn next(&mut self) {
        self.current += 1;
        self.check_eof();
    }

    pub fn check_eof(&mut self) {
        if self.current >= self.tokens.len() {
            self.eof = true;
        }
    }

    pub fn append(&mut self, value: &str, mode: AppendMode) {
        match mode {
            AppendMode::Append => self.result.append(value, false),
            AppendMode::AppendToMainFunction => self.result.append_to_main_function(value, false),
            AppendMode::ForceAppend => self.result.force_append(value, false),
            AppendMode::FFAppend => self.result.ff_append(value, false),
            AppendMode::AppendWithSpace => self.result.append(value, true),
            AppendMode::AppendToMainFunctionWithSpace => {
                self.result.append_to_main_function(value, true)
            }
            AppendMode::ForceAppendWithSpace => self.result.force_append(value, true),
            AppendMode::FFAppendWithSpace => self.result.ff_append(value, true),
        }
    }

    pub fn value(&self) -> String {
        self.current().map_or(String::new(), |t| t.value.clone())
    }

    pub fn kind(&self) -> TokenKind {
        self.current().map_or(TokenKind::Eof, |t| t.kind)
    }

    pub fn parse_mut(&mut self) -> Consumed {
        if self.value() != "mut" {
            return Consumed::consume(0);
        }

        // `mut (a, b) = expr` → `let (mut a, mut b) = expr`
        if self.peek_kind() == Some(TokenKind::ParenthesesStart) {
            if let Some((pattern, total)) = self.scan_tuple_destructure(self.current + 1, true) {
                // total includes `(` through `=`; add 1 for leading `mut`
                self.append(&format!("let ({pattern}) ="), AppendMode::AppendWithSpace);
                return Consumed::consume((total + 1) as isize);
            }
        }

        // `mut identifier = expr` → `let mut identifier = expr`
        if self.peek_kind() == Some(TokenKind::Identifier) {
            // `&mut name` / `&mut name as ...` is a borrow expression — skip.
            if self.current > 0 {
                let prev_value = &self.tokens[self.current - 1].value;
                if prev_value == "&" || prev_value == "&&" {
                    return Consumed::consume(0);
                }
            }
            if let Some(next) = self.select(self.current + 3) {
                if next.value != "=" {
                    self.append(
                        &format!("let mut {}", self.peek_value().unwrap_or_default()),
                        AppendMode::AppendWithSpace,
                    );
                    return Consumed::consume(2);
                }
            }
        }

        Consumed::consume(0)
    }

    /// Scan the token stream starting at `start` (the `(` of a potential
    /// tuple pattern) and determine whether it is a valid destructuring
    /// pattern followed by `=`.
    ///
    /// Returns `Some((bindings, total_consumed))` where `total_consumed`
    /// counts from `start` through (and including) the `=` sign, or `None`
    /// if the pattern is not a simple flat tuple destructure.
    ///
    /// When `all_mut` is true every binding is prefixed with `mut`.
    fn scan_tuple_destructure(&self, start: usize, all_mut: bool) -> Option<(String, usize)> {
        let mut i = start + 1; // skip the outer `(`
        let mut depth = 1usize;
        let mut pending_mut = false;
        let mut saw_binding = false;
        // Build the inner pattern verbatim, preserving nested tuples
        // (`a, (b, c)`); the caller wraps it in `let (...) =`.
        let mut pattern = String::new();

        loop {
            let tok = self.select(i)?;
            i += 1;
            match tok.kind {
                TokenKind::ParenthesesStart | TokenKind::ParametersStart => {
                    // Nested tuple pattern — recurse by tracking depth and
                    // copying the parens into the output.
                    depth += 1;
                    pattern.push('(');
                }
                TokenKind::ParenthesesEnd | TokenKind::ParametersEnd if depth == 1 => {
                    break; // matched the outer `)`; i now points past it
                }
                TokenKind::ParenthesesEnd | TokenKind::ParametersEnd => {
                    depth -= 1;
                    pattern.push(')');
                }
                TokenKind::Identifier => {
                    if all_mut || pending_mut {
                        pattern.push_str("mut ");
                    }
                    pattern.push_str(&tok.value);
                    pending_mut = false;
                    saw_binding = true;
                }
                TokenKind::Keyword if tok.value == "mut" => {
                    pending_mut = true;
                }
                TokenKind::Comma => {
                    pattern.push_str(", ");
                }
                _ => return None,
            }
        }

        if !saw_binding {
            return None;
        }

        // After the outer `)` must come `=` (not `=>`).
        let eq = self.select(i)?;
        if eq.value != "=" {
            return None;
        }
        if matches!(self.select(i + 1), Some(t) if t.kind == TokenKind::Operator && t.value == ">")
        {
            return None;
        }

        // total = tokens from `(` through `=` (inclusive)
        let total = (i + 1) - start;
        Some((pattern, total))
    }

    /// `(a, b) = expr` → `let (a, b) = expr`
    pub fn parse_tuple_destructure(&mut self) -> Consumed {
        if self.kind() != TokenKind::ParenthesesStart {
            return Consumed::consume(0);
        }
        // `SomeName(x, y) = ...` is a constructor pattern, not a tuple destructure.
        // Guard: if the previous significant token is an identifier or keyword,
        // this `(` belongs to a call/constructor, not a tuple binding.
        if let Some(prev) = self.previous_significant() {
            if matches!(
                prev.kind,
                TokenKind::Identifier
                    | TokenKind::Keyword
                    | TokenKind::AngleEnd
                    | TokenKind::Impl
                    | TokenKind::Struct
                    | TokenKind::Trait
            ) {
                return Consumed::consume(0);
            }
        }
        let Some((pattern, total)) = self.scan_tuple_destructure(self.current, false) else {
            return Consumed::consume(0);
        };
        self.append(&format!("let ({pattern}) ="), AppendMode::AppendWithSpace);
        Consumed::consume(total as isize)
    }

    pub fn parse_var(&mut self) -> Consumed {
        let mut consumed = 0;
        // Bail out for the wildcard pattern `_` (used in match arms,
        // destructuring, `let _ = ...` discards): the parser cannot tell that
        // `_ =>` is a match arm without seeing the `=>` ahead, so we just
        // refuse to treat `_ = anything` as a regular assignment. The
        // catch-all `_ = expr` discard is rare in practice and Copper users
        // would write `_ = expr` only inside Rust-shaped match contexts.
        if self.value() == "_" {
            return Consumed::consume(0);
        }
        // Bail out for `name =>` — that's a fat-arrow in a match arm, not
        // an assignment to `name`.
        if self.peek_value() == Some("=".to_string()) {
            if let Some(after_eq) = self.select(self.current + 2) {
                if after_eq.kind == TokenKind::Operator && after_eq.value == ">" {
                    return Consumed::consume(0);
                }
            }
        }
        // Don't inject `let` when the previous token is `const`, `static`, `type`,
        // `let`, or `fn` — the keyword is already the declaration opener.
        if self.current > 0 {
            let prev = &self.tokens[self.current - 1];
            if matches!(
                prev.value.as_str(),
                "const" | "static" | "type" | "let" | "fn"
            ) {
                return Consumed::consume(0);
            }
        }
        // Don't inject `let` when the previous significant token is `:` — we're
        // in a type-annotation position (e.g. the type name in `const X: int = 0`).
        if let Some(prev_sig) = self.previous_significant() {
            if prev_sig.kind == TokenKind::Colon {
                return Consumed::consume(0);
            }
        }
        if self.kind() == TokenKind::Identifier && self.peek_value() == Some("=".to_string()) {
            // `*name = expr` is a deref-assignment, not a let-declaration.
            // Likewise `&name` / `&mut name` followed by `=` is a borrow
            // used in an expression context. Don't inject `let`.
            if self.current > 0 {
                let prev_value = &self.tokens[self.current - 1].value;
                if prev_value == "*" || prev_value == "&" || prev_value == "&&" {
                    return Consumed::consume(0);
                }
            }
            if let Some(var_value) = self.select(self.current + 2) {
                if var_value.value != "=" {
                    let var_name = self.value();

                    // Check if next token (after =) is { or [
                    if var_value.kind == TokenKind::BraceStart
                        || var_value.kind == TokenKind::BracketStart
                    {
                        // Debug: show what type of token was detected
                        // JSON detected var_value.kind, var_value.value);

                        // This is a JSON object or array!
                        consumed += 3; // identifier + = + { or [

                        let is_array = var_value.kind == TokenKind::BracketStart;

                        // Collect all tokens until matching } or ]
                        let mut json_tokens = Vec::new();
                        let mut brace_count = if is_array { 0 } else { 1 };
                        let mut bracket_count = if is_array { 1 } else { 0 };
                        let mut current_idx = self.current + 3;

                        while (brace_count > 0 || bracket_count > 0)
                            && current_idx < self.tokens.len()
                        {
                            if let Some(token) = self.select(current_idx) {
                                // Process JSON token
                                match token.kind {
                                    TokenKind::BraceStart => {
                                        brace_count += 1;
                                        json_tokens.push(token.clone());
                                    }
                                    TokenKind::BraceEnd => {
                                        json_tokens.push(token.clone());
                                        brace_count -= 1;
                                    }
                                    TokenKind::BracketStart => {
                                        bracket_count += 1;
                                        json_tokens.push(token.clone());
                                    }
                                    TokenKind::BracketEnd => {
                                        json_tokens.push(token.clone());
                                        bracket_count -= 1;
                                    }
                                    TokenKind::Symbol => {
                                        // Special case: split symbols containing brackets/braces
                                        if token.value.contains(']') || token.value.contains('}') {
                                            for ch in token.value.chars() {
                                                match ch {
                                                    ']' => {
                                                        let mut bracket_token = token.clone();
                                                        bracket_token.kind = TokenKind::BracketEnd;
                                                        bracket_token.value = "]".to_string();
                                                        json_tokens.push(bracket_token);
                                                        bracket_count -= 1;
                                                    }
                                                    '}' => {
                                                        let mut brace_token = token.clone();
                                                        brace_token.kind = TokenKind::BraceEnd;
                                                        brace_token.value = "}".to_string();
                                                        json_tokens.push(brace_token);
                                                        brace_count -= 1;
                                                    }
                                                    _ => {
                                                        let mut symbol_token = token.clone();
                                                        symbol_token.kind = TokenKind::Symbol;
                                                        symbol_token.value = ch.to_string();
                                                        json_tokens.push(symbol_token);
                                                    }
                                                }
                                            }
                                        } else if brace_count > 0 || bracket_count > 0 {
                                            json_tokens.push(token.clone());
                                        }
                                    }
                                    _ => {
                                        if brace_count > 0 || bracket_count > 0 {
                                            json_tokens.push(token.clone());
                                        }
                                    }
                                }

                                current_idx += 1;
                                consumed += 1;
                            } else {
                                break;
                            }
                        }

                        // Build JSON content
                        let mut json_content = String::new();

                        for (i, token) in json_tokens.iter().enumerate() {
                            // Skip newline tokens that were converted to ";\n"
                            if token.kind == TokenKind::Newline || token.value.contains(";\n") {
                                continue;
                            }

                            let token_value = &token.value;

                            // Add appropriate spacing
                            if i > 0 && !json_content.is_empty() {
                                let last_char = json_content.chars().last().unwrap_or(' ');
                                let first_char = token_value.chars().next().unwrap_or(' ');

                                // JSON spacing rules
                                let needs_space = match (last_char, first_char) {
                                    // After comma or colon, always space
                                    (',', _) | (':', _) => true,
                                    // Before closing delimiters, no space
                                    (_, ',') | (_, ':') | (_, '}') | (_, ']') => false,
                                    // After opening delimiters, no space
                                    ('{', _) | ('[', _) => false,
                                    // Between values, add space
                                    _ if !",:]{}[]".contains(first_char)
                                        && !",:]{}[]".contains(last_char) =>
                                    {
                                        true
                                    }
                                    _ => false,
                                };

                                if needs_space {
                                    json_content.push(' ');
                                }
                            }

                            json_content.push_str(token_value);
                        }

                        // Mark that we're using JSON
                        self.uses_data_types = true;
                        self.result.mark_json_usage();

                        // Complete JSON detection

                        // Generate Rust code with json! macro
                        if is_array {
                            // Remove brackets from content for arrays
                            let mut clean_content = json_content.trim();
                            if clean_content.starts_with('[') {
                                clean_content = &clean_content[1..];
                            }
                            if clean_content.ends_with(']') {
                                clean_content = &clean_content[..clean_content.len() - 1];
                            }
                            self.append(
                                &format!("let {} = json!([{}]);", var_name, clean_content.trim()),
                                AppendMode::AppendWithSpace,
                            );
                        } else {
                            // Remove braces from content for objects
                            let mut clean_content = json_content.trim();
                            if clean_content.starts_with('{') {
                                clean_content = &clean_content[1..];
                            }
                            if clean_content.ends_with('}') {
                                clean_content = &clean_content[..clean_content.len() - 1];
                            }
                            self.append(
                                &format!("let {} = json!({{{}}});", var_name, clean_content.trim()),
                                AppendMode::AppendWithSpace,
                            );
                        }
                        self.append("\n", AppendMode::Append);
                    } else {
                        // Normal variable
                        consumed += 2;
                        self.append(&format!("let {} = ", var_name), AppendMode::AppendWithSpace);
                    }
                }
            }
        }
        Consumed::consume(consumed)
    }

    pub fn parse_type_declaration(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.kind() == TokenKind::Identifier && self.peek_value() == Some(":".to_string()) {
            if let Some(type_token) = self.select(self.current + 2) {
                // Check if it's a type declaration (identifier : type)
                if type_token.kind == TokenKind::Json
                    || type_token.kind == TokenKind::Xml
                    || type_token.kind == TokenKind::Toml
                    || type_token.kind == TokenKind::Identifier
                    || type_token.kind == TokenKind::ParamType
                    || type_token.kind == TokenKind::Keyword
                {
                    let var_name = self.value();
                    let (type_name, data_type) =
                        utils::convert_type_with_marking(&type_token.value);

                    // Mark data type usage when types are used
                    if let Some(dt) = data_type {
                        self.uses_data_types = true;
                        match dt.as_str() {
                            "json" => self.result.mark_json_usage(),
                            "xml" => self.result.mark_xml_usage(),
                            "toml" => self.result.mark_toml_usage(),
                            _ => {}
                        }
                    }

                    // Don't inject `let` when inside a const/static/let declaration.
                    let prev_is_decl = self.current > 0
                        && matches!(
                            self.tokens[self.current - 1].value.as_str(),
                            "const" | "static" | "let"
                        );

                    // Collect any generic type args following the base type (e.g. `Vec<T>`).
                    let mut full_type = type_name.clone();
                    let mut extra: isize = 0;
                    let base = self.current + 3;
                    if let Some(angle) = self.select(base + extra as usize) {
                        if angle.kind == TokenKind::AngleStart
                            || (angle.kind == TokenKind::Operator && angle.value == "<")
                        {
                            full_type.push('<');
                            extra += 1;
                            let mut depth = 1usize;
                            while let Some(t) = self.select(base + extra as usize) {
                                extra += 1;
                                let is_open = t.kind == TokenKind::AngleStart
                                    || (t.kind == TokenKind::Operator && t.value == "<");
                                let is_close = t.kind == TokenKind::AngleEnd
                                    || (t.kind == TokenKind::Operator && t.value == ">");
                                if is_open {
                                    depth += 1;
                                    full_type.push('<');
                                } else if is_close {
                                    depth -= 1;
                                    full_type.push('>');
                                    if depth == 0 {
                                        break;
                                    }
                                } else {
                                    full_type.push_str(&t.value);
                                }
                            }
                        }
                    }

                    // Distinguish a real typed declaration (`count: i32`) from a
                    // struct-literal field (`Vec2 { x: self.x }`), which must
                    // pass through untouched — otherwise `x: self.x` becomes
                    // `let x: self; .x`. It's a struct field when the name
                    // follows a `,` (a later field) or the value after the type
                    // continues as an expression (`.`/`(`/`[`/`,`/operator/`::`).
                    let name_follows_comma =
                        self.current > 0 && self.tokens[self.current - 1].kind == TokenKind::Comma;
                    let value_continues = match self.select(self.current + 3 + extra as usize) {
                        Some(t) => {
                            matches!(
                                t.kind,
                                TokenKind::Dot
                                    | TokenKind::OptionalChain
                                    | TokenKind::ParenthesesStart
                                    | TokenKind::BracketStart
                                    | TokenKind::Operator
                                    | TokenKind::Comma
                            ) || t.value == "::"
                            // A newline the tokenizer marked as a `,` separator
                            // (not `;`) sits between struct-literal fields
                            // (`Vec2 {\n x: a,\n y: b\n}`) — not a declaration.
                            || (t.kind == TokenKind::Newline
                                && t.value.trim_start().starts_with(','))
                        }
                        None => false,
                    };
                    if name_follows_comma || value_continues {
                        return Consumed::consume(0);
                    }

                    if prev_is_decl {
                        self.append(
                            &format!("{var_name}: {full_type}"),
                            AppendMode::AppendWithSpace,
                        );
                    } else {
                        self.append(
                            &format!("let {var_name}: {full_type};"),
                            AppendMode::AppendWithSpace,
                        );
                    }
                    consumed += 3 + extra; // identifier + : + type [+ generics]
                }
            }
        }
        Consumed::consume(consumed)
    }

    pub fn parse_json_object(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.kind() == TokenKind::JsonObject {
            let value = self.value();

            // Extract variable name and JSON content
            if let Some(eq_pos) = value.find('=') {
                let var_name = value[..eq_pos].trim();
                let json_content = value[eq_pos + 1..].trim();

                // Mark that we're using JSON
                self.uses_data_types = true;
                self.result.mark_json_usage();

                // Generate Rust code with json! macro
                self.append(
                    &format!("let {} = json!({});", var_name, json_content),
                    AppendMode::AppendWithSpace,
                );
                consumed += 1;
            }
        }
        Consumed::consume(consumed)
    }

    pub fn parse_any(&mut self) -> Consumed {
        let token_value = self.value();

        // Skip invalid tokens or tokens that shouldn't be in output
        if token_value.is_empty()
            || token_value.contains("Como parâmetros de função")
            || token_value.contains("rocessaJSON")
            || token_value.starts_with("//")
            || self.kind() == TokenKind::Comment
            || self.kind() == TokenKind::Unknown
        {
            return Consumed::consume(1);
        }

        // `unsafe func ...` — swallow `unsafe` here and let parse_function
        // emit `unsafe fn` once `func` is dispatched. Without this the
        // `unsafe` would land in main_function_code and fuse with whatever
        // comes after the function definition (e.g. `unsafelet x = ...`).
        if token_value == "unsafe" {
            if let Some(next) = self.peek() {
                if next.value == "func" {
                    self.pending_unsafe_fn = true;
                    return Consumed::consume(1);
                }
            }
        }

        // `async func ...` → set flag so parse_function emits `async fn`
        if token_value == "async" {
            if let Some(next) = self.peek() {
                if next.value == "func" {
                    self.pending_async_fn = true;
                    return Consumed::consume(1);
                }
            }
        }

        // `.await` — no leading space when directly after a dot
        if token_value == "await"
            && self.current > 0
            && self.tokens[self.current - 1].kind == TokenKind::Dot
        {
            self.append("await", AppendMode::Append);
            return Consumed::consume(1);
        }

        // `pub func ...` / `pub unsafe func ...` / `pub struct ...` (and
        // `public`, normalized to `pub`) — swallow the `pub` and let the item's
        // emitter prepend `pub `. Without this the visibility leaks onto the
        // next statement (`pub let x = ...`), because items emit to the module
        // stream while a stray `pub` lands in `main()`. Other items (class,
        // enum, const, …) fall through to the spaced-keyword emit below.
        if token_value == "pub" {
            let item_follows = match self.peek().map(|t| t.value.as_str()) {
                Some("func") | Some("struct") => true,
                Some("unsafe") => self
                    .select(self.current + 2)
                    .map(|t| t.value == "func")
                    .unwrap_or(false),
                _ => false,
            };
            if item_follows {
                self.pending_pub = true;
                return Consumed::consume(1);
            }
        }

        for (copper, rust) in RUST_MACROS.iter() {
            if token_value == *copper {
                // Check if next token is already !, if so, don't add another one
                if let Some(next_token) = self.peek() {
                    if next_token.value == "!" {
                        // Next token is already !, just output the macro name without !
                        self.append(copper, AppendMode::Append);
                        return Consumed::consume(1);
                    }
                }
                // Bang-less call form: `println(...)`. Rust's print macros
                // need a format string as the first argument. If the call
                // already passes one (a string / interpolated literal first
                // arg) we leave it alone — the interpolated case renders as
                // macro args via `is_inside_macro_call`. Otherwise inject a
                // `"{} {} ...", ` format string (one `{}` per top-level
                // argument) so `println(x)` → `println!("{}", x)` and
                // `println(a, b)` → `println!("{} {}", a, b)`, both valid Rust.
                if self.peek_kind() == Some(TokenKind::ParenthesesStart) {
                    if let Some((first_is_fmt, argc)) = self.scan_macro_call_args(self.current + 1) {
                        if !first_is_fmt && argc > 0 {
                            let mut fmt = String::with_capacity(argc * 3 + 4);
                            fmt.push_str(rust); // e.g. "println!"
                            fmt.push('(');
                            fmt.push('"');
                            for i in 0..argc {
                                if i > 0 { fmt.push(' '); }
                                fmt.push_str("{}");
                            }
                            fmt.push_str("\", ");
                            self.append(&fmt, AppendMode::Append);
                            // Consumed the macro name + the '(' — the args and
                            // closing ')' emit through the normal token path.
                            return Consumed::consume(2);
                        }
                    }
                }
                // Next token is not !, so add the ! suffix
                self.append(rust, AppendMode::Append);
                return Consumed::consume(1);
            }
        }

        // Control-flow keywords need spaces on both sides so they don't fuse
        // with neighbours: without the trailing space `while n` becomes
        // `whilen`; without the leading space `n if x > 0 =>` becomes
        // `nif x > 0 =>` (the `n` from a match-arm pattern fuses with the
        // guard's `if`). Pad with a leading space too — at statement starts
        // the extra space is harmless whitespace before indentation.
        if matches!(
            token_value.as_str(),
            "if" | "else"
                | "loop"
                | "while"
                | "for"
                | "in"
                | "match"
                | "return"
                | "break"
                | "continue"
                | "as"
                | "let"
                | "mut"
                | "pub"
                | "ref"
                | "move"
                | "yield"
                | "trait"
                | "enum"
                | "type"
                | "mod"
                | "impl"
                | "unsafe"
                | "const"
                | "static"
                | "async"
                | "await"
                | "dyn"
                | "extern"
                | "where"
        ) {
            self.append(&format!(" {}", token_value), AppendMode::AppendWithSpace);
            return Consumed::consume(1);
        }

        self.append(&token_value, AppendMode::Append);
        Consumed::consume(1)
    }

    /// Emit a `TokenKind::InterpolatedString` with the right shape for its
    /// surrounding context.
    ///
    /// * Inside a `name!(...)` macro call (detected by walking back to the
    ///   enclosing `(` and checking for a preceding `!`), the literal becomes
    ///   the macro's format string and arguments — `"Hi {}", name` — so
    ///   things like `println!("Hi $name")` compile to
    ///   `println!("Hi {}", name)` directly.
    /// * Anywhere else (assignments, return values, function args, …) we
    ///   fall back to the `format!(...)` wrapper that the tokenizer baked
    ///   into the token's `value`, since that's a plain `String` expression
    ///   and works in every position.
    pub fn parse_interpolated_string(&mut self) -> Consumed {
        if self.kind() != TokenKind::InterpolatedString {
            return Consumed::consume(0);
        }

        let rendered = if self.is_inside_macro_call() {
            if let Some(token) = self.current() {
                if let Data::Interpolation { placeholder, args } = &token.data {
                    let interp = interpolation::Interpolated {
                        placeholder: placeholder.clone(),
                        args: args.clone(),
                    };
                    interpolation::render_macro_args(&interp)
                } else {
                    self.value()
                }
            } else {
                self.value()
            }
        } else {
            self.value()
        };

        self.append(&rendered, AppendMode::Append);
        Consumed::consume(1)
    }

    /// Emit the prefix that opens an optional-chaining expression. Pushes
    /// the current `chain_delim_depth` onto the stack so we know where the
    /// chain ends, and writes `.as_ref().map(|__copt<n>| __copt<n>.` — the
    /// trailing `.` consumes the dot of `?.` (which the tokenizer folded
    /// into the OptionalChain token), and the closing `)` is appended later
    /// by [`maybe_close_optional_chains_for`].
    pub fn parse_optional_chain(&mut self) -> Consumed {
        if self.kind() != TokenKind::OptionalChain {
            return Consumed::consume(0);
        }
        self.optional_chain_counter += 1;
        let var = format!("__copt{}", self.optional_chain_counter);
        self.optional_chain_depths.push(self.chain_delim_depth);
        self.append(
            &format!(".as_ref().map(|{var}| {var}.", var = var),
            AppendMode::Append,
        );
        Consumed::consume(1)
    }

    /// Close any optional chains whose start depth equals the current
    /// `chain_delim_depth` if the upcoming token would end the chain at
    /// that depth (binary operator, separator, block boundary, or a
    /// matching closing delimiter that's about to leave our scope).
    fn maybe_close_optional_chains_for(&mut self, kind: TokenKind, value: &str) {
        while let Some(&chain_depth) = self.optional_chain_depths.last() {
            if self.chain_delim_depth != chain_depth {
                // Chain is still inside a deeper sub-expression; leave it
                // open until we come back out.
                break;
            }
            if !is_chain_breaker(kind, value) {
                break;
            }
            self.append(")", AppendMode::Append);
            self.optional_chain_depths.pop();
        }
    }

    /// Decide whether a `[` opens a list literal (`[1, 2, 3]` → `vec![1, 2, 3]`)
    /// or an index/slice access (`arr[0]`, `arr[1..3]`). The discriminator is
    /// the previous significant token: an identifier, closing paren, closing
    /// bracket, or `self`/keyword that produces a value means we're indexing
    /// something. Anything else (operators, commas, statement starts, opening
    /// delimiters) is a literal.
    pub fn parse_bracket(&mut self) -> Consumed {
        if self.kind() != TokenKind::BracketStart {
            return Consumed::consume(0);
        }

        let is_indexing = matches!(
            self.previous_significant_kind(),
            Some(TokenKind::Identifier)
                | Some(TokenKind::ParametersEnd)
                | Some(TokenKind::ParenthesesEnd)
                | Some(TokenKind::BracketEnd)
        );
        // A `[` immediately after `!` is the body of a macro invocation
        // (`vec![...]`, `assert![...]`, ...) — never a literal. Don't add a
        // second `vec!` prefix.
        let is_macro_body = matches!(
            self.previous_significant(),
            Some(t) if t.kind == TokenKind::Operator && t.value == "!"
        );
        // `&[T]` is a slice type reference, not a vec literal.
        let is_slice_type = matches!(
            self.previous_significant(),
            Some(t) if t.kind == TokenKind::Operator && t.value == "&"
        );

        if is_indexing || is_macro_body || is_slice_type {
            self.append("[", AppendMode::Append);
        } else {
            self.append("vec![", AppendMode::Append);
        }
        // The dispatch loop advances `current` via `consume_var` based on
        // the returned count — calling `self.next()` here would double-step
        // and swallow the first element of the literal.
        Consumed::consume(1)
    }

    /// Return the kind of the most recently seen non-Newline / non-Comment
    /// token before the current position, or `None` at the start of input.
    fn previous_significant_kind(&self) -> Option<TokenKind> {
        self.previous_significant().map(|t| t.kind)
    }

    fn previous_significant(&self) -> Option<&Token> {
        if self.current == 0 {
            return None;
        }
        let mut i = self.current;
        while i > 0 {
            i -= 1;
            if !matches!(
                self.tokens[i].kind,
                TokenKind::Newline | TokenKind::Comment | TokenKind::DocComment
            ) {
                return Some(&self.tokens[i]);
            }
        }
        None
    }

    /// Walk back from the current position, balancing parentheses, to find
    /// the `(` that opens the call we're inside (if any). If that `(` is
    /// preceded by an `!` operator, we're inside a Rust macro call and
    /// interpolated strings should expand to raw `"fmt", args` form.
    /// Scan a parenthesised argument list for a bang-less macro call.
    /// `paren_idx` is the index of the opening `(` (a ParenthesesStart).
    /// Returns `(first_arg_is_format_string, top_level_arg_count)`:
    ///   * `first_arg_is_format_string` — the first argument's leading token
    ///     is a string / interpolated-string literal, i.e. the call already
    ///     supplies a format string and needs no injection.
    ///   * `top_level_arg_count` — number of comma-separated arguments at
    ///     paren depth 1 (0 for an empty `()`).
    /// Returns `None` if `paren_idx` isn't an open paren or the list is
    /// unterminated.
    fn scan_macro_call_args(&self, paren_idx: usize) -> Option<(bool, usize)> {
        let open = self.tokens.get(paren_idx)?;
        if !matches!(
            open.kind,
            TokenKind::ParenthesesStart | TokenKind::ParametersStart
        ) {
            return None;
        }
        let mut depth: usize = 1;
        let mut argc: usize = 0;
        let mut seen_arg_token = false;
        let mut first_is_fmt = false;
        let mut first_token_seen = false;
        let mut i = paren_idx + 1;
        while i < self.tokens.len() {
            let tok = &self.tokens[i];
            match tok.kind {
                TokenKind::ParenthesesStart | TokenKind::ParametersStart => depth += 1,
                TokenKind::ParenthesesEnd | TokenKind::ParametersEnd => {
                    depth -= 1;
                    if depth == 0 {
                        if seen_arg_token {
                            argc += 1;
                        }
                        return Some((first_is_fmt, argc));
                    }
                }
                TokenKind::Comma if depth == 1 => {
                    argc += 1;
                    seen_arg_token = false;
                }
                TokenKind::Newline | TokenKind::Comment => {}
                _ if depth == 1 => {
                    if !first_token_seen {
                        first_token_seen = true;
                        first_is_fmt = matches!(
                            tok.kind,
                            TokenKind::String | TokenKind::InterpolatedString
                        );
                    }
                    seen_arg_token = true;
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    fn is_inside_macro_call(&self) -> bool {
        let mut depth: usize = 0;
        let mut i = self.current;
        while i > 0 {
            i -= 1;
            let tok = &self.tokens[i];
            match tok.kind {
                TokenKind::ParametersEnd | TokenKind::ParenthesesEnd => {
                    depth += 1;
                }
                TokenKind::ParametersStart | TokenKind::ParenthesesStart => {
                    if depth == 0 {
                        // Found the enclosing open paren. Look at the token
                        // immediately before it for a `!` operator.
                        if i == 0 {
                            return false;
                        }
                        let prev = &self.tokens[i - 1];
                        // A `!` immediately before the paren is the explicit
                        // Rust-macro form (`println!(...)`). Copper also lets
                        // you call the known format macros without the bang
                        // (`println("Hi $name")`) — the bang is injected at
                        // emit time by RUST_MACROS, so it isn't in the token
                        // stream yet. Recognise those names here too, so the
                        // interpolated string renders as macro args
                        // (`"Hi {}", name`) instead of a nested
                        // `format!(...)`.
                        if prev.kind == TokenKind::Operator && prev.value == "!" {
                            return true;
                        }
                        return RUST_MACROS.iter().any(|(copper, _)| *copper == prev.value);
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        false
    }

    pub fn parse_function(&mut self) -> Consumed {
        if self.value() == "func" {
            self.function_start = true;
            let is_pub = self.pending_pub;
            self.pending_pub = false;
            if self.pending_unsafe_fn {
                self.result.enter_unsafe_function_vis(is_pub);
                self.pending_unsafe_fn = false;
            } else if self.pending_async_fn {
                // emit `async fn ` instead of `fn `
                let prefix = if is_pub { "pub async fn " } else { "async fn " };
                self.result.force_append(prefix, false);
                self.result.is_function = true;
                self.result.is_copper_function = true;
                self.pending_async_fn = false;
            } else {
                self.result.enter_function_vis(is_pub);
            }
            // Mark that we're using Copper syntax
            self.result.is_copper_function = true;
            return Consumed::consume(1);
        } else if self.value() == "fn" {
            // For pure Rust syntax, just mark as function but don't add "fn"
            // because it's already present
            self.function_start = true;
            self.result.is_function = true;
            self.result.is_copper_function = false;
            self.append(&self.value(), AppendMode::AppendWithSpace);
            return Consumed::consume(1);
        }
        Consumed::consume(0)
    }

    pub fn parse_function_params(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.value() == "(" && self.kind() == TokenKind::ParametersStart {
            self.append(&self.value(), AppendMode::Append);
            consumed += 1;
        }
        if self.value() == ")" && self.kind() == TokenKind::ParametersEnd {
            if self.result.is_function {
                self.append(&self.value(), AppendMode::Append);
                // Only add return type for Copper syntax
                if self.result.is_copper_function {
                    let return_type = if self.result.return_type.is_empty() {
                        "()"
                    } else {
                        &self.result.return_type
                    };
                    self.append(&format!(" -> {}", return_type), AppendMode::Append);
                }
                self.result.exit_function();
                self.result.is_inside_function = true;
            } else {
                self.append(&self.value(), AppendMode::Append);
            }
            consumed += 1;
        }
        Consumed::consume(consumed)
    }

    pub fn parse_function_body(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.value() == "{" && self.kind() == TokenKind::BraceStart {
            // Inner blocks (match arms, if/else, nested scopes) just deepen
            // the count — only the matching outer `}` should exit the
            // function.
            if self.result.is_inside_function {
                self.function_brace_depth += 1;
            }
            self.append(&self.value(), AppendMode::Append);
            consumed += 1;
        }
        if self.value() == "}"
            && self.kind() == TokenKind::BraceEnd
            && !self.seen_import
            && !self.is_inside_class
        {
            self.append(&self.value(), AppendMode::Append);
            if self.result.is_inside_function {
                if self.function_brace_depth > 1 {
                    // Closing an inner block — stay inside the function.
                    self.function_brace_depth -= 1;
                } else {
                    // The matching outer `}` of the function body.
                    self.function_brace_depth = 0;
                    self.append("\n", AppendMode::AppendWithSpace);
                    self.result.is_inside_function = false;
                }
            } else {
                self.append("\n", AppendMode::AppendWithSpace);
            }
            consumed += 1;
        }
        Consumed::consume(consumed)
    }

    pub fn parse_operator(&mut self) -> Consumed {
        let mut consumed = 0;
        for (copper, rust) in COPPER_OPERATORS.iter() {
            if let Some(next_token) = self.peek() {
                if (next_token.value.clone() + &self.value()) == *copper {
                    self.append(rust, AppendMode::Append);
                    consumed += 2;
                    break;
                }
            }
        }
        Consumed::consume(consumed)
    }

    pub fn parse_import(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.value() == "from" && self.kind() == TokenKind::From {
            consumed += 1;
        }
        if self.value() == "import" && self.kind() == TokenKind::Import {
            self.seen_import = true;
            // `use` belongs at module level, not inside `main()` — otherwise a
            // module-level `func` can't see a type pulled in by the import.
            self.append("use", AppendMode::ForceAppendWithSpace);
            consumed += 1;
        }
        if self.value() == "{" && self.kind() == TokenKind::BraceStart && self.seen_import {
            self.is_import_list = true;
            consumed += 1;
        }
        if self.value() == "}" && self.kind() == TokenKind::BraceEnd && self.seen_import {
            consumed += 1;
        }
        if self.kind() == TokenKind::ModuleVar {
            self.current_import_vars.push(self.value());
            consumed += 1;
        }
        // Swallow comma separators inside import lists so they don't leak
        // into the emitted `use` statement (otherwise we get `use ,,,X`).
        if self.kind() == TokenKind::Comma && self.seen_import && self.is_import_list {
            consumed += 1;
        }
        if self.kind() == TokenKind::ModulePath {
            if self.seen_import {
                let module = self.value();
                if module == "cstd" {
                    for var in &self.current_import_vars {
                        self.result.mark_cstd_used(var);
                    }
                } else if Self::is_native_std_module(&module) {
                    self.result.mark_std_module(&module);
                } else if !matches!(
                    module.as_str(),
                    "std" | "core" | "alloc" | "crate" | "self" | "super"
                ) {
                    // A `from <crate>` that isn't std/cstd is a candidate
                    // external dependency. cforge drops it later if it turns out
                    // to be a sibling module (a copied `.rs`/`.crs`).
                    self.result.mark_external_crate(&module);
                }
                self.append(&self.value(), AppendMode::ForceAppend);
                if !self.current_import_vars.is_empty() {
                    if self.is_import_list {
                        self.append("::", AppendMode::ForceAppend);
                        self.append("{", AppendMode::ForceAppend);
                        self.append(
                            &self.current_import_vars.join(", "),
                            AppendMode::ForceAppend,
                        );
                        self.append("}", AppendMode::ForceAppend);
                        self.is_import_list = false;
                    } else {
                        let var = self.current_import_vars[0].clone();
                        self.append(" as", AppendMode::ForceAppendWithSpace);
                        self.append(&var, AppendMode::ForceAppend);
                    }
                    self.current_import_vars.clear();
                }
                // Terminate the `use` at module level. The statement's own
                // newline terminator goes to `main()` (a harmless empty `;`
                // rustfmt drops); without this the `use` would fuse with the
                // next module item (`use foo::{Bar}pub fn …`).
                self.append(";", AppendMode::ForceAppend);
                self.seen_import = false;
                consumed += 1;
            } else {
                self.append(&self.value(), AppendMode::Append);
                consumed += 1;
            }
        }
        Consumed::consume(consumed)
    }

    pub fn parse_regex(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.kind() == TokenKind::Regex {
            if !self.result.has_required_import("regex") {
                self.result.add_required_import("regex");
            }
            let value = self.value();
            let var = "__regex__";
            let resolved_regex = value.trim_start_matches('/').trim_end_matches('/');
            self.append(
                &format!("{}::Regex::new(r\"{}\").unwrap()", var, resolved_regex),
                AppendMode::Append,
            );
            consumed += 1;
        }
        Consumed::consume(consumed)
    }

    pub fn parse_class_definition(&mut self) -> Consumed {
        // Detects "class Name { ... }"
        if self.value() == "class" && self.kind() == TokenKind::Keyword {
            let mut consumed = 1;

            let class_name = if let Some(tok) = self.select(self.current + consumed) {
                if tok.kind == TokenKind::Identifier {
                    consumed += 1;
                    tok.value.clone()
                } else {
                    return Consumed::consume(0);
                }
            } else {
                return Consumed::consume(0);
            };
            self.current_class = Some(class_name.clone());
            self.is_inside_class = true;

            let mut brace_count = 0;
            let mut class_tokens = Vec::new();
            // first, advances until it finds the first '{'
            while let Some(tok) = self.select(self.current + consumed) {
                consumed += 1;
                if tok.kind == TokenKind::BraceStart {
                    brace_count = 1;
                    class_tokens.push(tok.clone());
                    break;
                }
            }
            // Now collects until all braces are closed
            while brace_count > 0 {
                if let Some(tok) = self.select(self.current + consumed) {
                    consumed += 1;
                    match tok.kind {
                        TokenKind::BraceStart => brace_count += 1,
                        TokenKind::BraceEnd => brace_count -= 1,
                        _ => {}
                    }
                    class_tokens.push(tok.clone());
                } else {
                    break;
                }
            }

            // Processes members and appends the generated Rust code at MODULE
            // level (like structs/functions) — not inside main() — so sibling
            // functions can reference the class type. ForceAppend routes to
            // the module stream.
            let parsed = self.process_class_members(&class_tokens, &class_name);
            self.append(&parsed, AppendMode::ForceAppendWithSpace);

            // reset
            self.is_inside_class = false;
            self.current_class = None;
            return Consumed::consume(consumed.try_into().unwrap());
        }

        Consumed::consume(0)
    }

    fn process_class_members(&mut self, class_tokens: &[Token], class_name: &str) -> String {
        let tokens: Vec<&Token> = class_tokens
            .iter()
            .filter(|t| t.kind != TokenKind::Newline)
            .collect();
        if tokens.len() < 2 {
            return String::new();
        }
        let inner = &tokens[1..tokens.len() - 1];

        let mut output = String::new();
        let mut fields = Vec::new();

        // Collect the fields
        let mut i = 0;
        while i + 2 < inner.len() {
            let a = &inner[i];
            let b = &inner[i + 1];
            let c = &inner[i + 2];
            let is_colon =
                (b.kind == TokenKind::Operator || b.kind == TokenKind::Colon) && b.value == ":";
            let is_type = c.kind == TokenKind::Identifier
                || c.kind == TokenKind::ParamType
                || c.kind == TokenKind::Keyword;

            if a.kind == TokenKind::Identifier && is_colon && is_type {
                fields.push((a.value.clone(), c.value.clone()));
                i += 3;
            } else {
                break;
            }
        }

        // Build the struct
        output.push_str(&format!("struct {} {{\n", class_name));
        for (n, t) in &fields {
            output.push_str(&format!("    {}: {},\n", n, t));
        }
        output.push_str("}\n\n");

        // Build the constructor
        output.push_str(&format!("impl {} {{\n", class_name));
        if let Some(pos) = inner
            .iter()
            .position(|t| t.kind == TokenKind::Identifier && t.value == class_name)
        {
            if pos + 1 < inner.len() && inner[pos + 1].kind == TokenKind::ParenthesesStart {
                // coleta params
                let mut params = Vec::new();
                let mut j = pos + 2;
                while j + 2 < inner.len()
                    && (inner[j].kind == TokenKind::Param || inner[j].kind == TokenKind::Identifier)
                {
                    let pname = inner[j].value.clone();
                    let sep = &inner[j + 1];
                    let ptyp = &inner[j + 2];
                    let ok_sep = (sep.kind == TokenKind::Operator || sep.kind == TokenKind::Colon)
                        && sep.value == ":";
                    // A type can be a ParamType (`String`, `Vec<..>`), a
                    // primitive Keyword (`i32`, `f64`, `bool`), or a plain
                    // Identifier (a user type). Accept all and normalise via
                    // convert_type (e.g. `int` -> `i64`).
                    let is_type = matches!(
                        ptyp.kind,
                        TokenKind::ParamType | TokenKind::Keyword | TokenKind::Identifier
                    );
                    if ok_sep && is_type {
                        let (rtype, _) = utils::convert_type_with_marking(&ptyp.value);
                        params.push((pname.clone(), rtype));
                        j += 3;
                        if j < inner.len() && inner[j].value == "," {
                            j += 1;
                        }
                    } else {
                        break;
                    }
                }

                // Jump to the first '{'
                while j < inner.len() && inner[j].kind != TokenKind::BraceStart {
                    j += 1;
                }
                if j >= inner.len() {
                    return output;
                }

                // Extract the assignments
                let mut assigns = Vec::new();

                // Initialize for the class body analysis
                let mut bi: usize = j + 1;
                while bi < inner.len() {
                    let t0 = &inner[bi];
                    if t0.kind == TokenKind::BraceEnd {
                        break;
                    }
                    // detecta "self . campo ="
                    if t0.kind == TokenKind::Keyword
                        && t0.value == "self"
                        && bi + 3 < inner.len()
                        && inner[bi + 1].kind == TokenKind::Dot
                        && inner[bi + 2].kind == TokenKind::Identifier
                        && inner[bi + 3].value == "="
                    {
                        let field = inner[bi + 2].value.clone();
                        // Collect all tokens until the end of the statement or block
                        let mut expr_tokens = Vec::new();
                        let mut xi = bi + 4;

                        while xi < inner.len() {
                            let tt = &inner[xi];
                            // Found statement end or block end
                            if tt.value == ";" || tt.kind == TokenKind::BraceEnd {
                                break;
                            }
                            // Found new assignment
                            if tt.kind == TokenKind::Keyword
                                && tt.value == "self"
                                && xi + 3 < inner.len()
                                && inner[xi + 1].kind == TokenKind::Dot
                                && inner[xi + 2].kind == TokenKind::Identifier
                                && inner[xi + 3].value == "="
                            {
                                break;
                            }

                            expr_tokens.push(tt.value.clone());
                            xi += 1;
                        }

                        let expr = expr_tokens.join(" ").trim().to_string();
                        assigns.push((field, expr));
                        bi = xi;
                        continue;
                    }

                    bi += 1;
                }

                // Generate the constructor signature
                let sig = params
                    .iter()
                    .map(|(n, t)| format!("{}: {}", n, t))
                    .collect::<Vec<_>>()
                    .join(", ");

                // Initialize the fields
                let init = fields
                    .iter()
                    .map(|(field_name, _)| {
                        // Searches in assigns if this field has an explicit initialization
                        if let Some((_, expr)) = assigns.iter().find(|(f, _)| f == field_name) {
                            format!("{}: {}", field_name, expr)
                        } else if let Some((param_name, _)) =
                            params.iter().find(|(n, _)| n == field_name)
                        {
                            format!("{}: {}", field_name, param_name)
                        } else {
                            // Default if not found in assigns or params
                            format!("{}: Default::default()", field_name)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                output.push_str(&format!("    pub fn new({}) -> Self {{\n", sig));
                output.push_str(&format!("        Self {{ {} }}\n", init));
                output.push_str("    }\n\n");
            }
        }

        // Methods: `RetType name(params) { body }`. Instance methods take an
        // implicit `&self` (Copper bodies reference `self.field` without
        // declaring `self`), so we emit `&self` and append any declared
        // params (skipping an explicit `self`). The constructor
        // (`ClassName(...)`) is handled above and skipped here.
        let mut k = 0;
        while k + 2 < inner.len() {
            let return_type = &inner[k];
            let name_t = &inner[k + 1];
            let pstart = &inner[k + 2];

            let valid_return_type = return_type.kind == TokenKind::Identifier
                || return_type.kind == TokenKind::ParamType
                || return_type.kind == TokenKind::Keyword;

            let is_method_head = valid_return_type
                && name_t.kind == TokenKind::Identifier
                && name_t.value != class_name
                && pstart.kind == TokenKind::ParenthesesStart;

            if !is_method_head {
                k += 1;
                continue;
            }

            // Collect declared params between `(` and the matching close.
            // An explicit `self` / `&self` / `mut self` is dropped — we
            // always emit `&self`.
            let mut params: Vec<String> = Vec::new();
            let mut idx = k + 3;
            while idx < inner.len() {
                let t = &inner[idx];
                if t.kind == TokenKind::ParametersEnd || t.kind == TokenKind::ParenthesesEnd {
                    break;
                }
                if t.value == "self" || t.value == "&" || t.value == "mut" || t.value == "," {
                    idx += 1;
                    continue;
                }
                if (t.kind == TokenKind::Param || t.kind == TokenKind::Identifier)
                    && idx + 2 < inner.len()
                {
                    let sep = &inner[idx + 1];
                    let typ = &inner[idx + 2];
                    let ok_sep = (sep.kind == TokenKind::Operator || sep.kind == TokenKind::Colon)
                        && sep.value == ":";
                    if ok_sep {
                        let (ptype, _) = utils::convert_type_with_marking(&typ.value);
                        params.push(format!("{}: {}", t.value, ptype));
                        idx += 3;
                        continue;
                    }
                }
                idx += 1;
            }

            // Advance to the method body's opening brace.
            while idx < inner.len() && inner[idx].kind != TokenKind::BraceStart {
                idx += 1;
            }
            if idx >= inner.len() {
                k += 1;
                continue;
            }

            // Collect the balanced body.
            let mut body = Vec::new();
            let mut depth = 1;
            idx += 1;
            while idx < inner.len() && depth > 0 {
                let tt = &inner[idx];
                if tt.kind == TokenKind::BraceStart {
                    depth += 1;
                } else if tt.kind == TokenKind::BraceEnd {
                    depth -= 1;
                }
                if depth > 0 {
                    body.push(tt.value.clone());
                }
                idx += 1;
            }

            let body_str = body.join(" ");
            let (rust_type, data_type) = utils::convert_type_with_marking(&return_type.value);
            if let Some(dt) = data_type {
                self.uses_data_types = true;
                match dt.as_str() {
                    "json" => self.result.mark_json_usage(),
                    "xml" => self.result.mark_xml_usage(),
                    "toml" => self.result.mark_toml_usage(),
                    _ => {}
                }
            }

            let self_and_params = if params.is_empty() {
                "&self".to_string()
            } else {
                format!("&self, {}", params.join(", "))
            };
            if rust_type == "()" {
                output.push_str(&format!("    pub fn {}({}) {{\n", name_t.value, self_and_params));
            } else {
                output.push_str(&format!(
                    "    pub fn {}({}) -> {} {{\n",
                    name_t.value, self_and_params, rust_type
                ));
            }
            output.push_str(&format!("        {}\n", body_str));
            output.push_str("    }\n\n");
            k = idx;
        }
        output.push_str("}\n");
        output
    }

    // Parses native struct definition
    pub fn parse_struct_definition(&mut self) -> Consumed {
        if self.value() == "struct" && self.kind() == TokenKind::Struct {
            let mut consumed = 1;

            let struct_name = if let Some(tok) = self.select(self.current + consumed) {
                if tok.kind == TokenKind::Identifier {
                    consumed += 1;
                    tok.value.clone()
                } else {
                    return Consumed::consume(0);
                }
            } else {
                return Consumed::consume(0);
            };

            self.current_struct = Some(struct_name.clone());
            self.is_inside_struct = true;

            // Generic parameter list `<T, U, ...>`. The tokenizer may emit the
            // brackets either as AngleStart/AngleEnd or — because the
            // comparison-operator rule fires first — as Operator("<") /
            // Operator(">"). Accept both, and track `<` depth so nested
            // generics (`<Vec<T>>`) and multiple params close correctly.
            let mut generics = String::new();
            let opens_generics = matches!(
                self.select(self.current + consumed),
                Some(t) if t.kind == TokenKind::AngleStart
                    || (t.kind == TokenKind::Operator && t.value == "<")
            );
            if opens_generics {
                consumed += 1;
                generics.push('<');
                let mut depth = 1usize;
                while let Some(tok) = self.select(self.current + consumed) {
                    consumed += 1;
                    let is_open = tok.kind == TokenKind::AngleStart
                        || (tok.kind == TokenKind::Operator && tok.value == "<");
                    let is_close = tok.kind == TokenKind::AngleEnd
                        || (tok.kind == TokenKind::Operator && tok.value == ">");
                    if is_open {
                        depth += 1;
                        generics.push('<');
                    } else if is_close {
                        depth -= 1;
                        generics.push('>');
                        if depth == 0 {
                            break;
                        }
                    } else {
                        generics.push_str(&tok.value);
                        if tok.value == "," {
                            generics.push(' ');
                        }
                    }
                }
            }

            let vis = if self.pending_pub {
                self.pending_pub = false;
                "pub "
            } else {
                ""
            };
            self.append(
                &format!("{}struct {}{} {{", vis, struct_name, generics),
                AppendMode::ForceAppendWithSpace,
            );

            // Processes struct fields
            while let Some(tok) = self.select(self.current + consumed) {
                consumed += 1;
                if tok.kind == TokenKind::BraceStart {
                    break;
                }
            }

            // Collect fields until the closing brace
            let mut brace_count = 1;
            let mut current_field = String::new();
            let mut in_field_name = true;
            // True once the type part of the current field has been emitted.
            // A following name-token (Identifier in name position) then starts a
            // NEW field even with no comma/newline separator (e.g. same-line
            // `x: int  y: int`); without this the fields fuse into `x:i64y:`.
            let mut seen_type = false;

            // For reflect codegen: ordered field names of this struct. A struct
            // with generic params is skipped (its `impl reflect::Reflect` would
            // need generic bounds — out of MVP scope).
            let mut reflect_fields: Vec<String> = Vec::new();
            let has_generics = !generics.is_empty();

            while brace_count > 0 && consumed < self.tokens.len() - self.current {
                if let Some(tok) = self.select(self.current + consumed) {
                    consumed += 1;
                    match tok.kind {
                        TokenKind::BraceStart => brace_count += 1,
                        TokenKind::BraceEnd => {
                            brace_count -= 1;
                            if brace_count == 0 {
                                if !current_field.trim().is_empty() {
                                    if let Some(name) =
                                        Self::reflect_field_name(&current_field)
                                    {
                                        reflect_fields.push(name);
                                    }
                                    self.append(
                                        &format!("    {},", current_field.trim()),
                                        AppendMode::ForceAppendWithSpace,
                                    );
                                }
                                break;
                            }
                        }
                        TokenKind::Identifier => {
                            let ident_value = tok.value.clone();
                            // A name-position Identifier arriving after the
                            // current field's type was emitted (no comma/newline
                            // between) starts a NEW field on the same line. Flush
                            // the complete field first, then read this token as
                            // the new field name.
                            if !in_field_name
                                && seen_type
                                && !current_field.trim().is_empty()
                            {
                                if let Some(name) =
                                    Self::reflect_field_name(&current_field)
                                {
                                    reflect_fields.push(name);
                                }
                                self.append(
                                    &format!("    {},", current_field.trim()),
                                    AppendMode::ForceAppendWithSpace,
                                );
                                current_field.clear();
                                in_field_name = true;
                                seen_type = false;
                            }
                            if in_field_name {
                                current_field = ident_value;
                                in_field_name = false;
                            } else {
                                // This is a type
                                seen_type = true;
                                let (converted_type, data_type) =
                                    utils::convert_type_with_marking(&ident_value);

                                // Mark data type usage for struct fields
                                if let Some(dt) = data_type {
                                    self.uses_data_types = true;
                                    match dt.as_str() {
                                        "json" => self.result.mark_json_usage(),
                                        "xml" => self.result.mark_xml_usage(),
                                        "toml" => self.result.mark_toml_usage(),
                                        _ => {}
                                    }
                                }

                                current_field.push_str(&converted_type);
                            }
                        }
                        // The `:` separator may arrive as a dedicated `Colon`
                        // token or — depending on the surrounding tokens — as
                        // an `Operator(":")`. Treat both as the name→type
                        // boundary: reset `seen_type` so the type that follows
                        // is recognised.
                        TokenKind::Colon => {
                            current_field.push_str(": ");
                            seen_type = false;
                        }
                        TokenKind::Operator if tok.value == ":" => {
                            current_field.push_str(": ");
                            seen_type = false;
                        }
                        TokenKind::ParamType
                        | TokenKind::Type
                        | TokenKind::Json
                        | TokenKind::Xml
                        | TokenKind::Toml => {
                            seen_type = true;
                            let (converted_type, data_type) =
                                utils::convert_type_with_marking(&tok.value);

                            // Mark data type usage for struct/param types
                            if let Some(dt) = data_type {
                                self.uses_data_types = true;
                                match dt.as_str() {
                                    "json" => self.result.mark_json_usage(),
                                    "xml" => self.result.mark_xml_usage(),
                                    "toml" => self.result.mark_toml_usage(),
                                    _ => {}
                                }
                            }

                            current_field.push_str(&converted_type);

                            // Note: We don't mark data types usage just by declaring them in structs
                            // We'll only mark when actually using the types in operations
                            // match tok.kind {
                            //     TokenKind::Json => {
                            //         self.uses_data_types = true;
                            //         self.result.mark_json_usage();
                            //     },
                            //     TokenKind::Xml => {
                            //         self.uses_data_types = true;
                            //         self.result.mark_xml_usage();
                            //     },
                            //     TokenKind::Toml => {
                            //         self.uses_data_types = true;
                            //         self.result.mark_toml_usage();
                            //     },
                            //     _ => {}
                            // }
                        }
                        TokenKind::Comma => {
                            if !current_field.trim().is_empty() {
                                if let Some(name) = Self::reflect_field_name(&current_field) {
                                    reflect_fields.push(name);
                                }
                                self.append(
                                    &format!("    {},", current_field.trim()),
                                    AppendMode::ForceAppendWithSpace,
                                );
                                current_field.clear();
                                in_field_name = true;
                                seen_type = false;
                            }
                        }
                        TokenKind::Newline => {
                            // A newline separates struct fields in Copper
                            // (`first: T` / `second: T` on their own lines, no
                            // comma). Flush the accumulated field — without
                            // this, consecutive fields fuse into
                            // `first: Tsecond: T`. Only flush once a field is
                            // complete (name + type seen); a blank line or the
                            // newline right after `{` leaves nothing to emit.
                            if !in_field_name && !current_field.trim().is_empty() {
                                if let Some(name) = Self::reflect_field_name(&current_field) {
                                    reflect_fields.push(name);
                                }
                                self.append(
                                    &format!("    {},", current_field.trim()),
                                    AppendMode::ForceAppendWithSpace,
                                );
                                current_field.clear();
                                in_field_name = true;
                                seen_type = false;
                            }
                        }
                        _ => {
                            if !tok.value.trim().is_empty() && tok.value != " " {
                                // Type content that the dedicated arms above
                                // didn't catch — most notably a primitive-type
                                // keyword like `bool` (tokenized as `Keyword`).
                                // Mark the type as seen so a following bare
                                // field name flushes (`done: bool  priority:`).
                                if !in_field_name && current_field.contains(':') {
                                    seen_type = true;
                                }
                                current_field.push_str(&tok.value);
                            }
                        }
                    }
                }
            }

            self.append("}", AppendMode::ForceAppendWithSpace);
            self.append("\n", AppendMode::ForceAppend);
            self.is_inside_struct = false;
            self.current_struct = None;

            // Record this struct for reflect codegen (emitted at EOF iff the
            // `reflect` module is imported). Skip generic structs.
            if !has_generics {
                self.result.record_reflect_struct(&struct_name, reflect_fields);
            }

            return Consumed::consume(consumed.try_into().unwrap());
        }

        Consumed::consume(0)
    }

    /// Extract the bare field name from an accumulated `current_field`
    /// (`"name: Type"`), trimmed. Returns `None` if it has no name part.
    fn reflect_field_name(current_field: &str) -> Option<String> {
        let name = current_field
            .split(':')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    pub fn parse_impl_block(&mut self) -> Consumed {
        if self.value() == "impl" && self.kind() == TokenKind::Impl {
            // `impl Trait` used as a parameter type (after `:`), not an impl block.
            // Don't intercept it here — let parse_any add the space.
            if let Some(prev) = self.previous_significant() {
                if prev.kind == TokenKind::Colon || prev.value == "," {
                    return Consumed::consume(0);
                }
            }

            let mut consumed = 1; // count the 'impl'

            // Generics for impl (optional)
            let mut impl_generics = String::new();
            if let Some(tok) = self.select(self.current + consumed) {
                if tok.kind == TokenKind::AngleStart {
                    consumed += 1;
                    impl_generics.push('<');

                    while let Some(tok) = self.select(self.current + consumed) {
                        consumed += 1;
                        if tok.kind == TokenKind::AngleEnd {
                            impl_generics.push('>');
                            break;
                        }
                        impl_generics.push_str(&tok.value);
                        if tok.value == "," {
                            impl_generics.push(' ');
                        }
                    }
                }
            }

            // Name of the type being implemented
            let target_type = if let Some(tok) = self.select(self.current + consumed) {
                if tok.kind == TokenKind::Identifier {
                    consumed += 1;
                    tok.value.clone()
                } else {
                    return Consumed::consume(0);
                }
            } else {
                return Consumed::consume(0);
            };

            // Type generics (optional)
            let mut type_generics = String::new();
            if let Some(tok) = self.select(self.current + consumed) {
                if tok.kind == TokenKind::AngleStart {
                    consumed += 1;
                    type_generics.push('<');

                    while let Some(tok) = self.select(self.current + consumed) {
                        consumed += 1;
                        if tok.kind == TokenKind::AngleEnd {
                            type_generics.push('>');
                            break;
                        }
                        type_generics.push_str(&tok.value);
                        if tok.value == "," {
                            type_generics.push(' ');
                        }
                    }
                }
            }

            // Verifies if is impl or for
            if let Some(tok) = self.select(self.current + consumed) {
                if tok.value == "for" {
                    consumed += 1;

                    // The trait name must be before the "for"
                    let trait_name = target_type.clone();

                    if let Some(tok) = self.select(self.current + consumed) {
                        if tok.kind == TokenKind::Identifier {
                            consumed += 1;
                            let actual_target = tok.value.clone();
                            self.current_impl_target = Some(actual_target.clone());
                            self.append(
                                &format!(
                                    "impl{} {} for {}{} {{",
                                    impl_generics, trait_name, actual_target, type_generics
                                ),
                                AppendMode::ForceAppendWithSpace,
                            );
                        }
                    }
                } else {
                    self.current_impl_target = Some(target_type.clone());
                    self.append(
                        &format!("impl{} {}{} {{", impl_generics, target_type, type_generics),
                        AppendMode::ForceAppendWithSpace,
                    );
                }
            } else {
                self.current_impl_target = Some(target_type.clone());
                self.append(
                    &format!("impl{} {}{} {{", impl_generics, target_type, type_generics),
                    AppendMode::ForceAppendWithSpace,
                );
            }

            self.is_inside_impl = true;

            // Process impl block
            while let Some(tok) = self.select(self.current + consumed) {
                consumed += 1;
                if tok.kind == TokenKind::BraceStart {
                    break;
                }
            }

            // Collects methods until the closing brace
            let mut brace_count = 1;
            let mut method_tokens = Vec::new();

            while brace_count > 0 && consumed < self.tokens.len() - self.current {
                if let Some(tok) = self.select(self.current + consumed) {
                    consumed += 1;
                    match tok.kind {
                        TokenKind::BraceStart => brace_count += 1,
                        TokenKind::BraceEnd => {
                            brace_count -= 1;
                            if brace_count == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    method_tokens.push(tok.clone());
                }
            }

            self.process_impl_methods(&method_tokens);

            self.append("}", AppendMode::ForceAppendWithSpace);
            self.append("\n", AppendMode::ForceAppend);
            self.is_inside_impl = false;
            self.current_impl_target = None;

            return Consumed::consume(consumed.try_into().unwrap());
        }

        Consumed::consume(0)
    }

    /// Lower an impl-method body (the tokens *between* its `{` and `}`) to Rust
    /// by running them through a fresh sub-parser — the same statement machinery
    /// that handles free-function and top-level bodies, so `let` injection, `;`
    /// terminators, struct literals, `if`/`for` blocks and `self.field` access
    /// all lower correctly. The sub-parser wraps top-level statements in
    /// `fn main() { … }`; we return the inside of that block, re-indented one
    /// level deeper so it nests under the method signature.
    fn lower_method_body(body_tokens: Vec<Token>) -> String {
        if body_tokens.is_empty() {
            return String::new();
        }
        let mut sub = Parser::new(body_tokens);
        let raw = sub.parse();

        // Pull out the inside of the sub-parser's `fn main() { … }` wrapper.
        let inner = match raw.find("fn main() {") {
            Some(start) => {
                let after = &raw[start + "fn main() {".len()..];
                match after.rfind('}') {
                    Some(end) => after[..end].trim_matches('\n'),
                    None => after.trim_matches('\n'),
                }
            }
            None => raw.trim_matches('\n'),
        };

        // Re-indent each non-empty line by 4 spaces so the body sits under the
        // `    fn …{` signature (the sub-parser already indents one level).
        inner
            .lines()
            .map(|line| {
                if line.trim().is_empty() {
                    String::new()
                } else {
                    format!("    {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn process_impl_methods(&mut self, tokens: &[Token]) {
        let mut i = 0;
        while i < tokens.len() {
            // Looks for function definitions: [pub] func type name(params) or [pub] fn name(params) -> type
            if (tokens[i].value == "pub"
                && i + 1 < tokens.len()
                && (tokens[i + 1].value == "func" || tokens[i + 1].value == "fn"))
                || tokens[i].value == "func"
                || tokens[i].value == "fn"
            {
                let start_idx = if tokens[i].value == "pub" { i } else { i };
                let is_copper_func = tokens[start_idx].value == "func"
                    || (tokens[start_idx].value == "pub"
                        && start_idx + 1 < tokens.len()
                        && tokens[start_idx + 1].value == "func");
                let fn_idx = if tokens[i].value == "pub" { i + 1 } else { i };

                let is_pub = tokens[start_idx].value == "pub";

                if is_copper_func {
                    // Copper Syntax: [pub] func type name(params)
                    if fn_idx + 2 >= tokens.len() {
                        i += 1;
                        continue;
                    }

                    let return_type_token = &tokens[fn_idx + 1];

                    // Gobble any generic arguments that follow the base return
                    // type (`Option<GameObject>`, `Result<T, E>`, `Vec<int>`).
                    // The tokenizer splits these into separate `<` / args / `>`
                    // tokens (angle brackets lex as `Operator`, see the parser's
                    // ReturnType arm). Without collecting them the method name
                    // would appear to be `<` and the whole method would be
                    // silently dropped.
                    let is_open = |t: &Token| {
                        t.kind == TokenKind::AngleStart
                            || (t.kind == TokenKind::Operator && t.value == "<")
                    };
                    let is_close = |t: &Token| {
                        t.kind == TokenKind::AngleEnd
                            || (t.kind == TokenKind::Operator && t.value == ">")
                    };
                    let mut name_idx = fn_idx + 2;
                    let mut generic_suffix = String::new();
                    if name_idx < tokens.len() && is_open(&tokens[name_idx]) {
                        let mut depth = 0usize;
                        while name_idx < tokens.len() {
                            let t = &tokens[name_idx];
                            if is_open(t) {
                                depth += 1;
                                generic_suffix.push('<');
                            } else if is_close(t) {
                                depth -= 1;
                                generic_suffix.push('>');
                                name_idx += 1;
                                if depth == 0 {
                                    break;
                                }
                                continue;
                            } else {
                                generic_suffix.push_str(&t.value);
                            }
                            name_idx += 1;
                        }
                    }

                    if name_idx >= tokens.len() || tokens[name_idx].kind != TokenKind::Identifier {
                        i += 1;
                        continue;
                    }
                    let method_name_token = &tokens[name_idx];

                    let (mut return_type, data_type) =
                        utils::convert_type_with_marking(&return_type_token.value);
                    return_type.push_str(&generic_suffix);

                    // Mark data type usage for return types
                    if let Some(dt) = data_type {
                        self.uses_data_types = true;
                        match dt.as_str() {
                            "json" => self.result.mark_json_usage(),
                            "xml" => self.result.mark_xml_usage(),
                            "toml" => self.result.mark_toml_usage(),
                            _ => {}
                        }
                    }

                    let method_name = &method_name_token.value;

                    // Find parameters
                    let mut param_start = name_idx + 1;
                    while param_start < tokens.len()
                        && tokens[param_start].kind != TokenKind::ParenthesesStart
                    {
                        param_start += 1;
                    }

                    if param_start >= tokens.len() {
                        i += 1;
                        continue;
                    }

                    let mut param_end = param_start + 1;
                    let mut paren_count = 1;
                    while param_end < tokens.len() && paren_count > 0 {
                        match tokens[param_end].kind {
                            TokenKind::ParenthesesStart => paren_count += 1,
                            TokenKind::ParametersEnd => paren_count -= 1,
                            _ => {}
                        }
                        param_end += 1;
                    }

                    // Finds body start
                    let mut body_start = param_end;
                    while body_start < tokens.len()
                        && tokens[body_start].kind != TokenKind::BraceStart
                    {
                        body_start += 1;
                    }

                    if body_start >= tokens.len() {
                        i += 1;
                        continue;
                    }

                    let mut body_end = body_start + 1;
                    let mut brace_count = 1;
                    while body_end < tokens.len() && brace_count > 0 {
                        match tokens[body_end].kind {
                            TokenKind::BraceStart => brace_count += 1,
                            TokenKind::BraceEnd => brace_count -= 1,
                            _ => {}
                        }
                        body_end += 1;
                    }

                    // Process parameters
                    let param_tokens: Vec<&Token> = tokens[param_start + 1..param_end - 1]
                        .iter()
                        .filter(|t| t.kind != TokenKind::Newline)
                        .collect();

                    let mut params = Vec::new();
                    let mut i_param = 0;

                    while i_param < param_tokens.len() {
                        // Detect `&` or `&mut` prefix for reference/borrow params
                        // (e.g. `&self`, `&mut self`, `&mut name: Type`).
                        let mut prefix = String::new();
                        if i_param < param_tokens.len()
                            && param_tokens[i_param].kind == TokenKind::Operator
                            && param_tokens[i_param].value == "&"
                        {
                            prefix.push('&');
                            i_param += 1;
                            if i_param < param_tokens.len() && param_tokens[i_param].value == "mut"
                            {
                                prefix.push_str("mut ");
                                i_param += 1;
                            }
                        }

                        if i_param < param_tokens.len()
                            && (param_tokens[i_param].kind == TokenKind::Param
                                || param_tokens[i_param].kind == TokenKind::Identifier
                                || param_tokens[i_param].value == "self"
                                || param_tokens[i_param].value == "mut")
                        {
                            // Handle `mut name` binding modifier
                            let mut binding_prefix = String::new();
                            if param_tokens[i_param].value == "mut" {
                                binding_prefix.push_str("mut ");
                                i_param += 1;
                            }

                            if i_param >= param_tokens.len() {
                                break;
                            }
                            let param_name = format!(
                                "{}{}{}",
                                prefix, binding_prefix, param_tokens[i_param].value
                            );

                            // Check if it has a type annotation (`name: Type`)
                            if i_param + 2 < param_tokens.len()
                                && param_tokens[i_param + 1].kind == TokenKind::Colon
                            {
                                let (mut param_type, data_type) = utils::convert_type_with_marking(
                                    &param_tokens[i_param + 2].value,
                                );

                                // Gobble generic arguments on the param type too
                                // (`Vec<GameObject>`, `Option<i32>`), mirroring
                                // the return-type handling above. Without this
                                // the `<...>` tokens leak and the comma inside
                                // `HashMap<K, V>` is mistaken for a param break.
                                let mut k = i_param + 3;
                                if k < param_tokens.len() && is_open(param_tokens[k]) {
                                    let mut depth = 0usize;
                                    while k < param_tokens.len() {
                                        let t = param_tokens[k];
                                        if is_open(t) {
                                            depth += 1;
                                            param_type.push('<');
                                        } else if is_close(t) {
                                            depth -= 1;
                                            param_type.push('>');
                                            k += 1;
                                            if depth == 0 {
                                                break;
                                            }
                                            continue;
                                        } else {
                                            param_type.push_str(&t.value);
                                        }
                                        k += 1;
                                    }
                                }

                                if let Some(dt) = data_type {
                                    self.uses_data_types = true;
                                    match dt.as_str() {
                                        "json" => self.result.mark_json_usage(),
                                        "xml" => self.result.mark_xml_usage(),
                                        "toml" => self.result.mark_toml_usage(),
                                        _ => {}
                                    }
                                }

                                params.push(format!("{}: {}", param_name, param_type));
                                i_param = k;
                            } else {
                                // Receiver-only param: `self`, `&self`, `&mut self`.
                                // A bare `self` borrows (`&self`) — matching the
                                // `class` path and the convention across Copper
                                // code (read-only accessors). Methods that need
                                // to consume the receiver write `&mut self` /
                                // an explicit owned form. `&self` / `&mut self`
                                // already carry their prefix and pass through.
                                if param_name == "self" {
                                    params.push("&self".to_string());
                                } else {
                                    params.push(param_name);
                                }
                                i_param += 1;
                            }

                            // Skip trailing comma
                            if i_param < param_tokens.len()
                                && param_tokens[i_param].kind == TokenKind::Comma
                            {
                                i_param += 1;
                            }
                        } else {
                            i_param += 1;
                        }
                    }

                    // Lower the body through a fresh sub-parser — the same
                    // statement machinery free functions use — so multi-statement
                    // bodies get `let` injection, `;` terminators, struct
                    // literals and nested `if`/blocks right. Newlines are kept
                    // (they carry the statement boundaries); the old naive
                    // token-join dropped them and collapsed the body to one line.
                    let body_tokens: Vec<Token> = tokens[body_start + 1..body_end - 1].to_vec();
                    let body_str = Self::lower_method_body(body_tokens);

                    // Generate method using Rust syntax
                    let visibility = if is_pub { "pub " } else { "" };
                    let param_str = params.join(", ");
                    let sig = if return_type == "()" {
                        format!("    {}fn {}({}) {{", visibility, method_name, param_str)
                    } else {
                        format!(
                            "    {}fn {}({}) -> {} {{",
                            visibility, method_name, param_str, return_type
                        )
                    };
                    self.append(&sig, AppendMode::ForceAppendWithSpace);
                    if !body_str.is_empty() {
                        self.append(&body_str, AppendMode::ForceAppendWithSpace);
                    }
                    self.append("    }", AppendMode::ForceAppendWithSpace);

                    i = body_end;
                } else {
                    // Sintaxe Rust: [pub] fn nome(params) -> tipo
                    if fn_idx + 1 >= tokens.len()
                        || tokens[fn_idx + 1].kind != TokenKind::Identifier
                    {
                        i += 1;
                        continue;
                    }

                    let method_name = &tokens[fn_idx + 1].value;

                    // Find parameters
                    let mut param_start = fn_idx + 2;
                    while param_start < tokens.len()
                        && tokens[param_start].kind != TokenKind::ParenthesesStart
                    {
                        param_start += 1;
                    }

                    if param_start >= tokens.len() {
                        i += 1;
                        continue;
                    }

                    let mut param_end = param_start + 1;
                    let mut paren_count = 1;
                    while param_end < tokens.len() && paren_count > 0 {
                        match tokens[param_end].kind {
                            TokenKind::ParenthesesStart => paren_count += 1,
                            TokenKind::ParametersEnd => paren_count -= 1,
                            _ => {}
                        }
                        param_end += 1;
                    }

                    // Find return type
                    let mut return_type = "()".to_string();
                    let mut body_start = param_end;

                    if body_start < tokens.len() && tokens[body_start].value == "->" {
                        body_start += 1;
                        if body_start < tokens.len() {
                            return_type = convert_type(&tokens[body_start].value);
                            body_start += 1;
                        }
                    }

                    // Find function body
                    while body_start < tokens.len()
                        && tokens[body_start].kind != TokenKind::BraceStart
                    {
                        body_start += 1;
                    }

                    if body_start >= tokens.len() {
                        i += 1;
                        continue;
                    }

                    let mut body_end = body_start + 1;
                    let mut brace_count = 1;
                    while body_end < tokens.len() && brace_count > 0 {
                        match tokens[body_end].kind {
                            TokenKind::BraceStart => brace_count += 1,
                            TokenKind::BraceEnd => brace_count -= 1,
                            _ => {}
                        }
                        body_end += 1;
                    }

                    // Extract parameters
                    let param_tokens: Vec<&Token> = tokens[param_start + 1..param_end - 1]
                        .iter()
                        .filter(|t| t.kind != TokenKind::Newline)
                        .collect();

                    let mut params = Vec::new();
                    let mut current_param = String::new();

                    for token in param_tokens {
                        match token.kind {
                            TokenKind::Comma => {
                                if !current_param.trim().is_empty() {
                                    params.push(current_param.trim().to_string());
                                    current_param.clear();
                                }
                            }
                            TokenKind::ParamType => {
                                current_param.push_str(&convert_type(&token.value));
                            }
                            _ => {
                                if !token.value.trim().is_empty() {
                                    if !current_param.is_empty()
                                        && !current_param.ends_with(' ')
                                        && !token.value.starts_with(':')
                                    {
                                        current_param.push(' ');
                                    }
                                    current_param.push_str(&token.value);
                                }
                            }
                        }
                    }

                    if !current_param.trim().is_empty() {
                        params.push(current_param.trim().to_string());
                    }

                    // Extract body
                    let body_tokens: Vec<&Token> = tokens[body_start + 1..body_end - 1]
                        .iter()
                        .filter(|t| t.kind != TokenKind::Newline || !t.value.trim().is_empty())
                        .collect();

                    let mut body_str = String::new();
                    for (idx, token) in body_tokens.iter().enumerate() {
                        if idx > 0 && !body_str.ends_with(' ') && !token.value.starts_with(';') {
                            body_str.push(' ');
                        }
                        body_str.push_str(&token.value);
                    }

                    // Generate method
                    let visibility = if is_pub { "pub " } else { "" };
                    let param_str = params.join(", ");

                    if return_type == "()" {
                        self.append(
                            &format!("    {}fn {}({}) {{", visibility, method_name, param_str),
                            AppendMode::ForceAppendWithSpace,
                        );
                        self.append(
                            &format!("        {}", body_str),
                            AppendMode::ForceAppendWithSpace,
                        );
                        self.append("    }", AppendMode::ForceAppendWithSpace);
                    } else {
                        self.append(
                            &format!(
                                "    {}fn {}({}) -> {} {{",
                                visibility, method_name, param_str, return_type
                            ),
                            AppendMode::ForceAppendWithSpace,
                        );
                        self.append(
                            &format!("        {}", body_str),
                            AppendMode::ForceAppendWithSpace,
                        );
                        self.append("    }", AppendMode::ForceAppendWithSpace);
                    }

                    i = body_end;
                }
            } else {
                i += 1;
            }
        }
    }

    /// `enum Name { Variant, Variant(T), ... }` — route to module level.
    pub fn parse_enum_definition(&mut self) -> Consumed {
        if self.value() != "enum" {
            return Consumed::consume(0);
        }
        let mut consumed = 1;

        // Optional generics: `enum Foo<T> { ... }` — skip for now, collect name only.
        let name = match self.select(self.current + consumed) {
            Some(t) if t.kind == TokenKind::Identifier => {
                consumed += 1;
                t.value.clone()
            }
            _ => return Consumed::consume(0),
        };

        // Skip to opening `{`
        while let Some(t) = self.select(self.current + consumed) {
            consumed += 1;
            if t.kind == TokenKind::BraceStart {
                break;
            }
        }

        // Collect body tokens until the matching `}`
        let mut depth = 1usize;
        let mut body = String::new();
        while let Some(t) = self.select(self.current + consumed) {
            consumed += 1;
            match t.kind {
                TokenKind::BraceStart => {
                    depth += 1;
                    body.push_str(" {");
                }
                TokenKind::BraceEnd => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    body.push('}');
                }
                TokenKind::Newline => {}
                _ => body.push_str(&t.value),
            }
        }

        self.result
            .force_append(&format!("enum {name} {{{body}}}\n"), false);
        Consumed::consume(consumed as isize)
    }

    /// `trait Name { func ReturnType method(params) }` — route to module level.
    pub fn parse_trait_definition(&mut self) -> Consumed {
        if self.value() != "trait" && self.kind() != TokenKind::Trait {
            return Consumed::consume(0);
        }
        let mut consumed = 1;

        let name = match self.select(self.current + consumed) {
            Some(t) if t.kind == TokenKind::Identifier => {
                consumed += 1;
                t.value.clone()
            }
            _ => return Consumed::consume(0),
        };

        // Skip to opening `{`
        while let Some(t) = self.select(self.current + consumed) {
            consumed += 1;
            if t.kind == TokenKind::BraceStart {
                break;
            }
        }

        // Collect all tokens until matching `}`
        let mut depth = 1usize;
        let mut body_tokens: Vec<Token> = Vec::new();
        while let Some(t) = self.select(self.current + consumed) {
            consumed += 1;
            match t.kind {
                TokenKind::BraceStart => {
                    depth += 1;
                    body_tokens.push(t.clone());
                }
                TokenKind::BraceEnd => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    body_tokens.push(t.clone());
                }
                _ => body_tokens.push(t.clone()),
            }
        }

        // Convert `func ReturnType name(params)` → `fn name(params) -> ReturnType;`
        let mut methods = String::new();
        let mut j = 0;
        while j < body_tokens.len() {
            let tok = &body_tokens[j];
            if tok.value == "func" {
                // func ReturnType name(params)
                if j + 2 >= body_tokens.len() {
                    j += 1;
                    continue;
                }
                let ret_tok = &body_tokens[j + 1];
                let nm_tok = &body_tokens[j + 2];
                if nm_tok.kind != TokenKind::Identifier {
                    j += 1;
                    continue;
                }
                let (ret_type, _) = utils::convert_type_with_marking(&ret_tok.value);
                let method_name = &nm_tok.value;
                j += 3;
                // Collect params
                while j < body_tokens.len() && body_tokens[j].kind != TokenKind::ParenthesesStart {
                    j += 1;
                }
                let mut params_src = String::from("(");
                let mut pdepth = 1usize;
                j += 1;
                while j < body_tokens.len() && pdepth > 0 {
                    match body_tokens[j].kind {
                        TokenKind::ParenthesesStart => {
                            pdepth += 1;
                            params_src.push('(');
                        }
                        TokenKind::ParenthesesEnd | TokenKind::ParametersEnd => {
                            pdepth -= 1;
                            if pdepth == 0 {
                                params_src.push(')');
                                break;
                            }
                            params_src.push(')');
                        }
                        TokenKind::Newline => {}
                        _ => params_src.push_str(&body_tokens[j].value),
                    }
                    j += 1;
                }
                j += 1;
                let ret_str = if ret_type == "()" {
                    String::new()
                } else {
                    format!(" -> {ret_type}")
                };
                methods.push_str(&format!("    fn {method_name}{params_src}{ret_str};\n"));
            } else {
                j += 1;
            }
        }

        self.result
            .force_append(&format!("trait {name} {{\n{methods}}}\n"), false);
        Consumed::consume(consumed as isize)
    }

    pub fn get_required_dependencies(&self) -> Vec<String> {
        self.result.get_required_dependencies()
    }

    /// Tokenize + parse `std/cstd.crs` (the Copper-written stdlib),
    /// upgrade every top-level `fn` to `pub fn`, and wrap the lot in
    /// `pub mod cstd { ... }` so user code can `use cstd::{input};`.
    fn transpile_cstd_module() -> String {
        Self::transpile_std_module("cstd", CSTD_SOURCE, CSTD_NATIVE)
    }

    /// True for native std modules selectable via `import { ... } from <name>`
    /// (besides cstd, which keeps its own dedicated path).
    fn is_native_std_module(name: &str) -> bool {
        matches!(name, "net" | "http" | "url" | "json" | "crypto" | "time" | "fs" | "ws" | "reflect")
    }

    /// (crs surface, native helpers) for a native std module name.
    fn std_module_sources(name: &str) -> Option<(&'static str, &'static str)> {
        match name {
            "net" => Some((NET_SOURCE, NET_NATIVE)),
            "http" => Some((HTTP_SOURCE, HTTP_NATIVE)),
            "url" => Some((URL_SOURCE, URL_NATIVE)),
            "json" => Some((JSON_SOURCE, JSON_NATIVE)),
            "crypto" => Some((CRYPTO_SOURCE, CRYPTO_NATIVE)),
            "time" => Some((TIME_SOURCE, TIME_NATIVE)),
            "fs" => Some((FS_SOURCE, FS_NATIVE)),
            "ws" => Some((WS_SOURCE, WS_NATIVE)),
            "reflect" => Some((REFLECT_SOURCE, REFLECT_NATIVE)),
            _ => None,
        }
    }

    /// Transpile a Copper-written std module, promote its top-level `fn`s to
    /// `pub fn`, and wrap it with the native helper block in
    /// `pub mod <mod_name> { ... }`.
    fn transpile_std_module(mod_name: &str, crs_source: &str, native: &str) -> String {
        let mut tk = Tokenizer::new(crs_source.to_string());
        let tokens = tk.tokenize();
        let mut sub = Parser::new(tokens);
        let raw = sub.parse();

        let body = raw.replace("fn main() {}", "").trim_end().to_string();

        let promoted = body
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                if let Some(rest) = trimmed.strip_prefix("fn ") {
                    let indent_len = line.len() - trimmed.len();
                    format!("{}pub fn {}", &line[..indent_len], rest)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "#[allow(dead_code)]\npub mod {} {{\n{}\n\n{}\n}}\n",
            mod_name, promoted, native
        )
    }

    /// Transpile + prepend every native std module the program imported.
    fn prepend_used_std_modules(&mut self) {
        for name in self.result.used_std_modules() {
            if let Some((crs, native)) = Self::std_module_sources(&name) {
                let module = Self::transpile_std_module(&name, crs, native);
                self.result.prepend_cstd_module(&module);
            }
        }
        // When `reflect` is imported, auto-derive `impl reflect::Reflect` for
        // every recorded (non-generic) struct, appended after the struct defs.
        self.result.append_reflect_impls();
    }

    pub fn parse(&mut self) -> String {
        loop {
            if self.eof {
                // Only add aliases if actually using data types
                if self.uses_data_types {
                    self.result.add_data_type_aliases();
                }
                if self.result.cstd_is_used() {
                    let module = Self::transpile_cstd_module();
                    self.result.prepend_cstd_module(&module);
                }
                self.prepend_used_std_modules();
                self.result.write_main_function();
                break self.result.get().expect("Format Error");
            }

            let token_info = self
                .current()
                .map(|t| (t.kind, t.value.clone(), t.struct_brace));
            if let Some((dispatched_kind, dispatched_value, dispatched_struct_brace)) = token_info {
                // Optional-chaining bookkeeping. Done before dispatch so the
                // closing `)` of any open chain is emitted *before* the token
                // that ends the chain (operator, separator, or a closing
                // delimiter that's about to leave our depth).
                self.maybe_close_optional_chains_for(dispatched_kind, &dispatched_value);

                match dispatched_kind {
                    TokenKind::Eof => {
                        self.eof = true;
                    }
                    TokenKind::DocComment => {
                        // `///` / `//!` doc comments are preserved verbatim so
                        // rustdoc sees them. Force them to the module stream so a
                        // top-level doc lands right before its item (a fn/struct
                        // also emits there), then swallow the trailing newline so
                        // no stray `;` separates the doc from the item.
                        self.append(&format!("{}\n", self.value()), AppendMode::ForceAppend);
                        self.next();
                        while self.kind() == TokenKind::Newline {
                            self.next();
                        }
                    }
                    TokenKind::In => {
                        // `in` sits between an identifier (e.g. the loop
                        // variable) and an expression, so it needs spaces on
                        // both sides regardless of what came before.
                        self.append(" in ", AppendMode::Append);
                        self.next();
                    }
                    TokenKind::OptionalChain => {
                        self.parse_optional_chain().consume_var(&mut self.current);
                    }
                    TokenKind::InterpolatedString => {
                        self.parse_interpolated_string()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    }
                    TokenKind::Identifier
                    | TokenKind::Keyword
                    | TokenKind::For
                    | TokenKind::Loop
                    | TokenKind::While
                    | TokenKind::Break
                    | TokenKind::Continue => {
                        self.parse_mut()
                            .or(|| self.parse_var())
                            .or(|| self.parse_type_declaration())
                            .or(|| self.parse_class_definition())
                            .or(|| self.parse_struct_definition())
                            .or(|| self.parse_impl_block())
                            .or(|| self.parse_enum_definition())
                            .or(|| self.parse_function())
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    }
                    TokenKind::ParametersEnd | TokenKind::ParametersStart => {
                        self.parse_function_params().consume_var(&mut self.current);
                    }
                    TokenKind::BracketStart => {
                        self.parse_bracket().consume_var(&mut self.current);
                    }
                    TokenKind::ParamType => {
                        self.append(&convert_type(&self.value()), AppendMode::Append);
                        self.next();
                    }
                    TokenKind::Json | TokenKind::Xml | TokenKind::Toml => {
                        // Note: We only convert the type, but don't mark as used yet
                        // Will be marked when actually used in operations
                        self.append(&convert_type(&self.value()), AppendMode::Append);
                        self.next();
                    }
                    TokenKind::ReturnType => {
                        // Capture the full return type, including any generic
                        // arguments that immediately follow (e.g.
                        // `Result<i32, ParseIntError>`). Without this the
                        // parser would only take the bare name and let the
                        // `<...>` leak into the function signature.
                        //
                        // Generic brackets are tokenized as `Operator("<")` /
                        // `Operator(">")` (the comparison-sign rule fires
                        // before the angle-bracket rule), not `AngleStart` /
                        // `AngleEnd`, so we discriminate by value here.
                        let mut full = convert_type(&self.value());
                        let mut consumed = 1;

                        let next_is_open_generic = matches!(
                            self.select(self.current + consumed),
                            Some(t)
                                if (t.kind == TokenKind::AngleStart)
                                    || (t.kind == TokenKind::Operator && t.value == "<")
                        );

                        if next_is_open_generic {
                            full.push('<');
                            consumed += 1;
                            let mut depth: usize = 1;
                            while let Some(t) = self.select(self.current + consumed) {
                                consumed += 1;
                                let is_open = (t.kind == TokenKind::AngleStart)
                                    || (t.kind == TokenKind::Operator && t.value == "<");
                                let is_close = (t.kind == TokenKind::AngleEnd)
                                    || (t.kind == TokenKind::Operator && t.value == ">");
                                if is_open {
                                    depth += 1;
                                    full.push('<');
                                } else if is_close {
                                    depth -= 1;
                                    full.push('>');
                                    if depth == 0 {
                                        break;
                                    }
                                } else {
                                    full.push_str(&t.value);
                                }
                            }
                        }

                        self.result.return_type(full);
                        for _ in 0..consumed {
                            self.next();
                        }
                    }
                    TokenKind::BraceStart | TokenKind::BraceEnd => {
                        self.parse_import()
                            .or(|| self.parse_function_body())
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    }
                    TokenKind::From
                    | TokenKind::ModuleVar
                    | TokenKind::ModulePath
                    | TokenKind::Import => {
                        self.parse_import()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    }
                    TokenKind::Comma if self.seen_import && self.is_import_list => {
                        self.parse_import()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    }
                    TokenKind::Operator => {
                        self.parse_operator()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    }
                    TokenKind::Regex => {
                        self.parse_regex()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    }
                    TokenKind::Struct => {
                        self.parse_struct_definition()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    }
                    TokenKind::Impl => {
                        self.parse_impl_block()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    }
                    TokenKind::Trait => {
                        self.parse_trait_definition()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    }
                    TokenKind::Attribute => {
                        // Attributes always live at module level — force them
                        // there regardless of current function context, and
                        // swallow the trailing newline so no stray `;` lands
                        // between the attribute and its item.
                        self.append(&self.value(), AppendMode::ForceAppend);
                        self.result.force_append("\n", false);
                        self.next();
                        while self.kind() == TokenKind::Newline {
                            self.next();
                        }
                    }
                    // `(a, b) = expr` — try tuple destructuring before
                    // falling back to a plain parenthesised expression.
                    TokenKind::ParenthesesStart => {
                        self.parse_tuple_destructure()
                            .or(|| {
                                self.append(&self.value(), AppendMode::Append);
                                Consumed::consume(1)
                            })
                            .consume_var(&mut self.current);
                    }
                    TokenKind::String => {
                        // A plain string literal as a DIRECT struct-literal
                        // field value is `&str`, but the field is typically
                        // `String`. Emit `"..".into()` so Rust coerces it to the
                        // field's type (works for `String` and `&str`). Only
                        // inside a struct literal at the field-value depth — a
                        // bare `mut name = "Brian"` (no struct) stays `&str`.
                        let coerce = matches!(
                            self.struct_lit_stack.last(),
                            Some(Some(d)) if *d == self.chain_delim_depth
                        );
                        if coerce {
                            self.append(
                                &format!("{}.into()", self.value()),
                                AppendMode::Append,
                            );
                        } else {
                            self.append(&self.value(), AppendMode::Append);
                        }
                        self.next();
                    }
                    _ => {
                        self.append(&self.value(), AppendMode::Append);
                        self.next();
                    }
                }

                // After dispatch, update the chain-tracking depth based on
                // the kind we just processed. Opens go up, closes go down.
                match dispatched_kind {
                    TokenKind::ParenthesesStart
                    | TokenKind::ParametersStart
                    | TokenKind::BracketStart
                    | TokenKind::BraceStart => {
                        self.chain_delim_depth += 1;
                    }
                    TokenKind::ParenthesesEnd
                    | TokenKind::ParametersEnd
                    | TokenKind::BracketEnd
                    | TokenKind::BraceEnd
                        if self.chain_delim_depth > 0 =>
                    {
                        self.chain_delim_depth -= 1;
                    }
                    _ => {}
                }

                // Track struct-literal nesting so a string literal in a field
                // value can be coerced (`"Ana".into()`). The tokenizer flagged
                // the `{`/`}` of a struct literal with `struct_brace`. Record
                // the depth just inside the brace so we only coerce DIRECT field
                // values, not strings nested in a call/array argument.
                match dispatched_kind {
                    TokenKind::BraceStart => {
                        self.struct_lit_stack.push(if dispatched_struct_brace {
                            Some(self.chain_delim_depth)
                        } else {
                            None
                        });
                    }
                    TokenKind::BraceEnd => {
                        self.struct_lit_stack.pop();
                    }
                    _ => {}
                }
            } else {
                if self.result.cstd_is_used() {
                    let module = Self::transpile_cstd_module();
                    self.result.prepend_cstd_module(&module);
                }
                self.prepend_used_std_modules();
                self.result.write_main_function();
                break self.result.get().expect("Format Error");
            }

            self.check_eof();
        }
    }
}

pub fn parse(tokens: Vec<Token>) -> String {
    let mut parser = Parser::new(tokens);
    parser.parse()
}

/// True when seeing this token (at the same depth as an open optional chain)
/// should close that chain. The chain is closed *before* the breaker is
/// emitted, so e.g. `obj?.foo + 1` becomes `obj.as_ref().map(|c| c.foo) + 1`.
///
/// Rules:
/// * `.` / `?.` / `(` / `[` keep the chain open — they're how method-call,
///   field-access, indexing, and nested optional-chaining continue.
/// * Closing delimiters at chain depth mean we're leaving the enclosing
///   scope, so the chain ends here.
/// * Statement separators and `{` (block start) end the chain.
/// * Any other operator ends the chain (it's the start of a binary op, the
///   try operator `?`, etc.).
fn is_chain_breaker(kind: TokenKind, value: &str) -> bool {
    match kind {
        TokenKind::Dot => false,
        TokenKind::OptionalChain => false,
        TokenKind::ParenthesesStart | TokenKind::ParametersStart | TokenKind::BracketStart => false,
        // Identifiers/literals can follow `.method`, so they continue.
        TokenKind::Identifier
        | TokenKind::Number
        | TokenKind::String
        | TokenKind::InterpolatedString
        | TokenKind::Keyword
        | TokenKind::Param
        | TokenKind::ParamType => false,
        // Closing delimiters at chain depth: chain ends before depth drops.
        TokenKind::ParenthesesEnd
        | TokenKind::ParametersEnd
        | TokenKind::BracketEnd
        | TokenKind::BraceEnd => true,
        // Block start, separators.
        TokenKind::BraceStart
        | TokenKind::Comma
        | TokenKind::Semicolon
        | TokenKind::Newline
        | TokenKind::Eof => true,
        // Comparison angles.
        TokenKind::AngleStart | TokenKind::AngleEnd | TokenKind::Range => true,
        // Operators: `.` is already filtered above; everything else (`+`,
        // `-`, `=`, `==`, `?` try, …) ends the chain.
        TokenKind::Operator => value != ".",
        _ => false,
    }
}
