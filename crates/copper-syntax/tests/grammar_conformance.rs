//! Drift guard: the typed expression parser ([`copper_syntax::expr`]) must keep
//! up with the real Copper language. Rather than re-listing the grammar (which
//! would itself drift), these tests parse expression/statement snippets taken
//! verbatim from the language's own `examples/*.crs` and the patterns the
//! transpiler supports, and assert the parser handles them **without** falling
//! back to [`ExprKind::Raw`] and **without** errors.
//!
//! When someone adds a new expression form to Copper and writes an example for
//! it (the project convention — see CLAUDE.md), extend `SNIPPETS` with that
//! form. A regression — a form the parser silently turns into `Raw` — fails
//! here instead of surfacing later as a broken MUI binding or JIT lowering.

use copper_syntax::expr::{parse_expr, parse_stmts, Expr, ExprKind};

/// Expression snippets that mirror forms used across `examples/*.crs`:
/// collections.crs, interpolation.crs, matching.crs, optional.crs,
/// ternary.crs, unsafe.crs, loops.crs.
const EXPR_SNIPPETS: &[&str] = &[
    // collections.crs
    "[1, 2, 3, 4, 5]",
    "nums[0]",
    "nums.iter().map(|x| x * 2).collect()",
    "nums.iter().filter(|x| x > threshold).copied().collect()",
    "s.parse::<i32>()",
    "Ok(n.abs())",
    // interpolation.crs
    "\"Hello $name!\"",
    "\"doubled = ${count * 2}\"",
    "count * 2",
    // matching.crs (expression parts)
    "match n { 0 => 0, 1 | 2 | 3 => 1, n if n < 0 => -1, _ => 2 }",
    "Some(7)",
    "vec![10, 20, 30].into_iter()",
    "iter.next()",
    "describe(0)",
    // optional.crs
    "user?.age",
    "user?.name.len()",
    "user?.name.chars().count()",
    "Some(User { name: \"Brian\", age: 30 })",
    // ternary.crs
    "x > 0 ? \"positive\" : \"non-positive\"",
    "x >= 9 ? \"A\" : (x >= 7 ? \"B\" : \"C\")",
    "x < 0 ? -x : x",
    "x % 2 == 0 ? \"even\" : \"odd\"",
    // unsafe.crs (expression parts)
    "&x as *const i32",
    "&mut y",
    "*mptr + 1",
    "msg.to_uppercase()",
    // loops / misc operators
    "0..5",
    "0..=5",
    "n > 0",
    "a && b || c",
    "a & b | c ^ d",
    "x as int",
];

/// Statement-block snippets (Copper bindings have no `let`).
const STMT_SNIPPETS: &[&str] = &[
    "mut nums = [1, 2, 3]\nmut first = nums[0]\nfirst",
    "mut threshold = 3\nthreshold",
    "count = 0\ncount += 1\ncount",
    "mut sign = x > 0 ? \"pos\" : \"neg\"\nsign",
];

/// Walk an expression tree; return true if any node is the `Raw` fallback
/// (i.e. the parser couldn't model that syntax).
fn has_raw(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Raw(_) => true,
        ExprKind::Member { base, .. } => has_raw(base),
        ExprKind::Call { callee, args, .. } => has_raw(callee) || args.iter().any(has_raw),
        ExprKind::Index { base, index } => has_raw(base) || has_raw(index),
        ExprKind::Cast { expr, .. } => has_raw(expr),
        ExprKind::Try { expr } => has_raw(expr),
        ExprKind::Unary { expr, .. } => has_raw(expr),
        ExprKind::Binary { lhs, rhs, .. } => has_raw(lhs) || has_raw(rhs),
        ExprKind::Ternary { cond, then, els } => has_raw(cond) || has_raw(then) || has_raw(els),
        ExprKind::Assign { target, value, .. } => has_raw(target) || has_raw(value),
        ExprKind::Range { start, end, .. } => has_raw(start) || has_raw(end),
        ExprKind::Array(xs) => xs.iter().any(has_raw),
        ExprKind::Tuple(xs) => xs.iter().any(has_raw),
        ExprKind::Closure { body, .. } => has_raw(body),
        ExprKind::StructLit { fields, spread, .. } => {
            fields.iter().any(|(_, v)| has_raw(v)) || spread.as_deref().is_some_and(has_raw)
        }
        ExprKind::If { cond, then, els } => {
            has_raw(cond) || has_raw(then) || els.as_deref().is_some_and(has_raw)
        }
        ExprKind::Match { scrutinee, arms } => {
            has_raw(scrutinee)
                || arms
                    .iter()
                    .any(|a| has_raw(&a.body) || a.guard.as_ref().is_some_and(has_raw))
        }
        ExprKind::Block(b) => b.tail.as_deref().is_some_and(has_raw),
        ExprKind::Literal(_) | ExprKind::Ident(_) | ExprKind::Path { .. } => false,
    }
}

#[test]
fn expression_snippets_parse_without_raw_fallback() {
    let mut failures = Vec::new();
    for src in EXPR_SNIPPETS {
        let (expr, errs) = parse_expr(src);
        match expr {
            None => failures.push(format!("{src:?}: produced no expression")),
            Some(e) => {
                if has_raw(&e) {
                    failures.push(format!("{src:?}: fell back to Raw"));
                }
                if !errs.is_empty() {
                    failures.push(format!("{src:?}: errors {errs:?}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "grammar drift — these real Copper forms are not handled:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn statement_snippets_parse_clean() {
    let mut failures = Vec::new();
    for src in STMT_SNIPPETS {
        let (block, errs) = parse_stmts(src);
        if !errs.is_empty() {
            failures.push(format!("{src:?}: errors {errs:?}"));
        }
        let raw_tail = block.tail.as_deref().is_some_and(has_raw);
        if raw_tail {
            failures.push(format!("{src:?}: tail fell back to Raw"));
        }
    }
    assert!(
        failures.is_empty(),
        "statement drift:\n  {}",
        failures.join("\n  ")
    );
}

/// Read the actual `examples/*.crs` and confirm every interpolated expression
/// (`${ ... }`) inside them parses cleanly — these are real expressions written
/// by the language authors, so they are the truest drift signal.
#[test]
fn example_interpolations_parse() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/copper");
    let mut checked = 0;
    let mut failures = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return, // examples not present in this checkout — skip.
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("crs") {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        for inner in extract_interpolations(&src) {
            checked += 1;
            let (expr, errs) = parse_expr(&inner);
            if expr.as_ref().map(has_raw).unwrap_or(true) || !errs.is_empty() {
                failures.push(format!("{}: ${{{inner}}}", path.display()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "interpolated expressions from real examples failed ({checked} checked):\n  {}",
        failures.join("\n  ")
    );
}

/// Pull the inner source of every `${ ... }` interpolation out of a `.crs`
/// file, tracking brace depth so nested `{}` are handled.
fn extract_interpolations(src: &str) -> Vec<String> {
    let bytes: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == '$' && bytes[i + 1] == '{' {
            let mut depth = 1;
            let mut buf = String::new();
            i += 2;
            while i < bytes.len() && depth > 0 {
                match bytes[i] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                buf.push(bytes[i]);
                i += 1;
            }
            if !buf.trim().is_empty() {
                out.push(buf);
            }
        }
        i += 1;
    }
    out
}
