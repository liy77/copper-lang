//! Build-time transpiler: lowers a Copper control-flow `Expr` into a Rust
//! expression string, emitted inside the view fn so it re-evaluates against
//! live signal values each rebuild. Reactivity is whole-view rebuild, not
//! const-folding — nothing here is evaluated at codegen time.

use copper_syntax::expr::{BinOp, Expr, ExprKind, Literal, UnOp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigKind {
    Int,
    Str,
    List,
}

fn sig_var(name: &str) -> String {
    format!("__sig_{}", super::sanitize_ident(name))
}

/// Lower `expr` to a Rust expression string. `sig` classifies a name as a
/// signal kind; `env` supplies a literal substitution for non-signal params.
pub fn expr_to_rust(
    expr: &Expr,
    sig: &dyn Fn(&str) -> Option<SigKind>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    match &expr.kind {
        ExprKind::Literal(Literal::Int(n)) => Ok(format!("{n}")),
        ExprKind::Literal(Literal::Bool(b)) => Ok(format!("{b}")),
        ExprKind::Literal(Literal::Float(f)) => Ok(format!("{f}f64")),
        ExprKind::Literal(Literal::Str(t)) => {
            // Only plain literal strings (no interpolation) are supported here.
            let s = super::str_template_literal(t)
                .ok_or_else(|| "interpolated string in control-flow expr".to_string())?;
            Ok(format!("{s:?}.to_string()"))
        }
        ExprKind::Ident(name) => match sig(name) {
            Some(SigKind::Int) => Ok(format!("{}.borrow().get()", sig_var(name))),
            Some(SigKind::Str) => Ok(format!("{}.borrow().get()", sig_var(name))),
            Some(SigKind::List) => Ok(format!("{}.borrow().clone()", sig_var(name))),
            None => env(name)
                .ok_or_else(|| format!("unknown name `{name}` in control-flow expr")),
        },
        ExprKind::Unary { op, expr } => {
            let inner = expr_to_rust(expr, sig, env)?;
            let o = match op {
                UnOp::Not => "!",
                UnOp::Neg => "-",
                other => {
                    return Err(format!(
                        "unsupported unary operator {other:?} in control-flow expr"
                    ))
                }
            };
            Ok(format!("({o}{inner})"))
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let l = expr_to_rust(lhs, sig, env)?;
            let r = expr_to_rust(rhs, sig, env)?;
            // `<str signal> == "lit"`: rhs literal already `.to_string()`-suffixed
            // by the Str-literal arm, so the comparison is String == String.
            let o = match op {
                BinOp::Eq => "==",
                BinOp::Ne => "!=",
                BinOp::Lt => "<",
                BinOp::Gt => ">",
                BinOp::Le => "<=",
                BinOp::Ge => ">=",
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Rem => "%",
                BinOp::And => "&&",
                BinOp::Or => "||",
                other => {
                    return Err(format!(
                        "unsupported operator {other:?} in control-flow expr"
                    ))
                }
            };
            Ok(format!("({l} {o} {r})"))
        }
        // `items.len()` → Call{ callee: Member{ base, "len" }, args: [] }
        ExprKind::Call { callee, args, .. } if args.is_empty() => {
            if let ExprKind::Member { base, field, .. } = &callee.kind {
                if field == "len" {
                    let b = expr_to_rust(base, sig, env)?;
                    return Ok(format!("({b}.len() as i32)"));
                }
            }
            Err("unsupported call in control-flow expr".to_string())
        }
        ExprKind::Call { .. } => Err("unsupported call in control-flow expr".to_string()),
        ExprKind::Index { base, index } => {
            let b = expr_to_rust(base, sig, env)?;
            let i = expr_to_rust(index, sig, env)?;
            Ok(format!("{b}[({i}) as usize].clone()"))
        }
        ExprKind::Array(items) => {
            let parts: Result<Vec<_>, _> =
                items.iter().map(|x| expr_to_rust(x, sig, env)).collect();
            Ok(format!("vec![{}]", parts?.join(", ")))
        }
        ExprKind::Range {
            start,
            end,
            inclusive,
        } => {
            let s = expr_to_rust(start, sig, env)?;
            let en = expr_to_rust(end, sig, env)?;
            Ok(format!(
                "({s}..{}{en})",
                if *inclusive { "=" } else { "" }
            ))
        }
        other => Err(format!(
            "unsupported expression {other:?} in control-flow position"
        )),
    }
}

/// Append every bare identifier referenced in `expr` (caller filters to signals).
pub fn collect_reads(expr: &Expr, out: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::Ident(n) => out.push(n.clone()),
        ExprKind::Unary { expr, .. } => collect_reads(expr, out),
        ExprKind::Binary { lhs, rhs, .. } => {
            collect_reads(lhs, out);
            collect_reads(rhs, out);
        }
        ExprKind::Call { callee, args, .. } => {
            collect_reads(callee, out);
            args.iter().for_each(|a| collect_reads(a, out));
        }
        ExprKind::Member { base, .. } => collect_reads(base, out),
        ExprKind::Index { base, index } => {
            collect_reads(base, out);
            collect_reads(index, out);
        }
        ExprKind::Array(xs) => xs.iter().for_each(|x| collect_reads(x, out)),
        ExprKind::Range { start, end, .. } => {
            collect_reads(start, out);
            collect_reads(end, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use copper_syntax::expr::parse_expr;

    fn sig(name: &str) -> Option<SigKind> {
        match name {
            "count" => Some(SigKind::Int),
            "status" => Some(SigKind::Str),
            "items" => Some(SigKind::List),
            _ => None,
        }
    }
    fn env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn lowers_int_signal_comparison() {
        let e = parse_expr("count > 3").0.unwrap();
        let rust = expr_to_rust(&e, &sig, &env).unwrap();
        assert_eq!(rust, "(__sig_count.borrow().get() > 3)");
    }

    #[test]
    fn lowers_string_signal_eq() {
        let e = parse_expr("status == \"on\"").0.unwrap();
        let rust = expr_to_rust(&e, &sig, &env).unwrap();
        assert_eq!(rust, "(__sig_status.borrow().get() == \"on\".to_string())");
    }

    #[test]
    fn lowers_list_iter_source() {
        let e = parse_expr("items").0.unwrap();
        let rust = expr_to_rust(&e, &sig, &env).unwrap();
        assert_eq!(rust, "__sig_items.borrow().clone()");
    }

    #[test]
    fn rejects_unsupported() {
        let e = parse_expr("foo(bar)").0.unwrap(); // unknown call
        assert!(expr_to_rust(&e, &sig, &env).is_err());
    }

    #[test]
    fn collects_idents() {
        let e = parse_expr("count > 3 && status == \"x\"").0.unwrap();
        let mut v = Vec::new();
        collect_reads(&e, &mut v);
        assert!(v.contains(&"count".to_string()) && v.contains(&"status".to_string()));
    }
}
