//! High-level auto-borrow, auto-clone, string `+` concatenation, and `for`
//! preservation pass over the Copper token stream.
//!
//! Runs in [`super::Parser::new`] after `sugar::rewrite` and `ternary::rewrite`,
//! before the streaming parser emits Rust source.

use copper_syntax::lexicon::convert_type;
use copper_syntax::tokenizer::{
    kind::TokenKind,
    tokens::{Data, Token},
};
use std::collections::HashMap;

pub(super) fn rewrite(tokens: Vec<Token>) -> Vec<Token> {
    let sigs = collect_signatures(&tokens);
    let t1 = rewrite_scopes(tokens, &sigs);
    rewrite_string_plus(t1)
}

fn synth(kind: TokenKind, value: &str) -> Token {
    Token::new(
        kind,
        value.to_string(),
        0,
        (0, 0),
        Data::new(0, 0),
        true,
    )
}

fn is_trivia(t: &Token) -> bool {
    matches!(
        t.kind,
        TokenKind::Whitespace | TokenKind::Comment | TokenKind::DocComment
    )
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

// ============================================================================
// Signature collection
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParamMode {
    RefStr,
    Ref,
    RefMut,
    OwnedString,
    Copy,
    OwnedOther,
}

#[derive(Debug, Clone)]
struct FuncSig {
    params: Vec<(String, ParamMode)>,
}

fn classify_type_str(raw: &str) -> ParamMode {
    let t = raw.trim();
    if t.starts_with("&mut ") || t.starts_with("& mut ") {
        return ParamMode::RefMut;
    }
    if t.starts_with('&') {
        let inner = t.trim_start_matches('&').trim();
        if inner == "str" || inner.ends_with(" str") {
            return ParamMode::RefStr;
        }
        return ParamMode::Ref;
    }
    let rust = convert_type(t);
    match rust.as_str() {
        "String" => ParamMode::OwnedString,
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64"
        | "u128" | "usize" | "f32" | "f64" | "bool" | "char" | "()" => ParamMode::Copy,
        _ => ParamMode::OwnedOther,
    }
}

fn collect_signatures(tokens: &[Token]) -> HashMap<String, FuncSig> {
    let mut map: HashMap<String, FuncSig> = HashMap::new();

    // Register qualified stdlib signatures (always safe, never collide with user bare names).
    let std_sigs: &[(&str, &[ParamMode])] = &[
        ("cstd::input", &[ParamMode::RefStr]),
        ("cstd::read_int", &[ParamMode::RefStr]),
        ("cstd::read_float", &[ParamMode::RefStr]),
        ("cstd::die", &[ParamMode::RefStr]),
        ("cstd::env", &[ParamMode::RefStr]),
        ("cstd::exists", &[ParamMode::RefStr]),
        ("cstd::is_file", &[ParamMode::RefStr]),
        ("cstd::is_dir", &[ParamMode::RefStr]),
        ("cstd::read_file", &[ParamMode::RefStr]),
        ("cstd::write_file", &[ParamMode::RefStr, ParamMode::RefStr]),
        ("cstd::append_file", &[ParamMode::RefStr, ParamMode::RefStr]),
        ("cstd::trim", &[ParamMode::RefStr]),
        ("cstd::panic_if", &[ParamMode::Copy, ParamMode::RefStr]),
        ("cstd::list_dir", &[ParamMode::RefStr]),
        ("cstd::run", &[ParamMode::RefStr]),
        ("cstd::to_int", &[ParamMode::RefStr]),
        ("cstd::to_float", &[ParamMode::RefStr]),
        ("cstd::split", &[ParamMode::RefStr, ParamMode::RefStr]),
        ("cstd::join", &[ParamMode::OwnedOther, ParamMode::RefStr]),
        ("fs::read", &[ParamMode::RefStr]),
        ("fs::write", &[ParamMode::RefStr, ParamMode::RefStr]),
        ("fs::append", &[ParamMode::RefStr, ParamMode::RefStr]),
        ("fs::exists", &[ParamMode::RefStr]),
        ("fs::is_file", &[ParamMode::RefStr]),
        ("fs::is_dir", &[ParamMode::RefStr]),
        ("fs::mkdir", &[ParamMode::RefStr]),
        ("fs::remove_file", &[ParamMode::RefStr]),
        ("fs::remove_dir", &[ParamMode::RefStr]),
        ("fs::copy", &[ParamMode::RefStr, ParamMode::RefStr]),
        ("fs::rename", &[ParamMode::RefStr, ParamMode::RefStr]),
        ("fs::size", &[ParamMode::RefStr]),
        ("fs::list", &[ParamMode::RefStr]),
        ("fs::read_lines", &[ParamMode::RefStr]),
        ("crypto::base64_encode", &[ParamMode::RefStr]),
        ("crypto::base64_decode", &[ParamMode::RefStr]),
        ("crypto::hex_encode", &[ParamMode::RefStr]),
        ("crypto::hex_decode", &[ParamMode::RefStr]),
        ("crypto::sha256", &[ParamMode::RefStr]),
        ("crypto::sha512", &[ParamMode::RefStr]),
        ("crypto::hmac_sha256", &[ParamMode::RefStr, ParamMode::RefStr]),
        ("crypto::crc32", &[ParamMode::RefStr]),
        ("url::encode", &[ParamMode::RefStr]),
        ("url::encode_component", &[ParamMode::RefStr]),
        ("url::decode", &[ParamMode::RefStr]),
        ("url::join", &[ParamMode::RefStr, ParamMode::RefStr]),
    ];
    for (name, modes) in std_sigs {
        map.insert(
            (*name).to_string(),
            FuncSig {
                params: modes
                    .iter()
                    .enumerate()
                    .map(|(idx, m)| (format!("_{idx}"), *m))
                    .collect(),
            },
        );
    }

    // Also register bare imported std names when `import { ... } from cstd|fs|crypto|url` appears.
    let mut i = 0usize;
    while i < tokens.len() {
        if tokens[i].kind == TokenKind::Import {
            let mut j = i + 1;
            let mut imported = Vec::new();
            let mut from_mod = None;
            while j < tokens.len()
                && tokens[j].kind != TokenKind::Newline
                && tokens[j].kind != TokenKind::Semicolon
            {
                if tokens[j].kind == TokenKind::ModuleVar {
                    imported.push(tokens[j].value.clone());
                } else if tokens[j].kind == TokenKind::ModulePath {
                    from_mod = Some(tokens[j].value.clone());
                }
                j += 1;
            }
            if let Some(m) = from_mod {
                for item in imported {
                    let q = format!("{m}::{item}");
                    if let Some(sig) = map.get(&q).cloned() {
                        map.entry(item).or_insert(sig);
                    }
                }
            }
            i = j;
            continue;
        }

        if tokens[i].kind == TokenKind::Keyword && tokens[i].value == "func" {
            if let Some((name, sig, end_paren)) = parse_func_sig(tokens, i) {
                // Only record free functions under bare name (or `Type::fn` for impl methods
                // if needed later); skip `self`-receiving methods so they don't shadow free fns.
                let has_self = sig.params.first().is_some_and(|(n, _)| n == "self");
                if !has_self {
                    map.insert(name, sig);
                }
                i = end_paren + 1;
                continue;
            }
        }
        i += 1;
    }

    map
}

fn parse_func_sig(tokens: &[Token], func_idx: usize) -> Option<(String, FuncSig, usize)> {
    let mut cursor = func_idx + 1;
    let mut open = None;
    while cursor < tokens.len() {
        let v = tokens[cursor].value.as_str();
        if v == "(" && matches!(tokens[cursor].kind, TokenKind::ParenthesesStart | TokenKind::ParametersStart) {
            open = Some(cursor);
            break;
        }
        if v == "{" || v == ";" || tokens[cursor].kind == TokenKind::Eof {
            return None;
        }
        cursor += 1;
    }
    let open = open?;
    let close = find_matching_paren(tokens, open)?;
    let fn_name = prev_sig(tokens, open)
        .filter(|&p| tokens[p].kind == TokenKind::Identifier)
        .map(|p| tokens[p].value.clone())?;

    let inner = &tokens[open + 1..close];
    let mut params = Vec::new();
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

    for slice in slices {
        let sig_toks: Vec<&Token> = slice
            .iter()
            .filter(|t| !is_trivia(t) && t.kind != TokenKind::Newline)
            .collect();
        if sig_toks.is_empty() {
            continue;
        }
        if sig_toks.iter().any(|t| t.value == "self") {
            params.push(("self".to_string(), ParamMode::Ref));
            continue;
        }
        let colon_pos = sig_toks.iter().position(|t| t.value == ":");
        if let Some(cp) = colon_pos {
            let pname = sig_toks[..cp]
                .last()
                .map(|t| t.value.clone())
                .unwrap_or_default();
            let mut ty_str = String::new();
            for (k, t) in sig_toks[cp + 1..].iter().enumerate() {
                if k > 0 && matches!(sig_toks[cp + 1 + k - 1].value.as_str(), "mut" | "dyn" | "impl")
                {
                    ty_str.push(' ');
                }
                ty_str.push_str(&t.value);
            }
            params.push((pname, classify_type_str(&ty_str)));
        }
    }

    Some((fn_name, FuncSig { params }, close))
}

// ============================================================================
// Scoped rewriting: call-site auto-borrow/clone & `for` preservation
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VarKind {
    Borrowed,
    BorrowedStr,
    MutBorrowed,
    OwnedString,
    OwnedOther,
    Copy,
}

impl VarKind {
    fn from_param_mode(m: ParamMode) -> Self {
        match m {
            ParamMode::RefStr | ParamMode::Ref => VarKind::Borrowed,
            ParamMode::RefMut => VarKind::MutBorrowed,
            ParamMode::OwnedString => VarKind::OwnedString,
            ParamMode::Copy => VarKind::Copy,
            ParamMode::OwnedOther => VarKind::OwnedOther,
        }
    }
}

fn rewrite_scopes(tokens: Vec<Token>, sigs: &HashMap<String, FuncSig>) -> Vec<Token> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0usize;

    // Top-level statements (outside func/struct/class/impl/enum/trait) form an
    // implicit `main` scope. Track top-level locals across those statements too!
    let mut top_vars: HashMap<String, VarKind> = HashMap::new();

    while i < tokens.len() {
        // Function definition: rewrite its body with its own scope!
        if tokens[i].kind == TokenKind::Keyword && tokens[i].value == "func" {
            let sig = parse_func_sig(&tokens, i);
            // Copy up to `{`
            let mut j = i;
            let mut open_brace = None;
            while j < tokens.len() {
                if tokens[j].value == "{" {
                    open_brace = Some(j);
                    break;
                }
                if tokens[j].kind == TokenKind::Semicolon {
                    break;
                }
                j += 1;
            }
            if let Some(ob) = open_brace {
                if let Some(cb) = find_matching_brace(&tokens, ob) {
                    out.extend_from_slice(&tokens[i..=ob]);
                    let mut fn_vars = HashMap::new();
                    if let Some((_, fsig, _)) = sig {
                        for (pname, pmode) in fsig.params {
                            fn_vars.insert(pname, VarKind::from_param_mode(pmode));
                        }
                    }
                    let rewritten_body =
                        rewrite_block_tokens(&tokens[ob + 1..cb], sigs, &mut fn_vars, 0);
                    out.extend(rewritten_body);
                    out.push(tokens[cb].clone());
                    i = cb + 1;
                    continue;
                }
            }
        }

        // Skip struct / class / impl / trait / enum declarations at top-level.
        // (`impl` / `class` bodies contain methods; rewrite inside `impl` `func`s by
        // only skipping `struct`/`enum`/`trait`/`class` headers or leaving `impl` to
        // hit `func` inside it!)
        if matches!(
            tokens[i].kind,
            TokenKind::Struct | TokenKind::Trait
        ) || (tokens[i].kind == TokenKind::Keyword
            && matches!(tokens[i].value.as_str(), "class" | "enum"))
        {
            let mut j = i;
            while j < tokens.len() && tokens[j].value != "{" && tokens[j].kind != TokenKind::Semicolon {
                j += 1;
            }
            if j < tokens.len() && tokens[j].value == "{" {
                if let Some(cb) = find_matching_brace(&tokens, j) {
                    out.extend_from_slice(&tokens[i..=cb]);
                    i = cb + 1;
                    continue;
                }
            }
        }

        // `impl` block: emit header up to `{`, then let the loop enter and handle inner `func`s.
        if tokens[i].kind == TokenKind::Impl {
            let mut j = i;
            while j < tokens.len() && tokens[j].value != "{" {
                j += 1;
            }
            if j < tokens.len() {
                out.extend_from_slice(&tokens[i..=j]);
                i = j + 1;
                continue;
            }
        }

        // Top-level statement chunk: collect up to next top-level item (`func`/`struct`/`class`/`impl`/`trait`/`enum`).
        let start = i;
        while i < tokens.len() {
            let t = &tokens[i];
            if (t.kind == TokenKind::Keyword
                && matches!(t.value.as_str(), "func" | "class" | "enum"))
                || matches!(
                    t.kind,
                    TokenKind::Struct | TokenKind::Impl | TokenKind::Trait
                )
            {
                break;
            }
            if t.value == "{" {
                if let Some(cb) = find_matching_brace(&tokens, i) {
                    i = cb + 1;
                    continue;
                }
            }
            i += 1;
        }
        if i > start {
            let chunk = rewrite_block_tokens(&tokens[start..i], sigs, &mut top_vars, 0);
            out.extend(chunk);
        } else {
            out.push(tokens[i].clone());
            i += 1;
        }
    }

    out
}

