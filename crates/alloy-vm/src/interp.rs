//! Interpretador tree-walking.

use crate::env::Env;
use crate::error::RuntimeError;
use crate::value::Value;
use copper_syntax::expr::{AssignOp, BinOp, Block, Expr, ExprKind, Literal, Stmt, StrPart, UnOp};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
pub struct Interpreter;

impl Interpreter {
    pub fn new() -> Self {
        Interpreter
    }

    pub fn eval_expr(
        &mut self,
        expr: &Expr,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RuntimeError> {
        match &expr.kind {
            ExprKind::Literal(lit) => self.eval_literal(lit, env),
            ExprKind::Ident(name) => env.borrow().get(name).ok_or_else(|| {
                RuntimeError::new(format!("variável `{name}` não definida"), expr.span)
            }),
            ExprKind::Unary { op, expr: inner } => {
                let v = self.eval_expr(inner, env)?;
                self.eval_unary(*op, v, expr.span)
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let l = self.eval_expr(lhs, env)?;
                let r = self.eval_expr(rhs, env)?;
                self.eval_binary(*op, l, r, expr.span)
            }
            ExprKind::Ternary { cond, then, els } => {
                let c = self.eval_expr(cond, env)?;
                match c.as_bool() {
                    Some(true) => self.eval_expr(then, env),
                    Some(false) => self.eval_expr(els, env),
                    None => Err(RuntimeError::new(
                        format!("condição do ternário não é bool (é {})", c.type_name()),
                        cond.span,
                    )),
                }
            }
            ExprKind::Assign { target, op, value } => {
                let name = match &target.kind {
                    ExprKind::Ident(n) => n.clone(),
                    _ => {
                        return Err(RuntimeError::new(
                            "alvo de atribuição não suportado (só nomes simples no MVP)",
                            target.span,
                        ))
                    }
                };
                let rhs = self.eval_expr(value, env)?;
                let new_val = match op {
                    AssignOp::Plain => rhs,
                    _ => {
                        let cur = env.borrow().get(&name).ok_or_else(|| {
                            RuntimeError::new(
                                format!("variável `{name}` não definida"),
                                target.span,
                            )
                        })?;
                        let binop = match op {
                            AssignOp::Add => BinOp::Add,
                            AssignOp::Sub => BinOp::Sub,
                            AssignOp::Mul => BinOp::Mul,
                            AssignOp::Div => BinOp::Div,
                            AssignOp::Rem => BinOp::Rem,
                            AssignOp::BitAnd => BinOp::BitAnd,
                            AssignOp::BitOr => BinOp::BitOr,
                            AssignOp::BitXor => BinOp::BitXor,
                            AssignOp::Plain => unreachable!(),
                        };
                        self.eval_binary(binop, cur, rhs, expr.span)?
                    }
                };
                if !env.borrow_mut().set(&name, new_val.clone()) {
                    return Err(RuntimeError::new(
                        format!("variável `{name}` não definida"),
                        target.span,
                    ));
                }
                Ok(Value::Unit)
            }
            _ => Err(RuntimeError::new(
                "construção ainda não suportada pela VM",
                expr.span,
            )),
        }
    }

    pub fn eval_block(
        &mut self,
        block: &Block,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RuntimeError> {
        let scope = Env::child(env);
        for stmt in &block.stmts {
            self.eval_stmt(stmt, &scope)?;
        }
        match &block.tail {
            Some(e) => self.eval_expr(e, &scope),
            None => Ok(Value::Unit),
        }
    }

    pub fn eval_stmt(
        &mut self,
        stmt: &Stmt,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RuntimeError> {
        match stmt {
            Stmt::Let { name, value, .. } => {
                let v = match value {
                    Some(e) => self.eval_expr(e, env)?,
                    None => Value::Unit,
                };
                env.borrow_mut().define(name.clone(), v);
                Ok(Value::Unit)
            }
            Stmt::Expr(e) => self.eval_expr(e, env),
            Stmt::IncDec { target, inc, span } => {
                let name = match &target.kind {
                    ExprKind::Ident(n) => n.clone(),
                    _ => return Err(RuntimeError::new("alvo de ++/-- inválido", *span)),
                };
                let cur = env.borrow().get(&name).ok_or_else(|| {
                    RuntimeError::new(format!("variável `{name}` não definida"), *span)
                })?;
                let next = match cur {
                    Value::Int(n) => Value::Int(if *inc { n + 1 } else { n - 1 }),
                    other => {
                        return Err(RuntimeError::new(
                            format!("++/-- requer int, achou {}", other.type_name()),
                            *span,
                        ))
                    }
                };
                env.borrow_mut().set(&name, next);
                Ok(Value::Unit)
            }
            _ => Err(RuntimeError::new(
                "statement ainda não suportado pela VM",
                stmt.span(),
            )),
        }
    }

    fn eval_literal(
        &mut self,
        lit: &Literal,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RuntimeError> {
        Ok(match lit {
            Literal::Int(n) => Value::Int(*n),
            Literal::Float(x) => Value::Float(*x),
            Literal::Bool(b) => Value::Bool(*b),
            Literal::Str(tpl) => {
                let mut out = String::new();
                for part in &tpl.parts {
                    match part {
                        StrPart::Lit(s) => out.push_str(s),
                        StrPart::Expr(e) => out.push_str(&self.eval_expr(e, env)?.to_string()),
                    }
                }
                Value::Str(out)
            }
        })
    }

    fn eval_unary(
        &mut self,
        op: UnOp,
        v: Value,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        match (op, v) {
            (UnOp::Neg, Value::Int(n)) => n
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| RuntimeError::new("overflow em negação de inteiro", span)),
            (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
            (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
            (op, v) => Err(RuntimeError::new(
                format!("operador unário {op:?} inválido para {}", v.type_name()),
                span,
            )),
        }
    }

    pub(crate) fn eval_binary(
        &mut self,
        op: BinOp,
        l: Value,
        r: Value,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        use BinOp::*;
        use Value::*;
        match (op, l, r) {
            (Add, Int(a), Int(b)) => a
                .checked_add(b)
                .map(Int)
                .ok_or_else(|| RuntimeError::new("overflow em soma de inteiros", span)),
            (Sub, Int(a), Int(b)) => a
                .checked_sub(b)
                .map(Int)
                .ok_or_else(|| RuntimeError::new("overflow em subtração de inteiros", span)),
            (Mul, Int(a), Int(b)) => a
                .checked_mul(b)
                .map(Int)
                .ok_or_else(|| RuntimeError::new("overflow em multiplicação de inteiros", span)),
            (Div, Int(a), Int(b)) => a
                .checked_div(b)
                .map(Int)
                .ok_or_else(|| RuntimeError::new("divisão por zero ou overflow", span)),
            (Rem, Int(a), Int(b)) => a
                .checked_rem(b)
                .map(Int)
                .ok_or_else(|| RuntimeError::new("resto por zero ou overflow", span)),
            (Add, Float(a), Float(b)) => Ok(Float(a + b)),
            (Sub, Float(a), Float(b)) => Ok(Float(a - b)),
            (Mul, Float(a), Float(b)) => Ok(Float(a * b)),
            (Div, Float(a), Float(b)) => Ok(Float(a / b)),
            (Add, Str(a), Str(b)) => Ok(Str(a + &b)),
            (Eq, a, b) => Ok(Bool(a == b)),
            (Ne, a, b) => Ok(Bool(a != b)),
            (Lt, Int(a), Int(b)) => Ok(Bool(a < b)),
            (Le, Int(a), Int(b)) => Ok(Bool(a <= b)),
            (Gt, Int(a), Int(b)) => Ok(Bool(a > b)),
            (Ge, Int(a), Int(b)) => Ok(Bool(a >= b)),
            (And, Bool(a), Bool(b)) => Ok(Bool(a && b)),
            (Or, Bool(a), Bool(b)) => Ok(Bool(a || b)),
            (op, a, b) => Err(RuntimeError::new(
                format!(
                    "operador {op:?} inválido para {} e {}",
                    a.type_name(),
                    b.type_name()
                ),
                span,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use copper_syntax::expr::parse_expr;

    fn eval(src: &str) -> Value {
        let (expr, errs) = parse_expr(src);
        assert!(errs.is_empty(), "parse errs: {errs:?}");
        let expr = expr.expect("sem expr");
        let env = Env::new();
        Interpreter::new()
            .eval_expr(&expr, &env)
            .expect("erro de runtime")
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(eval("1 + 2 * 3"), Value::Int(7));
        assert_eq!(eval("(1 + 2) * 3"), Value::Int(9));
        assert_eq!(eval("10 / 2 - 1"), Value::Int(4));
    }

    #[test]
    fn comparisons_and_bool() {
        assert_eq!(eval("1 < 2"), Value::Bool(true));
        assert_eq!(eval("!false"), Value::Bool(true));
        assert_eq!(eval("true && false"), Value::Bool(false));
    }

    fn eval_err(src: &str) -> Result<Value, RuntimeError> {
        let (expr, errs) = parse_expr(src);
        assert!(errs.is_empty(), "parse errs: {errs:?}");
        let expr = expr.expect("sem expr");
        let env = Env::new();
        Interpreter::new().eval_expr(&expr, &env)
    }

    #[test]
    fn div_by_zero_is_err() {
        assert!(eval_err("1 / 0").is_err(), "divisão por zero deve ser Err");
    }

    #[test]
    fn rem_by_zero_is_err() {
        assert!(eval_err("5 % 0").is_err(), "resto por zero deve ser Err");
    }

    fn dummy_span() -> copper_syntax::ast::Span {
        copper_syntax::ast::Span { start: 0, end: 0 }
    }

    #[test]
    fn div_min_over_neg1_is_err() {
        // i64::MIN / -1 overflows — must not panic, must return Err
        let result = Interpreter::new().eval_binary(
            BinOp::Div,
            Value::Int(i64::MIN),
            Value::Int(-1),
            dummy_span(),
        );
        assert!(result.is_err(), "i64::MIN / -1 deve ser Err (overflow)");
    }

    #[test]
    fn rem_min_over_neg1_is_err() {
        // i64::MIN % -1 overflows — must not panic, must return Err
        let result = Interpreter::new().eval_binary(
            BinOp::Rem,
            Value::Int(i64::MIN),
            Value::Int(-1),
            dummy_span(),
        );
        assert!(result.is_err(), "i64::MIN % -1 deve ser Err (overflow)");
    }

    #[test]
    fn undefined_variable_is_err() {
        assert!(
            eval_err("naoexiste").is_err(),
            "variável não definida deve ser Err"
        );
    }

    use copper_syntax::expr::parse_stmts;

    fn run_block(src: &str) -> Value {
        let (block, errs) = parse_stmts(src);
        assert!(errs.is_empty(), "parse errs: {errs:?}");
        let env = Env::new();
        Interpreter::new()
            .eval_block(&block, &env)
            .expect("erro de runtime")
    }

    #[test]
    fn let_assign_incdec() {
        assert_eq!(run_block("mut x = 1\nx = x + 4\nx"), Value::Int(5));
        assert_eq!(run_block("mut y = 10\ny += 5\ny"), Value::Int(15));
        assert_eq!(run_block("mut c = 0\nc++\nc++\nc"), Value::Int(2));
    }

    #[test]
    fn ternary_and_string_interp() {
        assert_eq!(eval("1 < 2 ? 10 : 20"), Value::Int(10));
        // interpolação simples
        assert_eq!(eval("\"v=${1 + 1}\""), Value::Str("v=2".into()));
    }
}
