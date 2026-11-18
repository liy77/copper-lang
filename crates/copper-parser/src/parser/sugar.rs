//! High-level syntactic sugar rewrites on the Copper token stream.
//!
//! Runs in [`super::Parser::new`] before `ternary::rewrite` and the main
//! streaming parser, lowering ergonomic Copper forms into standard tokens:
//!
//! 1. Modern `func` syntax:
//!    - `func name(params) -> RetType` → `func RetType name(params)`
//!    - `func ... (...) => expr`        → `func ... (...) { return expr }`
//!    - Default parameter values (`x: int = 10`) & named arguments (`f(x: 2)`)
//! 2. Arrow lambdas (`x => expr`, `() => expr`, `(a, b) => expr`) → `|x| expr`
//!    (ignoring `match` arm `pat => expr` separators).
//! 3. `try { ... } catch err { ... }` blocks → `Result`-returning IIFE + `match`.
//! 4. Null-coalescing (`lhs ?? rhs`) → `(lhs).unwrap_or(rhs)` (with `.cloned()`
//!    when `lhs` is a `?.` field access producing `Option<&T>`).

use copper_syntax::lexicon::convert_type;
use copper_syntax::tokenizer::{
    kind::TokenKind,
    tokens::{Data, Token},
};
use std::collections::HashMap;

pub(super) fn rewrite(tokens: Vec<Token>) -> Vec<Token> {
    let t1 = rewrite_funcs_and_calls(tokens);
    let t2 = rewrite_arrow_lambdas(t1);
    let t3 = rewrite_try_catch(t2);
    rewrite_null_coalesce(t3)
}

fn synth(kind: TokenKind, value: &str) -> Token {
    Token::new(kind, value.to_string(), value.len(), Data::None, true)
}

fn is_trivia(t: &Token) -> bool {
    matches!(
        t.kind,
        TokenKind::Whitespace | TokenKind::Comment | TokenKind::DocComment
    )
}

fn is_sep_or_newline(t: &Token) -> bool {
    matches!(t.kind, TokenKind::Newline | TokenKind::Semicolon)
}

fn prev_sig(tokens: &[Token], mut idx: usize) -> Option<usize> {
    while idx > 0 {
        idx -= 1;
        if !is_trivia(&tokens[idx]) && tokens[idx].kind != TokenKind::Newline {
            return Some(idx);
        }
    }
    None
}