fn rewrite_block_tokens(
    tokens: &[Token],
    sigs: &HashMap<String, FuncSig>,
    vars: &mut HashMap<String, VarKind>,
    loop_depth: usize,
) -> Vec<Token> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0usize;
    let outer_vars: HashMap<String, VarKind> = vars.clone();

    while i < tokens.len() {
        // Track local variable bindings: `[mut] ident [: Type] = expr`
        if is_binding_at(tokens, i) {
            record_binding(tokens, i, vars);
        }

        // Handle `for pat in iter_ident { ... }`: preserve `iter_ident` if used later!
        if tokens[i].kind == TokenKind::For {
            if let Some(in_pos) = find_for_in_keyword(tokens, i + 1) {
                if let Some(brace_open) = find_for_brace(tokens, in_pos + 1) {
                    if let Some(brace_close) = find_matching_brace(tokens, brace_open) {
                        let iter_toks: Vec<(usize, &Token)> = tokens[in_pos + 1..brace_open]
                            .iter()
                            .enumerate()
                            .filter(|(_, t)| !is_trivia(t) && t.kind != TokenKind::Newline)
                            .collect();
                        let clone_iter = if iter_toks.len() == 1
                            && iter_toks[0].1.kind == TokenKind::Identifier
                        {
                            let vname = &iter_toks[0].1.value;
                            let vk = vars.get(vname).copied().unwrap_or(VarKind::OwnedOther);
                            vk != VarKind::Copy
                                && (is_ident_used_in(&tokens[brace_close + 1..], vname)
                                    || (loop_depth > 0 && outer_vars.contains_key(vname)))
                        } else {
                            false
                        };

                        out.extend_from_slice(&tokens[i..=in_pos]);
                        if clone_iter {
                            let ident = iter_toks[0].1.clone();
                            out.push(ident);
                            out.push(synth(TokenKind::Dot, "."));
                            out.push(synth(TokenKind::Identifier, "clone"));
                            out.push(synth(TokenKind::ParenthesesStart, "("));
                            out.push(synth(TokenKind::ParenthesesEnd, ")"));
                        } else {
                            out.extend_from_slice(&tokens[in_pos + 1..brace_open]);
                        }
                        out.push(tokens[brace_open].clone());
                        let inner = rewrite_block_tokens(
                            &tokens[brace_open + 1..brace_close],
                            sigs,
                            vars,
                            loop_depth + 1,
                        );
                        out.extend(inner);
                        out.push(tokens[brace_close].clone());
                        i = brace_close + 1;
                        continue;
                    }
                }
            }
        }

        // Track `while` / `loop` bodies so `loop_depth` is accurate for future-use checks.
        if matches!(tokens[i].kind, TokenKind::While | TokenKind::Loop) {
            if let Some(brace_open) = find_for_brace(tokens, i + 1) {
                if let Some(brace_close) = find_matching_brace(tokens, brace_open) {
                    out.extend_from_slice(&tokens[i..=brace_open]);
                    let inner = rewrite_block_tokens(
                        &tokens[brace_open + 1..brace_close],
                        sigs,
                        vars,
                        loop_depth + 1,
                    );
                    out.extend(inner);
                    out.push(tokens[brace_close].clone());
                    i = brace_close + 1;
                    continue;
                }
            }
        }

        // Call site: `[mod::]callee(args)`
        if let Some((callee_name, open_paren, close_paren)) = match_call_site(tokens, i) {
            if let Some(sig) = sigs.get(&callee_name) {
                let is_method = prev_sig(tokens, i)
                    .map(|p| tokens[p].value == ".")
                    .unwrap_or(false);
                if !is_method {
                    // Recursively rewrite nested calls inside arguments first? No, apply on
                    // args directly and recurse inside each arg!
                    let after_call = &tokens[close_paren + 1..];
                    let rewritten_args = rewrite_call_args(
                        &tokens[open_paren + 1..close_paren],
                        sig,
                        sigs,
                        vars,
                        after_call,
                        loop_depth,
                        &outer_vars,
                    );
                    out.extend_from_slice(&tokens[i..=open_paren]);
                    out.extend(rewritten_args);
                    out.push(tokens[close_paren].clone());
                    i = close_paren + 1;
                    continue;
                }
            }
        }

        out.push(tokens[i].clone());
        i += 1;
    }

    out
}

