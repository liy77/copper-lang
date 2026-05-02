//! Rewrites JS-style ternary expressions (`cond ? then : else`) into Rust's
//! `if cond { then } else { else }` form at the token-stream level, before
//! the main parser runs.
//!
//! Doing this as a token rewrite means the rest of the parser doesn't have
//! to know ternaries exist — it just sees a normal `if`/`else` block.
//!
//! Disambiguation:
//! * `obj?.field` is already a single [`TokenKind::OptionalChain`] token, so
//!   it never reaches this pass.
//! * `expr?` (try operator) leaves `?` as a plain [`TokenKind::Operator`]
//!   with no matching `:` at the same delimiter depth — those are skipped.
//! * Only `?` operators that *do* have a matching top-level `:` are
//!   rewritten.
//!
//! The walk-back for the condition stops at:
//! * statement separators (`;`, newline, `,`) at depth 0,
//! * the matching open delimiter of the enclosing scope,
//! * an assignment-style operator (`=`, `+=`, ...) or a `:` at depth 0,
//! so that `let x = a > 0 ? "p" : "n"` or `f(a, b ? c : d)` both pick the
//! intuitive condition.

use crate::tokenizer::{
    kind::TokenKind,
    tokens::{Data, Token},
};

pub(super) fn rewrite(mut tokens: Vec<Token>) -> Vec<Token> {
    // Iteratively rewrite the first ternary we find. Each rewrite replaces
    // a `?`/`:` pair with `if`/`{`/`}`/`else`/`{`/`}`, eliminating one
    // ternary per pass. Nested ternaries are picked up in subsequent
    // iterations because the inner `?`/`:` survive the outer rewrite
    // unchanged.
    loop {
        match find_first_ternary(&tokens) {
            Some(found) => {
                tokens = apply_rewrite(tokens, found);
            }
            None => break,
        }
    }
    tokens
}

struct Ternary {
    cond_start: usize,
    q_pos: usize,
    colon_pos: usize,
    else_end: usize, // inclusive
}

fn find_first_ternary(tokens: &[Token]) -> Option<Ternary> {
    for i in 0..tokens.len() {
        let tok = &tokens[i];
        if !(tok.kind == TokenKind::Operator && tok.value == "?") {
            continue;
        }
        let colon_pos = match find_matching_colon(tokens, i) {
            Some(p) => p,
            None => continue, // try operator, no matching colon
        };
        let cond_start = find_condition_start(tokens, i);
        let else_end = find_else_end(tokens, colon_pos);
        if cond_start >= i || else_end < colon_pos {
            // Defensive: malformed, skip rather than corrupt the stream.
            continue;
        }
        return Some(Ternary {
            cond_start,
            q_pos: i,
            colon_pos,
            else_end,
        });
    }
    None
}

fn find_matching_colon(tokens: &[Token], q_pos: usize) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut nested_q: i32 = 0;
    let mut j = q_pos + 1;
    while j < tokens.len() {
        let tok = &tokens[j];
        match tok.kind {
            TokenKind::ParenthesesStart
            | TokenKind::ParametersStart
            | TokenKind::BracketStart
            | TokenKind::BraceStart => {
                depth += 1;
            }
            TokenKind::ParenthesesEnd
            | TokenKind::ParametersEnd
            | TokenKind::BracketEnd
            | TokenKind::BraceEnd => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::Comma => {
                if depth == 0 {
                    return None;
                }
            }
            TokenKind::Operator if depth == 0 => {
                if tok.value == "?" {
                    nested_q += 1;
                } else if tok.value == ":" {
                    if nested_q > 0 {
                        nested_q -= 1;
                    } else {
                        return Some(j);
                    }
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

fn find_condition_start(tokens: &[Token], q_pos: usize) -> usize {
    let mut depth: i32 = 0;
    let mut j = q_pos;
    while j > 0 {
        j -= 1;
        let tok = &tokens[j];
        match tok.kind {
            TokenKind::ParenthesesEnd
            | TokenKind::ParametersEnd
            | TokenKind::BracketEnd
            | TokenKind::BraceEnd => {
                depth += 1;
            }
            TokenKind::ParenthesesStart
            | TokenKind::ParametersStart
            | TokenKind::BracketStart
            | TokenKind::BraceStart => {
                if depth == 0 {
                    return j + 1;
                }
                depth -= 1;
            }
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::Comma => {
                if depth == 0 {
                    return j + 1;
                }
            }
            TokenKind::Operator if depth == 0 => {
                if matches!(
                    tok.value.as_str(),
                    "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "^=" | "|=" | ":"
                ) {
                    return j + 1;
                }
            }
            _ => {}
        }
    }
    0
}

fn find_else_end(tokens: &[Token], colon_pos: usize) -> usize {
    let mut depth: i32 = 0;
    let mut j = colon_pos + 1;
    let mut last = j;
    while j < tokens.len() {
        let tok = &tokens[j];
        match tok.kind {
            TokenKind::ParenthesesStart
            | TokenKind::ParametersStart
            | TokenKind::BracketStart
            | TokenKind::BraceStart => {
                depth += 1;
            }
            TokenKind::ParenthesesEnd
            | TokenKind::ParametersEnd
            | TokenKind::BracketEnd
            | TokenKind::BraceEnd => {
                if depth == 0 {
                    return j.saturating_sub(1).max(last);
                }
                depth -= 1;
            }
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::Comma => {
                if depth == 0 {
                    return j.saturating_sub(1).max(last);
                }
            }
            _ => {}
        }
        last = j;
        j += 1;
    }
    last
}

fn apply_rewrite(tokens: Vec<Token>, t: Ternary) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len() + 6);

    out.extend(tokens[..t.cond_start].iter().cloned());
    out.push(synth_keyword("if"));
    out.extend(tokens[t.cond_start..t.q_pos].iter().cloned());
    out.push(synth_brace_start());
    out.extend(tokens[t.q_pos + 1..t.colon_pos].iter().cloned());
    out.push(synth_brace_end());
    out.push(synth_keyword("else"));
    out.push(synth_brace_start());
    out.extend(tokens[t.colon_pos + 1..=t.else_end].iter().cloned());
    out.push(synth_brace_end());
    out.extend(tokens[t.else_end + 1..].iter().cloned());

    out
}

fn synth_keyword(value: &str) -> Token {
    Token::new(
        TokenKind::Keyword,
        value.to_string(),
        value.len(),
        Data::None,
        true,
    )
}

fn synth_brace_start() -> Token {
    Token::new(TokenKind::BraceStart, "{".to_string(), 1, Data::None, true)
}

fn synth_brace_end() -> Token {
    Token::new(TokenKind::BraceEnd, "}".to_string(), 1, Data::None, true)
}