fn next_sig(tokens: &[Token], mut idx: usize) -> Option<usize> {
    while idx < tokens.len() {
        if !is_trivia(&tokens[idx]) && tokens[idx].kind != TokenKind::Newline {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

fn find_matching_paren(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        match t.value.as_str() {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_matching_brace(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        match t.value.as_str() {
            "{" => depth += 1,
            "}" => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_fat_arrow(tokens: &[Token], idx: usize) -> bool {
    tokens.get(idx).map(|t| t.value.as_str()) == Some("=")
        && tokens.get(idx + 1).map(|t| t.value.as_str()) == Some(">")
}

// ============================================================================
// 1. Modern `func`: `-> RetType`, `=> expr`, default params & named args
// ============================================================================

#[derive(Clone)]
struct ParamSpec {
    name: String,
    default: Option<Vec<Token>>,
}

#[derive(Clone)]
struct FuncSpec {
    params: Vec<ParamSpec>,
}

fn rewrite_funcs_and_calls(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut func_specs: HashMap<String, FuncSpec> = HashMap::new();
    let mut i = 0usize;

    while i < tokens.len() {
        if tokens[i].kind == TokenKind::Keyword && tokens[i].value == "func" {
            if let Some((rewritten, next_i, maybe_spec)) = parse_and_rewrite_func(&tokens, i) {
                if let Some((fname, spec)) = maybe_spec {
                    func_specs.insert(fname, spec);
                }
                out.extend(rewritten);
                i = next_i;
                continue;
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }

    if func_specs.is_empty() {
        return out;
    }

    rewrite_call_sites(out, &func_specs)
}

/// Rewrite one `func ...` declaration starting at `func_idx`.
/// Returns `(emitted_tokens, next_index, optional_(fn_name, FuncSpec))`.
fn parse_and_rewrite_func(
    tokens: &[Token],
    func_idx: usize,
) -> Option<(Vec<Token>, usize, Option<(String, FuncSpec)>)> {
    // Skip ahead from `func` to find the parameter list `(` (before `{` or `=>`).
    let mut cursor = func_idx + 1;
    let mut paren_open = None;
    while cursor < tokens.len() {
        let v = tokens[cursor].value.as_str();
        if v == "(" && matches!(tokens[cursor].kind, TokenKind::ParenthesesStart | TokenKind::ParametersStart) {
            paren_open = Some(cursor);
            break;
        }
        if v == "{" || v == ";" || tokens[cursor].kind == TokenKind::Eof {
            return None;
        }
        cursor += 1;
    }
    let paren_open = paren_open?;
    let paren_close = find_matching_paren(tokens, paren_open)?;

    // Function name is the last identifier right before `(` (or before `<generics>(`).
    let fn_name = prev_sig(tokens, paren_open)
        .filter(|&p| tokens[p].kind == TokenKind::Identifier)
        .map(|p| tokens[p].value.clone());

    // Header tokens between `func` and `paren_open`:
    // If `func` had NO return type before the name, `tokens[func_idx + 1 .. paren_open]`
    // starts directly with the function name (not `ReturnType`).
    let header_slice = &tokens[func_idx + 1..paren_open];
    let has_leading_ret = header_slice
        .iter()
        .any(|t| t.kind == TokenKind::ReturnType);

    // Parse parameter list (strip `= default` and record ParamSpec).
    let (clean_param_tokens, param_specs) = strip_default_params(&tokens[paren_open + 1..paren_close]);

    // Check for trailing `-> RetType` after `)` and before `{` or `=>`.
    let mut after = paren_close + 1;
    while after < tokens.len() && is_trivia(&tokens[after]) {
        after += 1;
    }
    let mut ret_tokens: Vec<Token> = Vec::new();
    if !has_leading_ret
        && tokens.get(after).map(|t| t.value.as_str()) == Some("-")
        && tokens.get(after + 1).map(|t| t.value.as_str()) == Some(">")
    {
        after += 2;
        while after < tokens.len() && is_trivia(&tokens[after]) {
            after += 1;
        }
        let mut angle_depth = 0i32;
        let mut paren_depth = 0i32;
        while after < tokens.len() {
            let t = &tokens[after];
            if angle_depth == 0
                && paren_depth == 0
                && (t.value == "{" || is_fat_arrow(tokens, after) || t.kind == TokenKind::Newline)
            {
                break;
            }
            match t.value.as_str() {
                "<" => angle_depth += 1,
                ">" => angle_depth = (angle_depth - 1).max(0),
                "(" => paren_depth += 1,
                ")" => paren_depth = (paren_depth - 1).max(0),
                _ => {}
            }
            ret_tokens.push(t.clone());
            after += 1;
        }
        // Trim trailing whitespace from ret_tokens
        while ret_tokens.last().is_some_and(is_trivia) {
            ret_tokens.pop();
        }
    }

    // Check for `=> expr` single-expression body!
    while after < tokens.len() && (is_trivia(&tokens[after]) || tokens[after].kind == TokenKind::Newline) {
        if tokens[after].kind == TokenKind::Newline && tokens[after].value.contains(';') {
            break;
        }
        after += 1;
    }
    let mut arrow_body: Option<(Vec<Token>, usize)> = None;
    if is_fat_arrow(tokens, after) {
        let expr_start = after + 2;
        let mut p = expr_start;
        let mut depth = 0i32;
        while p < tokens.len() {
            let t = &tokens[p];
            if depth == 0 && (t.kind == TokenKind::Newline || t.kind == TokenKind::Semicolon || t.kind == TokenKind::Eof) {
                break;
            }
            match t.value.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            p += 1;
        }
        let mut expr_toks = tokens[expr_start..p].to_vec();
        while expr_toks.first().is_some_and(is_trivia) {
            expr_toks.remove(0);
        }
        // If followed by a Newline token, consume it so we emit clean `}\n`
        let next_pos = if p < tokens.len() && tokens[p].kind == TokenKind::Newline {
            p + 1
        } else {
            p
        };
        arrow_body = Some((expr_toks, next_pos));
    }

    // If nothing changed (standard `func`), fast-path return None so existing tokens stay verbatim.
    let has_defaults = param_specs.iter().any(|p| p.default.is_some());
    if ret_tokens.is_empty() && arrow_body.is_none() && !has_defaults {
        let spec = fn_name.map(|n| (n, FuncSpec { params: param_specs }));
        return Some((tokens[func_idx..=paren_close].to_vec(), paren_close + 1, spec));
    }

    let mut emitted = Vec::new();
    emitted.push(tokens[func_idx].clone()); // `func`

    if !ret_tokens.is_empty() {
        // Fold tuple return `(A, B)` into one ReturnType token, or tag head token ReturnType
        // so `mod.rs:4388` handles it identically to `func RetType name(...)`.
        if ret_tokens.first().map(|t| t.value.as_str()) == Some("(") {
            let joined: String = ret_tokens.iter().map(|t| t.value.as_str()).collect();
            emitted.push(synth(TokenKind::ReturnType, &joined));
        } else {
            // If trailing `?` or `!` is a separate token (e.g. `Vec<int>!`), leave as tokens
            // after marking the first one ReturnType.
            let mut first = ret_tokens[0].clone();
            first.kind = TokenKind::ReturnType;
            emitted.push(first);
            emitted.extend(ret_tokens.into_iter().skip(1));
        }
    }

    emitted.extend_from_slice(header_slice);
    emitted.push(tokens[paren_open].clone());
    emitted.extend(clean_param_tokens);
    emitted.push(tokens[paren_close].clone());

    let next_i = if let Some((expr_toks, next_pos)) = arrow_body {
        emitted.push(synth(TokenKind::BraceStart, "{"));
        emitted.push(synth(TokenKind::Keyword, "return"));
        emitted.extend(expr_toks);
        emitted.push(synth(TokenKind::Newline, ";\n"));
        emitted.push(synth(TokenKind::BraceEnd, "}"));
        emitted.push(synth(TokenKind::Newline, "\n"));
        next_pos
    } else {
        after
    };

    let spec = fn_name.map(|n| (n, FuncSpec { params: param_specs }));
    Some((emitted, next_i, spec))
}

fn strip_default_params(inner: &[Token]) -> (Vec<Token>, Vec<ParamSpec>) {
    let mut clean = Vec::new();
    let mut specs = Vec::new();

    // Split top-level comma-separated parameters.
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut slices: Vec<&[Token]> = Vec::new();
    for (idx, t) in inner.iter().enumerate() {
        match t.value.as_str() {
            "<" | "(" | "[" | "{" => depth += 1,
            ">" | ")" | "]" | "}" => depth = (depth - 1).max(0),
            "," if depth == 0 => {
                slices.push(&inner[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    if start < inner.len() {
        slices.push(&inner[start..]);
    }

    for (s_idx, slice) in slices.iter().enumerate() {
        let non_empty: Vec<&Token> = slice
            .iter()
            .filter(|t| !is_trivia(t) && t.kind != TokenKind::Newline)
            .collect();
        if non_empty.is_empty() {
            continue;
        }
        // Check for top-level `=` in this parameter (default value).
        let mut eq_pos = None;
        let mut d = 0i32;
        for (idx, t) in slice.iter().enumerate() {
            match t.value.as_str() {
                "<" | "(" | "[" | "{" => d += 1,
                ">" | ")" | "]" | "}" => d = (d - 1).max(0),
                "=" if d == 0 => {
                    eq_pos = Some(idx);
                    break;
                }
                _ => {}
            }
        }

        // Extract param name: identifier before `:` (or `self`).
        let colon_pos = slice.iter().position(|t| t.value == ":");
        let pname = if let Some(cp) = colon_pos {
            slice[..cp]
                .iter()
                .rev()
                .find(|t| {
                    matches!(
                        t.kind,
                        TokenKind::Param | TokenKind::Identifier | TokenKind::Keyword
                    )
                })
                .map(|t| t.value.clone())
                .unwrap_or_default()
        } else {
            non_empty
                .last()
                .map(|t| t.value.clone())
                .unwrap_or_default()
        };

        if let Some(ep) = eq_pos {
            let mut def_toks: Vec<Token> = slice[ep + 1..].to_vec();
            while def_toks.first().is_some_and(is_trivia) {
                def_toks.remove(0);
            }
            while def_toks.last().is_some_and(is_trivia) {
                def_toks.pop();
            }
            if !pname.is_empty() && pname != "self" {
                specs.push(ParamSpec {
                    name: pname,
                    default: Some(def_toks),
                });
            }
            let head = &slice[..ep];
            clean.extend(head.iter().cloned());
        } else {
            if !pname.is_empty() && pname != "self" {
                specs.push(ParamSpec {
                    name: pname,
                    default: None,
                });
            }
            clean.extend(slice.iter().cloned());
        }
        if s_idx + 1 < slices.len() {
            clean.push(synth(TokenKind::Comma, ","));
        }
    }

    (clean, specs)
}

fn rewrite_call_sites(tokens: Vec<Token>, specs: &HashMap<String, FuncSpec>) -> Vec<Token> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0usize;
    while i < tokens.len() {
        // Skip function definitions (`func ... (...)`).
        if tokens[i].kind == TokenKind::Keyword && tokens[i].value == "func" {
            out.push(tokens[i].clone());
            i += 1;
            while i < tokens.len() {
                out.push(tokens[i].clone());
                if tokens[i].value == ")" {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        if tokens[i].kind == TokenKind::Identifier {
            if let Some(spec) = specs.get(&tokens[i].value) {
                // Ensure not a method call `.foo(` and not a macro `foo!(`.
                let is_method = prev_sig(&tokens, i)
                    .map(|p| tokens[p].value == ".")
                    .unwrap_or(false);
                if !is_method {
                    if let Some(open) = next_sig(&tokens, i + 1) {
                        if matches!(
                            tokens[open].kind,
                            TokenKind::ParenthesesStart | TokenKind::ParametersStart
                        ) && tokens[open].value == "("
                        {
                            if let Some(close) = find_matching_paren(&tokens, open) {
                                if let Some(new_args) =
                                    resolve_call_args(&tokens[open + 1..close], spec)
                                {
                                    out.push(tokens[i].clone());
                                    out.push(tokens[open].clone());
                                    out.extend(new_args);
                                    out.push(tokens[close].clone());
                                    i = close + 1;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    out
}

/// Resolve positional + named arguments and fill omitted default params.
/// Returns `None` if nothing needs rewriting (all args positional and count matches).
fn resolve_call_args(inner: &[Token], spec: &FuncSpec) -> Option<Vec<Token>> {
    // Split call args at top-level commas.
    let mut raw_args: Vec<&[Token]> = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, t) in inner.iter().enumerate() {
        match t.value.as_str() {
            "(" | "[" | "{" | "<" => depth += 1,
            ")" | "]" | "}" | ">" => depth = (depth - 1).max(0),
            "," if depth == 0 => {
                raw_args.push(&inner[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    if start < inner.len() {
        let tail = &inner[start..];
        if tail.iter().any(|t| !is_trivia(t) && t.kind != TokenKind::Newline) {
            raw_args.push(tail);
        }
    }

    let mut has_named = false;
    let mut slots: Vec<Option<Vec<Token>>> = vec![None; spec.params.len()];
    let mut next_pos = 0usize;

    for arg in &raw_args {
        // Trim trivia/newlines to inspect leading `name : value`.
        let trimmed: Vec<Token> = arg
            .iter()
            .filter(|t| !is_trivia(t) && t.kind != TokenKind::Newline)
            .cloned()
            .collect();
        if trimmed.is_empty() {
            continue;
        }
        let named = trimmed.len() >= 3
            && matches!(trimmed[0].kind, TokenKind::Identifier | TokenKind::Param)
            && trimmed[1].value == ":";
        if named {
            has_named = true;
            let pname = &trimmed[0].value;
            let idx = spec.params.iter().position(|p| &p.name == pname)?;
            slots[idx] = Some(trimmed[2..].to_vec());
        } else {
            while next_pos < slots.len() && slots[next_pos].is_some() {
                next_pos += 1;
            }
            if next_pos >= slots.len() {
                return None;
            }
            slots[next_pos] = Some((*arg).to_vec());
            next_pos += 1;
        }
    }

    let missing_any = slots.iter().any(|s| s.is_none());
    if !has_named && !missing_any {
        return None;
    }

    // Fill missing slots from defaults. If any slot is missing without a default, abort rewrite.
    for (idx, slot) in slots.iter_mut().enumerate() {
        if slot.is_none() {
            if let Some(def) = &spec.params[idx].default {
                *slot = Some(def.clone());
            } else {
                return None;
            }
        }
    }

    let mut out = Vec::new();
    for (idx, slot) in slots.into_iter().enumerate() {
        if idx > 0 {
            out.push(synth(TokenKind::Comma, ","));
        }
        out.extend(slot.unwrap());
    }
    Some(out)
}

// ============================================================================
// 2. Arrow lambdas (`x => expr`, `() => expr`, `(a, b) => expr`)
// ============================================================================

fn rewrite_arrow_lambdas(tokens: Vec<Token>) -> Vec<Token> {
    // Precompute for every token index whether it sits at the top level of a
    // `match` body (where `=>` is a match-arm separator, NOT a lambda!).
    let is_match_top = compute_match_arm_positions(&tokens);

    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut i = 0usize;

    while i < tokens.len() {
        if is_fat_arrow(&tokens, i) && !is_match_top[i] {
            // Case A: single identifier `x => ...`
            if let Some(prev) = prev_sig(&out, out.len()) {
                let is_ident = matches!(
                    out[prev].kind,
                    TokenKind::Identifier
                        | TokenKind::Param
                        | TokenKind::ParamType
                        | TokenKind::Type
                );
                let after_dot_or_path = prev_sig(&out, prev)
                    .map(|pp| matches!(out[pp].value.as_str(), "." | "::"))
                    .unwrap_or(false);
                if is_ident && !after_dot_or_path {
                    let ident_tok = out[prev].clone();
                    out.truncate(prev);
                    out.push(synth(TokenKind::Operator, "|"));
                    out.push(ident_tok);
                    out.push(synth(TokenKind::Operator, "|"));
                    i += 2; // skip `=`, `>`
                    continue;
                }

                // Case B: `( ... ) => ...`
                if out[prev].value == ")" {
                    if let Some(open) = find_matching_paren_backwards(&out, prev) {
                        let before_open_is_ident = prev_sig(&out, open)
                            .map(|bp| out[bp].kind == TokenKind::Identifier)
                            .unwrap_or(false);
                        if !before_open_is_ident {
                            if let Some(closure_params) =
                                extract_arrow_param_list(&out[open + 1..prev])
                            {
                                out.truncate(open);
                                out.push(synth(TokenKind::Operator, "|"));
                                out.extend(closure_params);
                                out.push(synth(TokenKind::Operator, "|"));
                                i += 2; // skip `=`, `>`
                                continue;
                            }
                        }
                    }
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }

    out
}

fn compute_match_arm_positions(tokens: &[Token]) -> Vec<bool> {
    let mut res = vec![false; tokens.len()];
    // Stack of `(is_match_body, paren_bracket_depth_inside_brace)`
    let mut brace_stack: Vec<(bool, usize)> = Vec::new();
    let mut expect_match_brace = false;
    let mut match_paren_depth = 0usize;

    for (i, t) in tokens.iter().enumerate() {
        let is_top = brace_stack
            .last()
            .map(|(m, d)| *m && *d == 0)
            .unwrap_or(false);
        res[i] = is_top;

        if t.kind == TokenKind::Keyword && t.value == "match" {
            expect_match_brace = true;
            match_paren_depth = 0;
            continue;
        }
        match t.value.as_str() {
            "(" | "[" => {
                if expect_match_brace {
                    match_paren_depth += 1;
                }
                if let Some((_, d)) = brace_stack.last_mut() {
                    *d += 1;
                }
            }
            ")" | "]" => {
                if expect_match_brace && match_paren_depth > 0 {
                    match_paren_depth -= 1;
                }
                if let Some((_, d)) = brace_stack.last_mut() {
                    *d = d.saturating_sub(1);
                }
            }
            "{" => {
                let opens_match = expect_match_brace && match_paren_depth == 0;
                if opens_match {
                    expect_match_brace = false;
                }
                brace_stack.push((opens_match, 0));
            }
            "}" => {
                brace_stack.pop();
            }
            _ => {}
        }
    }
    res
}

fn find_matching_paren_backwards(tokens: &[Token], close: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = close as isize;
    while i >= 0 {
        let idx = i as usize;
        match tokens[idx].value.as_str() {
            ")" => depth += 1,
            "(" => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
        i -= 1;
    }
    None
}

fn extract_arrow_param_list(inner: &[Token]) -> Option<Vec<Token>> {
    let sig: Vec<&Token> = inner
        .iter()
        .filter(|t| !is_trivia(t) && t.kind != TokenKind::Newline)
        .collect();
    if sig.is_empty() {
        return Some(Vec::new());
    }
    // Validate comma-separated `ident (: Type)?`
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < sig.len() {
        let p = sig[idx];
        if !matches!(
            p.kind,
            TokenKind::Identifier | TokenKind::Param | TokenKind::ParamType | TokenKind::Type
        ) {
            return None;
        }
        out.push((*p).clone());
        idx += 1;
        if idx < sig.len() && sig[idx].value == ":" {
            out.push((*sig[idx]).clone());
            idx += 1;
            let mut ty_buf = String::new();
            let mut depth = 0i32;
            while idx < sig.len() {
                let v = sig[idx].value.as_str();
                if depth == 0 && v == "," {
                    break;
                }
                match v {
                    "<" | "(" | "[" => depth += 1,
                    ">" | ")" | "]" => depth = (depth - 1).max(0),
                    _ => {}
                }
                ty_buf.push_str(v);
                idx += 1;
            }
            if ty_buf.is_empty() {
                return None;
            }
            out.push(synth(TokenKind::ParamType, &convert_type(&ty_buf)));
        }
        if idx < sig.len() {
            if sig[idx].value == "," {
                out.push((*sig[idx]).clone());
                idx += 1;
            } else {
                return None;
            }
        }
    }
    Some(out)
}

// ============================================================================
// 3. `try { ... } catch err { ... }`
// ============================================================================

fn rewrite_try_catch(tokens: Vec<Token>) -> Vec<Token> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0usize;

    while i < tokens.len() {
        if tokens[i].value == "try" {
            if let Some(try_open) = next_sig(&tokens, i + 1) {
                if tokens[try_open].value == "{" {
                    if let Some(try_close) = find_matching_brace(&tokens, try_open) {
                        if let Some(catch_kw) = next_sig(&tokens, try_close + 1) {
                            if tokens[catch_kw].value == "catch" {
                                if let Some(after_catch) = next_sig(&tokens, catch_kw + 1) {
                                    let (err_name, catch_open) = if tokens[after_catch].value == "{"
                                    {
                                        ("_".to_string(), Some(after_catch))
                                    } else if matches!(
                                        tokens[after_catch].kind,
                                        TokenKind::Identifier | TokenKind::Param
                                    ) {
                                        let open = next_sig(&tokens, after_catch + 1)
                                            .filter(|&o| tokens[o].value == "{");
                                        (tokens[after_catch].value.clone(), open)
                                    } else {
                                        ("_".to_string(), None)
                                    };

                                    if let Some(catch_open) = catch_open {
                                        if let Some(catch_close) =
                                            find_matching_brace(&tokens, catch_open)
                                        {
                                            let try_body = strip_tail_expr_semi(
                                                &tokens[try_open + 1..try_close],
                                            );
                                            let catch_body = strip_tail_expr_semi(
                                                &tokens[catch_open + 1..catch_close],
                                            );

                                            // Emit:
                                            // match (|| -> std::result::Result<_, Box<dyn std::error::Error>> { Ok({ <try_body> }) })() {
                                            //     Ok(__ctry_ok) => __ctry_ok,
                                            //     Err(<err_name>) => { <catch_body> }
                                            // }
                                            out.push(synth(TokenKind::Keyword, "match"));
                                            out.push(synth(TokenKind::ParenthesesStart, "("));
                                            out.push(synth(TokenKind::Operator, "|"));
                                            out.push(synth(TokenKind::Operator, "|"));
                                            out.push(synth(TokenKind::Operator, "-"));
                                            out.push(synth(TokenKind::Operator, ">"));
                                            out.push(synth(TokenKind::Identifier, "std"));
                                            out.push(synth(TokenKind::Operator, "::"));
                                            out.push(synth(TokenKind::Identifier, "result"));
                                            out.push(synth(TokenKind::Operator, "::"));
                                            out.push(synth(TokenKind::Identifier, "Result"));
                                            out.push(synth(TokenKind::Operator, "<"));
                                            out.push(synth(TokenKind::Identifier, "_"));
                                            out.push(synth(TokenKind::Comma, ","));
                                            out.push(synth(TokenKind::Identifier, "Box"));
                                            out.push(synth(TokenKind::Operator, "<"));
                                            out.push(synth(TokenKind::Keyword, "dyn"));
                                            out.push(synth(TokenKind::Identifier, "std"));
                                            out.push(synth(TokenKind::Operator, "::"));
                                            out.push(synth(TokenKind::Identifier, "error"));
                                            out.push(synth(TokenKind::Operator, "::"));
                                            out.push(synth(TokenKind::Identifier, "Error"));
                                            out.push(synth(TokenKind::Operator, ">"));
                                            out.push(synth(TokenKind::Operator, ">"));
                                            out.push(synth(TokenKind::BraceStart, "{"));
                                            out.push(synth(TokenKind::Identifier, "Ok"));
                                            out.push(synth(TokenKind::ParenthesesStart, "("));
                                            out.push(synth(TokenKind::BraceStart, "{"));
                                            out.extend(try_body);
                                            out.push(synth(TokenKind::BraceEnd, "}"));
                                            out.push(synth(TokenKind::ParenthesesEnd, ")"));
                                            out.push(synth(TokenKind::BraceEnd, "}"));
                                            out.push(synth(TokenKind::ParenthesesEnd, ")"));
                                            out.push(synth(TokenKind::ParenthesesStart, "("));
                                            out.push(synth(TokenKind::ParenthesesEnd, ")"));
                                            out.push(synth(TokenKind::BraceStart, "{"));
                                            out.push(synth(TokenKind::Identifier, "Ok"));
                                            out.push(synth(TokenKind::ParenthesesStart, "("));
                                            out.push(synth(TokenKind::Identifier, "__ctry_ok"));
                                            out.push(synth(TokenKind::ParenthesesEnd, ")"));
                                            out.push(synth(TokenKind::Operator, "="));
                                            out.push(synth(TokenKind::Operator, ">"));
                                            out.push(synth(TokenKind::Identifier, "__ctry_ok"));
                                            out.push(synth(TokenKind::Comma, ","));
                                            out.push(synth(TokenKind::Identifier, "Err"));
                                            out.push(synth(TokenKind::ParenthesesStart, "("));
                                            out.push(synth(TokenKind::Identifier, &err_name));
                                            out.push(synth(TokenKind::ParenthesesEnd, ")"));
                                            out.push(synth(TokenKind::Operator, "="));
                                            out.push(synth(TokenKind::Operator, ">"));
                                            out.push(synth(TokenKind::BraceStart, "{"));
                                            out.extend(catch_body);
                                            out.push(synth(TokenKind::BraceEnd, "}"));
                                            out.push(synth(TokenKind::BraceEnd, "}"));

                                            i = catch_close + 1;
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }

    out
}

/// If the final statement inside a `try`/`catch` block looks like a tail expression
/// (not `let`/`mut`/assignment/control-flow), strip the tokenizer's `;` so the
/// block evaluates to that expression's value.
fn strip_tail_expr_semi(body: &[Token]) -> Vec<Token> {
    let mut out = body.to_vec();
    // Find last token index that is non-trivia and not a pure `\n` Newline.
    let mut last_idx = None;
    for k in (0..out.len()).rev() {
        if is_trivia(&out[k]) {
            continue;
        }
        if out[k].kind == TokenKind::Newline && !out[k].value.contains(';') {
            continue;
        }
        last_idx = Some(k);
        break;
    }
    let Some(li) = last_idx else {
        return out;
    };
    if !(out[li].kind == TokenKind::Newline && out[li].value.contains(';'))
        && out[li].kind != TokenKind::Semicolon
    {
        return out;
    }

    // Walk back to start of that last statement (matching depth 0).
    let mut depth = 0i32;
    let mut stmt_start = 0usize;
    for k in (0..li).rev() {
        match out[k].value.as_str() {
            ")" | "]" | "}" => depth += 1,
            "(" | "[" | "{" => {
                if depth == 0 {
                    stmt_start = k + 1;
                    break;
                }
                depth -= 1;
            }
            _ if depth == 0 && (is_sep_or_newline(&out[k]) || out[k].kind == TokenKind::Comma) => {
                stmt_start = k + 1;
                break;
            }
            _ => {}
        }
    }

    let first_tok = (stmt_start..li)
        .find(|&k| !is_trivia(&out[k]) && out[k].kind != TokenKind::Newline);
    if let Some(ft) = first_tok {
        let kw = out[ft].value.as_str();
        let is_stmt = matches!(
            kw,
            "let" | "mut" | "return" | "break" | "continue" | "for" | "while" | "loop"
        );
        let is_bare_assign = out[ft].kind == TokenKind::Identifier
            && out.get(ft + 1).map(|t| t.value.as_str()) == Some("=")
            && out.get(ft + 2).map(|t| t.value.as_str()) != Some("=");
        if !is_stmt && !is_bare_assign {
            if out[li].kind == TokenKind::Semicolon {
                out.remove(li);
            } else {
                out[li].value = out[li].value.replace(';', "");
            }
        }
    }
    out
}

// ============================================================================
// 4. Null-coalescing (`lhs ?? rhs`)
// ============================================================================

fn rewrite_null_coalesce(mut tokens: Vec<Token>) -> Vec<Token> {
    loop {
        let Some(pos) = tokens
            .iter()
            .position(|t| t.kind == TokenKind::Operator && t.value == "??")
        else {
            break;
        };
        let lhs_start = find_coalesce_lhs_start(&tokens, pos);
        let rhs_end = find_coalesce_rhs_end(&tokens, pos);
        if lhs_start >= pos || rhs_end <= pos {
            break;
        }
        tokens = apply_coalesce_rewrite(tokens, lhs_start, pos, rhs_end);
    }
    tokens
}

fn find_coalesce_lhs_start(tokens: &[Token], q_pos: usize) -> usize {
    let mut depth = 0i32;
    let mut i = q_pos;
    while i > 0 {
        i -= 1;
        let tok = &tokens[i];
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
                    return i + 1;
                }
                depth -= 1;
            }
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::Comma if depth == 0 => {
                return i + 1;
            }
            TokenKind::Keyword | TokenKind::In | TokenKind::Yield if depth == 0 => {
                if matches!(
                    tok.value.as_str(),
                    "return" | "yield" | "in" | "mut" | "let" | "if" | "else" | "match"
                ) {
                    return i + 1;
                }
            }
            TokenKind::Operator if depth == 0 => {
                if matches!(
                    tok.value.as_str(),
                    "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | ":" | "?"
                        | "||" | "&&"
                ) || (tok.value == ">"
                    && i > 0
                    && tokens[i - 1].value == "=")
                {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    0
}

fn find_coalesce_rhs_end(tokens: &[Token], q_pos: usize) -> usize {
    let mut depth = 0i32;
    let mut j = q_pos + 1;
    let mut last_non_trivia = q_pos;
    while j < tokens.len() {
        let tok = &tokens[j];
        if tok.kind == TokenKind::Eof {
            break;
        }
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
                    break;
                }
                depth -= 1;
            }
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::Comma if depth == 0 => {
                break;
            }
            TokenKind::Operator if depth == 0 => {
                if matches!(tok.value.as_str(), "??" | "?" | ":" | "=") {
                    break;
                }
            }
            _ => {}
        }
        if !is_trivia(tok) {
            last_non_trivia = j;
        }
        j += 1;
    }
    last_non_trivia
}

fn apply_coalesce_rewrite(
    tokens: Vec<Token>,
    lhs_start: usize,
    q_pos: usize,
    rhs_end: usize,
) -> Vec<Token> {
    let lhs = &tokens[lhs_start..q_pos];
    let rhs = &tokens[q_pos + 1..=rhs_end];

    // If `lhs` uses `?.` (OptionalChain) and ends in a field access (an identifier,
    // not a method call `)`), its type is `Option<&Field>`, so emit `.cloned()`
    // before `.unwrap_or(...)`.
    let has_opt_chain = lhs.iter().any(|t| t.kind == TokenKind::OptionalChain);
    let last_lhs_sig = lhs
        .iter()
        .rev()
        .find(|t| !is_trivia(t) && t.kind != TokenKind::Newline);
    let is_opt_field_ref = has_opt_chain
        && last_lhs_sig
            .map(|t| t.kind == TokenKind::Identifier)
            .unwrap_or(false);

    // If `rhs` is a single string literal `"..."`, append `.into()` so it coerces
    // seamlessly to both `String` (`Option<String>` / `Option<&String>`) and `&str`.
    let rhs_sig: Vec<&Token> = rhs
        .iter()
        .filter(|t| !is_trivia(t) && t.kind != TokenKind::Newline)
        .collect();
    let rhs_is_str_lit = rhs_sig.len() == 1 && rhs_sig[0].kind == TokenKind::String;

    let mut out = Vec::with_capacity(tokens.len() + 12);
    out.extend_from_slice(&tokens[..lhs_start]);
    out.push(synth(TokenKind::ParenthesesStart, "("));
    out.extend_from_slice(lhs);
    out.push(synth(TokenKind::ParenthesesEnd, ")"));
    if is_opt_field_ref {
        out.push(synth(TokenKind::Dot, "."));
        out.push(synth(TokenKind::Identifier, "cloned"));
        out.push(synth(TokenKind::ParenthesesStart, "("));
        out.push(synth(TokenKind::ParenthesesEnd, ")"));
    }
    out.push(synth(TokenKind::Dot, "."));
    out.push(synth(TokenKind::Identifier, "unwrap_or"));
    out.push(synth(TokenKind::ParenthesesStart, "("));
    out.extend_from_slice(rhs);
    if rhs_is_str_lit {
        out.push(synth(TokenKind::Dot, "."));
        out.push(synth(TokenKind::Identifier, "into"));
        out.push(synth(TokenKind::ParenthesesStart, "("));
        out.push(synth(TokenKind::ParenthesesEnd, ")"));
    }
    out.push(synth(TokenKind::ParenthesesEnd, ")"));
    out.extend_from_slice(&tokens[rhs_end + 1..]);
    out
}