fn match_call_site(tokens: &[Token], idx: usize) -> Option<(String, usize, usize)> {
    if tokens[idx].kind != TokenKind::Identifier {
        return None;
    }
    // Check `ident (` or `mod :: ident (`
    let n1 = next_sig(tokens, idx + 1)?;
    if tokens[n1].value == "::" {
        let n2 = next_sig(tokens, n1 + 1)?;
        if tokens[n2].kind == TokenKind::Identifier {
            let n3 = next_sig(tokens, n2 + 1)?;
            if tokens[n3].value == "(" {
                // Ensure not `mod::ident!(`
                if let Some(close) = find_matching_paren(tokens, n3) {
                    let qname = format!("{}::{}", tokens[idx].value, tokens[n2].value);
                    return Some((qname, n3, close));
                }
            }
        }
        return None;
    }
    if tokens[n1].value == "(" {
        if let Some(close) = find_matching_paren(tokens, n1) {
            return Some((tokens[idx].value.clone(), n1, close));
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn rewrite_call_args(
    inner: &[Token],
    sig: &FuncSig,
    all_sigs: &HashMap<String, FuncSig>,
    vars: &mut HashMap<String, VarKind>,
    after_call: &[Token],
    loop_depth: usize,
    outer_vars: &HashMap<String, VarKind>,
) -> Vec<Token> {
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

    let mut out = Vec::new();
    for (arg_idx, arg) in raw_args.iter().enumerate() {
        if arg_idx > 0 {
            out.push(synth(TokenKind::Comma, ","));
        }

        // First rewrite any nested calls inside this argument.
        let inner_rewritten = rewrite_block_tokens(arg, all_sigs, vars, loop_depth);
        let Some((_, param_mode)) = sig.params.get(arg_idx) else {
            out.extend(inner_rewritten);
            continue;
        };

        let non_ws: Vec<(usize, &Token)> = inner_rewritten
            .iter()
            .enumerate()
            .filter(|(_, t)| !is_trivia(t) && t.kind != TokenKind::Newline)
            .collect();
        if non_ws.is_empty() {
            out.extend(inner_rewritten);
            continue;
        }

        let already_ref = non_ws[0].1.value == "&" || non_ws[0].1.value == "&&";
        let already_mut_ref = already_ref
            && non_ws.get(1).is_some_and(|(_, t)| t.value == "mut");
        let is_plain_str_lit =
            non_ws.len() == 1 && non_ws[0].1.kind == TokenKind::String && !non_ws[0].1.value.starts_with("b\"");
        let is_interp_str =
            non_ws.len() == 1 && non_ws[0].1.kind == TokenKind::InterpolatedString;
        let single_ident = if non_ws.len() == 1 && non_ws[0].1.kind == TokenKind::Identifier {
            Some(non_ws[0].1.value.clone())
        } else {
            None
        };
        let ident_kind = single_ident
            .as_ref()
            .and_then(|name| vars.get(name).copied());
        let used_later = |name: &str| -> bool {
            let later_args = raw_args[arg_idx + 1..]
                .iter()
                .any(|a| is_ident_used_in(a, name));
            later_args
                || is_ident_used_in(after_call, name)
                || (loop_depth > 0 && outer_vars.contains_key(name))
        };

        match param_mode {
            ParamMode::RefStr => {
                if already_ref
                    || is_plain_str_lit
                    || matches!(
                        ident_kind,
                        Some(VarKind::Borrowed | VarKind::BorrowedStr | VarKind::MutBorrowed)
                    )
                {
                    out.extend(inner_rewritten);
                } else if single_ident.is_some()
                    || is_simple_place(&non_ws)
                {
                    out.push(synth(TokenKind::Operator, "&"));
                    out.extend(inner_rewritten);
                } else if is_interp_str {
                    out.push(synth(TokenKind::Operator, "&"));
                    out.push(synth(TokenKind::ParenthesesStart, "("));
                    out.extend(inner_rewritten);
                    out.push(synth(TokenKind::ParenthesesEnd, ")"));
                } else {
                    out.push(synth(TokenKind::Operator, "&"));
                    out.extend(inner_rewritten);
                }
            }
            ParamMode::Ref => {
                if already_ref
                    || matches!(
                        ident_kind,
                        Some(VarKind::Borrowed | VarKind::BorrowedStr | VarKind::MutBorrowed)
                    )
                {
                    out.extend(inner_rewritten);
                } else {
                    out.push(synth(TokenKind::Operator, "&"));
                    out.extend(inner_rewritten);
                }
            }
            ParamMode::RefMut => {
                if already_mut_ref || matches!(ident_kind, Some(VarKind::MutBorrowed)) {
                    out.extend(inner_rewritten);
                } else if already_ref {
                    out.extend(inner_rewritten);
                } else {
                    out.push(synth(TokenKind::Operator, "&"));
                    out.push(synth(TokenKind::Keyword, "mut"));
                    out.extend(inner_rewritten);
                }
            }
            ParamMode::OwnedString => {
                if is_plain_str_lit || matches!(ident_kind, Some(VarKind::BorrowedStr)) {
                    out.extend(inner_rewritten);
                    out.push(synth(TokenKind::Dot, "."));
                    out.push(synth(TokenKind::Identifier, "into"));
                    out.push(synth(TokenKind::ParenthesesStart, "("));
                    out.push(synth(TokenKind::ParenthesesEnd, ")"));
                } else if let Some(ref name) = single_ident {
                    if matches!(ident_kind, Some(VarKind::Borrowed)) {
                        out.extend(inner_rewritten);
                        out.push(synth(TokenKind::Dot, "."));
                        out.push(synth(TokenKind::Identifier, "to_string"));
                        out.push(synth(TokenKind::ParenthesesStart, "("));
                        out.push(synth(TokenKind::ParenthesesEnd, ")"));
                    } else if used_later(name) {
                        out.extend(inner_rewritten);
                        out.push(synth(TokenKind::Dot, "."));
                        out.push(synth(TokenKind::Identifier, "clone"));
                        out.push(synth(TokenKind::ParenthesesStart, "("));
                        out.push(synth(TokenKind::ParenthesesEnd, ")"));
                    } else {
                        out.extend(inner_rewritten);
                    }
                } else {
                    out.extend(inner_rewritten);
                }
            }
            ParamMode::OwnedOther => {
                if let Some(ref name) = single_ident {
                    if matches!(
                        ident_kind,
                        Some(VarKind::OwnedOther | VarKind::OwnedString) | None
                    ) && !matches!(ident_kind, Some(VarKind::Copy))
                        && used_later(name)
                    {
                        out.extend(inner_rewritten);
                        out.push(synth(TokenKind::Dot, "."));
                        out.push(synth(TokenKind::Identifier, "clone"));
                        out.push(synth(TokenKind::ParenthesesStart, "("));
                        out.push(synth(TokenKind::ParenthesesEnd, ")"));
                        continue;
                    }
                }
                out.extend(inner_rewritten);
            }
            ParamMode::Copy => {
                out.extend(inner_rewritten);
            }
        }
    }

    out
}

fn is_simple_place(non_ws: &[(usize, &Token)]) -> bool {
    non_ws
        .iter()
        .all(|(_, t)| matches!(t.kind, TokenKind::Identifier | TokenKind::Dot))
}

fn is_binding_at(tokens: &[Token], idx: usize) -> bool {
    let at_stmt_start = idx == 0
        || matches!(
            prev_sig_or_nl(tokens, idx).map(|p| tokens[p].kind),
            Some(TokenKind::Newline | TokenKind::Semicolon | TokenKind::BraceStart | TokenKind::BraceEnd)
        );
    if !at_stmt_start {
        return false;
    }
    let cur = &tokens[idx];
    if cur.value == "mut" || cur.value == "let" {
        return true;
    }
    if cur.kind == TokenKind::Identifier {
        if let Some(n1) = next_sig(tokens, idx + 1) {
            if tokens[n1].value == "=" && tokens.get(n1 + 1).map(|t| t.value.as_str()) != Some(">") {
                return true;
            }
            if tokens[n1].value == ":" {
                return true;
            }
        }
    }
    false
}

fn prev_sig_or_nl(tokens: &[Token], mut idx: usize) -> Option<usize> {
    while idx > 0 {
        idx -= 1;
        if !is_trivia(&tokens[idx]) {
            return Some(idx);
        }
    }
    None
}

fn record_binding(tokens: &[Token], idx: usize, vars: &mut HashMap<String, VarKind>) {
    let mut p = idx;
    while p < tokens.len() && matches!(tokens[p].value.as_str(), "let" | "mut") {
        p = next_sig(tokens, p + 1).unwrap_or(tokens.len());
    }
    if p >= tokens.len() || tokens[p].kind != TokenKind::Identifier {
        return;
    }
    let name = tokens[p].value.clone();
    let Some(mut cur) = next_sig(tokens, p + 1) else {
        return;
    };

    // Optional `: Type`
    let mut explicit_ty = None;
    if tokens[cur].value == ":" {
        cur += 1;
        let mut ty_buf = String::new();
        while cur < tokens.len() && tokens[cur].value != "=" {
            if tokens[cur].kind == TokenKind::Newline || tokens[cur].kind == TokenKind::Semicolon {
                return;
            }
            if !is_trivia(&tokens[cur]) {
                if !ty_buf.is_empty() && tokens[cur - 1].value == "mut" {
                    ty_buf.push(' ');
                }
                ty_buf.push_str(&tokens[cur].value);
            }
            cur += 1;
        }
        explicit_ty = Some(classify_type_str(&ty_buf));
    }

    if cur >= tokens.len() || tokens[cur].value != "=" {
        return;
    }
    // Collect RHS up to statement end at depth 0
    let rhs_start = cur + 1;
    let mut rhs_end = rhs_start;
    let mut depth = 0i32;
    while rhs_end < tokens.len() {
        let t = &tokens[rhs_end];
        if depth == 0 && (t.kind == TokenKind::Newline || t.kind == TokenKind::Semicolon) {
            break;
        }
        match t.value.as_str() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth = (depth - 1).max(0),
            _ => {}
        }
        rhs_end += 1;
    }

    if let Some(pm) = explicit_ty {
        vars.insert(name, VarKind::from_param_mode(pm));
        return;
    }

    let rhs_sig: Vec<&Token> = tokens[rhs_start..rhs_end]
        .iter()
        .filter(|t| !is_trivia(t) && t.kind != TokenKind::Newline)
        .collect();
    if rhs_sig.is_empty() {
        return;
    }

    let kind = infer_rhs_var_kind(&rhs_sig);
    vars.insert(name, kind);
}

fn infer_rhs_var_kind(rhs: &[&Token]) -> VarKind {
    if rhs[0].value == "&" {
        if rhs.get(1).is_some_and(|t| t.value == "mut") {
            return VarKind::MutBorrowed;
        }
        return VarKind::Borrowed;
    }
    if rhs.len() == 1 && rhs[0].kind == TokenKind::String && !rhs[0].value.starts_with("b\"") {
        return VarKind::BorrowedStr;
    }
    if rhs.iter().any(|t| t.kind == TokenKind::InterpolatedString)
        || rhs.windows(2).any(|w| {
            w[0].value == "." && matches!(w[1].value.as_str(), "to_string" | "to_owned")
        })
    {
        return VarKind::OwnedString;
    }
    if rhs.len() == 1
        && (rhs[0].kind == TokenKind::Number || matches!(rhs[0].value.as_str(), "true" | "false"))
    {
        return VarKind::Copy;
    }
    if rhs.iter().all(|t| {
        matches!(t.kind, TokenKind::Number | TokenKind::Operator | TokenKind::ParenthesesStart | TokenKind::ParenthesesEnd)
            && !matches!(t.value.as_str(), "&" | "?")
    }) {
        return VarKind::Copy;
    }
    VarKind::OwnedOther
}

fn find_for_in_keyword(tokens: &[Token], mut idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    while idx < tokens.len() {
        match tokens[idx].value.as_str() {
            "(" | "[" => depth += 1,
            ")" | "]" => depth = (depth - 1).max(0),
            "{" if depth == 0 => return None,
            "in" if depth == 0 && matches!(tokens[idx].kind, TokenKind::In | TokenKind::Keyword) => {
                return Some(idx)
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn find_for_brace(tokens: &[Token], mut idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    while idx < tokens.len() {
        match tokens[idx].value.as_str() {
            "(" | "[" => depth += 1,
            ")" | "]" => depth = (depth - 1).max(0),
            "{" if depth == 0 => return Some(idx),
            ";" if depth == 0 => return None,
            _ => {}
        }
        idx += 1;
    }
    None
}

/// Returns true if `ident` appears as a value reference in `tokens`
/// (ignoring field names after `.`, path segments after `::`, and struct-literal
/// field labels `ident: val`).
fn is_ident_used_in(tokens: &[Token], ident: &str) -> bool {
    for (i, t) in tokens.iter().enumerate() {
        if t.value != ident || !matches!(t.kind, TokenKind::Identifier | TokenKind::Param) {
            continue;
        }
        if let Some(p) = prev_sig(tokens, i) {
            if matches!(tokens[p].value.as_str(), "." | "::") {
                continue;
            }
        }
        if let Some(n) = next_sig(tokens, i + 1) {
            // Struct field label `ident: value` (not a call)
            if tokens[n].value == ":" && tokens.get(n + 1).map(|x| x.value.as_str()) != Some(":") {
                continue;
            }
        }
        return true;
    }
    false
}

// ============================================================================
// String `+` concatenation (`"a" + b` -> `format!("{}{}", "a", b)`)
// ============================================================================

fn rewrite_string_plus(tokens: Vec<Token>) -> Vec<Token> {
    // Process each function body and top-level chunk independently so string-typed
    // variable names in one function never leak into another.
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0usize;
    let mut top_str_vars: HashMap<String, bool> = HashMap::new();

    while i < tokens.len() {
        if tokens[i].kind == TokenKind::Keyword && tokens[i].value == "func" {
            let mut j = i;
            let mut open_brace = None;
            while j < tokens.len() {
                if tokens[j].value == "{" {
                    open_brace = Some(j);
                    break;
                }
                if tokens[j].kind == TokenKind::Semicolon {
                    break;
                }
                j += 1;
            }
            if let Some(ob) = open_brace {
                if let Some(cb) = find_matching_brace(&tokens, ob) {
                    out.extend_from_slice(&tokens[i..=ob]);
                    let mut fn_str_vars = HashMap::new();
                    let body = rewrite_string_plus_slice(
                        tokens[ob + 1..cb].to_vec(),
                        &mut fn_str_vars,
                    );
                    out.extend(body);
                    out.push(tokens[cb].clone());
                    i = cb + 1;
                    continue;
                }
            }
        }

        if matches!(tokens[i].kind, TokenKind::Struct | TokenKind::Trait)
            || (tokens[i].kind == TokenKind::Keyword
                && matches!(tokens[i].value.as_str(), "class" | "enum"))
        {
            let mut j = i;
            while j < tokens.len() && tokens[j].value != "{" && tokens[j].kind != TokenKind::Semicolon {
                j += 1;
            }
            if j < tokens.len() && tokens[j].value == "{" {
                if let Some(cb) = find_matching_brace(&tokens, j) {
                    out.extend_from_slice(&tokens[i..=cb]);
                    i = cb + 1;
                    continue;
                }
            }
        }

        if tokens[i].kind == TokenKind::Impl {
            let mut j = i;
            while j < tokens.len() && tokens[j].value != "{" {
                j += 1;
            }
            if j < tokens.len() {
                out.extend_from_slice(&tokens[i..=j]);
                i = j + 1;
                continue;
            }
        }

        let start = i;
        while i < tokens.len() {
            let t = &tokens[i];
            if (t.kind == TokenKind::Keyword
                && matches!(t.value.as_str(), "func" | "class" | "enum"))
                || matches!(
                    t.kind,
                    TokenKind::Struct | TokenKind::Impl | TokenKind::Trait
                )
            {
                break;
            }
            if t.value == "{" {
                if let Some(cb) = find_matching_brace(&tokens, i) {
                    i = cb + 1;
                    continue;
                }
            }
            i += 1;
        }
        if i > start {
            let chunk = rewrite_string_plus_slice(tokens[start..i].to_vec(), &mut top_str_vars);
            out.extend(chunk);
        } else {
            out.push(tokens[i].clone());
            i += 1;
        }
    }
    out
}

fn rewrite_string_plus_slice(
    mut tokens: Vec<Token>,
    str_vars: &mut HashMap<String, bool>,
) -> Vec<Token> {
    for i in 0..tokens.len() {
        if is_binding_at(&tokens, i) {
            let mut tmp = HashMap::new();
            record_binding(&tokens, i, &mut tmp);
            for (k, v) in tmp {
                if matches!(v, VarKind::BorrowedStr | VarKind::OwnedString) {
                    str_vars.insert(k, true);
                } else {
                    str_vars.remove(&k);
                }
            }
        }
    }

    loop {
        let mut rewrite_pos = None;
        for i in 0..tokens.len() {
            if tokens[i].kind == TokenKind::Operator && tokens[i].value == "+" {
                // Skip `++`
                if tokens.get(i + 1).is_some_and(|t| t.value == "+")
                    || (i > 0 && tokens[i - 1].value == "+")
                {
                    continue;
                }
                let lhs_start = find_plus_lhs_start(&tokens, i);
                let rhs_end = find_plus_rhs_end(&tokens, i);
                if lhs_start < i && rhs_end > i {
                    let lhs = &tokens[lhs_start..i];
                    let rhs = &tokens[i + 1..=rhs_end];
                    if looks_like_string_expr(lhs, str_vars)
                        || looks_like_string_expr(rhs, str_vars)
                    {
                        rewrite_pos = Some((lhs_start, i, rhs_end));
                        break;
                    }
                }
            }
        }
        let Some((lhs_start, plus_pos, rhs_end)) = rewrite_pos else {
            break;
        };
        let lhs = tokens[lhs_start..plus_pos].to_vec();
        let rhs = tokens[plus_pos + 1..=rhs_end].to_vec();
        let mut next_toks = Vec::with_capacity(tokens.len() + 10);
        next_toks.extend_from_slice(&tokens[..lhs_start]);
        next_toks.push(synth(TokenKind::Identifier, "format"));
        next_toks.push(synth(TokenKind::Operator, "!"));
        next_toks.push(synth(TokenKind::ParenthesesStart, "("));
        next_toks.push(synth(TokenKind::String, "\"{}{}\""));
        next_toks.push(synth(TokenKind::Comma, ","));
        next_toks.extend(lhs);
        next_toks.push(synth(TokenKind::Comma, ","));
        next_toks.extend(rhs);
        next_toks.push(synth(TokenKind::ParenthesesEnd, ")"));
        next_toks.extend_from_slice(&tokens[rhs_end + 1..]);
        tokens = next_toks;
    }

    tokens
}

fn looks_like_string_expr(toks: &[Token], str_vars: &HashMap<String, bool>) -> bool {
    let sig: Vec<&Token> = toks
        .iter()
        .filter(|t| !is_trivia(t) && t.kind != TokenKind::Newline)
        .collect();
    if sig.is_empty() {
        return false;
    }
    if sig.len() == 1 {
        let t = sig[0];
        if t.kind == TokenKind::InterpolatedString {
            return true;
        }
        if t.kind == TokenKind::String && !t.value.starts_with("b\"") {
            return true;
        }
        if t.kind == TokenKind::Identifier && str_vars.contains_key(&t.value) {
            return true;
        }
    }
    // Result of our previous `format!("{}{}", ...)` rewrite
    if sig.len() >= 2 && sig[0].value == "format" && sig[1].value == "!" {
        return true;
    }
    false
}

fn find_plus_lhs_start(tokens: &[Token], plus_pos: usize) -> usize {
    let mut depth = 0i32;
    let mut i = plus_pos;
    while i > 0 {
        i -= 1;
        let t = &tokens[i];
        match t.kind {
            TokenKind::ParenthesesEnd
            | TokenKind::ParametersEnd
            | TokenKind::BracketEnd
            | TokenKind::BraceEnd => depth += 1,
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
                    t.value.as_str(),
                    "return" | "yield" | "in" | "mut" | "let" | "if" | "else" | "match"
                ) {
                    return i + 1;
                }
            }
            TokenKind::Operator if depth == 0 => {
                if matches!(
                    t.value.as_str(),
                    "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | ":" | "?"
                        | "==" | "!=" | "<" | "<=" | ">" | ">=" | "||" | "&&" | "+"
                ) {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    0
}

fn find_plus_rhs_end(tokens: &[Token], plus_pos: usize) -> usize {
    let mut depth = 0i32;
    let mut j = plus_pos + 1;
    let mut last = plus_pos;
    while j < tokens.len() {
        let t = &tokens[j];
        if t.kind == TokenKind::Eof {
            break;
        }
        match t.kind {
            TokenKind::ParenthesesStart
            | TokenKind::ParametersStart
            | TokenKind::BracketStart
            | TokenKind::BraceStart => depth += 1,
            TokenKind::ParenthesesEnd
            | TokenKind::ParametersEnd
            | TokenKind::BracketEnd
            | TokenKind::BraceEnd => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::Comma if depth == 0 => break,
            TokenKind::Operator if depth == 0 => {
                if matches!(
                    t.value.as_str(),
                    "+" | "==" | "!=" | "<" | "<=" | ">" | ">=" | "||" | "&&" | "?" | ":" | "="
                ) {
                    break;
                }
            }
            _ => {}
        }
        if !is_trivia(t) {
            last = j;
        }
        j += 1;
    }
    last
}
