//! Interpretador tree-walking.

use crate::env::Env;
use crate::error::RuntimeError;
use crate::value::Value;
use copper_syntax::expr::{
    AssignOp, BinOp, Block, Expr, ExprKind, Literal, Pattern, Stmt, StrPart, UnOp,
};
use copper_syntax::program::{Item, Program};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Resultado interno de executar uma sequência de statements.
enum Outcome {
    /// Continuou normal, com o valor de bloco acumulado.
    Normal(Value),
    Return(Value),
    Break,
    Continue,
}

#[derive(Clone)]
struct FuncDef {
    params: Vec<String>,
    body: Block,
}

#[derive(Default)]
pub struct Interpreter {
    funcs: HashMap<String, FuncDef>,
    globals: Option<Rc<RefCell<Env>>>,
}

impl Interpreter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_program(&mut self, prog: &Program) {
        for item in &prog.items {
            if let Item::Function {
                name, params, body, ..
            } = item
            {
                self.funcs.insert(
                    name.clone(),
                    FuncDef {
                        params: params.iter().map(|p| p.name.clone()).collect(),
                        body: body.clone(),
                    },
                );
            }
        }
    }

    pub fn run_program(&mut self, prog: &Program) -> Result<Value, RuntimeError> {
        self.load_program(prog);
        let globals = Env::new();
        self.globals = Some(Rc::clone(&globals));
        let mut last = Value::Unit;
        for item in &prog.items {
            if let Item::Stmt(stmt) = item {
                match self.exec_stmt(stmt, &globals)? {
                    Outcome::Return(v) => return Ok(v),
                    Outcome::Normal(v) => last = v,
                    _ => {}
                }
            }
        }
        if self.funcs.contains_key("main") {
            return self.call_user("main", vec![], copper_syntax::ast::Span::default());
        }
        Ok(last)
    }

    fn call_user(
        &mut self,
        name: &str,
        args: Vec<Value>,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        let def = self
            .funcs
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("função `{name}` não definida"), span))?;
        if def.params.len() != args.len() {
            return Err(RuntimeError::new(
                format!(
                    "`{name}` espera {} args, recebeu {}",
                    def.params.len(),
                    args.len()
                ),
                span,
            ));
        }
        let base = self.globals.clone().unwrap_or_default();
        let scope = Env::child(&base);
        for (p, a) in def.params.iter().zip(args) {
            scope.borrow_mut().define(p.clone(), a);
        }
        match self.run_block_in(&def.body, &scope)? {
            Outcome::Return(v) => Ok(v),
            Outcome::Normal(v) => Ok(v),
            Outcome::Break | Outcome::Continue => {
                Err(RuntimeError::new("break/continue fora de loop", span))
            }
        }
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
            ExprKind::Call { callee, args, .. } => {
                let arg_vals: Vec<Value> = args
                    .iter()
                    .map(|a| self.eval_expr(a, env))
                    .collect::<Result<_, _>>()?;
                match &callee.kind {
                    ExprKind::Ident(name) if name == "println" || name == "print" => {
                        let line = arg_vals
                            .iter()
                            .map(|v| v.to_string())
                            .collect::<Vec<_>>()
                            .join(" ");
                        if name == "println" {
                            println!("{line}");
                        } else {
                            print!("{line}");
                        }
                        Ok(Value::Unit)
                    }
                    ExprKind::Ident(name) => {
                        let name = name.clone();
                        self.call_user(&name, arg_vals, expr.span)
                    }
                    _ => Err(RuntimeError::new(
                        "alvo de chamada não suportado",
                        callee.span,
                    )),
                }
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
        match self.run_block(block, env)? {
            Outcome::Normal(v) => Ok(v),
            Outcome::Return(v) => Ok(v),
            Outcome::Break | Outcome::Continue => Err(RuntimeError::new(
                "break/continue fora de um loop",
                block.span,
            )),
        }
    }

    fn run_block(
        &mut self,
        block: &Block,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Outcome, RuntimeError> {
        let scope = Env::child(env);
        for stmt in &block.stmts {
            match self.exec_stmt(stmt, &scope)? {
                Outcome::Normal(_) => {}
                other => return Ok(other),
            }
        }
        match &block.tail {
            Some(e) => Ok(Outcome::Normal(self.eval_expr(e, &scope)?)),
            None => Ok(Outcome::Normal(Value::Unit)),
        }
    }

    /// Como `run_block`, mas sem criar um escopo filho extra (o chamador já
    /// criou um — usado pelo `for`, que injeta a variável de laço).
    fn run_block_in(
        &mut self,
        block: &Block,
        scope: &Rc<RefCell<Env>>,
    ) -> Result<Outcome, RuntimeError> {
        for stmt in &block.stmts {
            match self.exec_stmt(stmt, scope)? {
                Outcome::Normal(_) => {}
                other => return Ok(other),
            }
        }
        match &block.tail {
            Some(e) => Ok(Outcome::Normal(self.eval_expr(e, scope)?)),
            None => Ok(Outcome::Normal(Value::Unit)),
        }
    }

    fn exec_stmt(&mut self, stmt: &Stmt, env: &Rc<RefCell<Env>>) -> Result<Outcome, RuntimeError> {
        match stmt {
            Stmt::Let { name, value, .. } => {
                let v = match value {
                    Some(e) => self.eval_expr(e, env)?,
                    None => Value::Unit,
                };
                env.borrow_mut().define(name.clone(), v);
                Ok(Outcome::Normal(Value::Unit))
            }
            Stmt::Expr(e) => Ok(Outcome::Normal(self.eval_expr(e, env)?)),
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
                Ok(Outcome::Normal(Value::Unit))
            }
            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => self.eval_expr(e, env)?,
                    None => Value::Unit,
                };
                Ok(Outcome::Return(v))
            }
            Stmt::Break { .. } => Ok(Outcome::Break),
            Stmt::Continue { .. } => Ok(Outcome::Continue),
            Stmt::If {
                cond,
                let_pattern,
                then,
                els,
                span,
            } => {
                if let_pattern.is_some() {
                    return Err(RuntimeError::new(
                        "`if let` ainda não suportado pela VM",
                        *span,
                    ));
                }
                let c = self.eval_expr(cond, env)?;
                match c.as_bool() {
                    Some(true) => self.run_block(then, env),
                    Some(false) => match els {
                        Some(s) => self.exec_stmt(s, env),
                        None => Ok(Outcome::Normal(Value::Unit)),
                    },
                    None => Err(RuntimeError::new("condição de `if` não é bool", cond.span)),
                }
            }
            Stmt::While {
                cond,
                let_pattern,
                body,
                span,
            } => {
                if let_pattern.is_some() {
                    return Err(RuntimeError::new(
                        "`while let` ainda não suportado pela VM",
                        *span,
                    ));
                }
                loop {
                    let c = self.eval_expr(cond, env)?;
                    match c.as_bool() {
                        Some(true) => match self.run_block(body, env)? {
                            Outcome::Break => break,
                            Outcome::Return(v) => return Ok(Outcome::Return(v)),
                            _ => {}
                        },
                        Some(false) => break,
                        None => {
                            return Err(RuntimeError::new(
                                "condição de `while` não é bool",
                                cond.span,
                            ))
                        }
                    }
                }
                Ok(Outcome::Normal(Value::Unit))
            }
            Stmt::Loop { body, .. } => {
                loop {
                    match self.run_block(body, env)? {
                        Outcome::Break => break,
                        Outcome::Return(v) => return Ok(Outcome::Return(v)),
                        _ => {}
                    }
                }
                Ok(Outcome::Normal(Value::Unit))
            }
            Stmt::For {
                pattern,
                iter,
                body,
                span,
            } => {
                let var = match pattern {
                    Pattern::Ident(n) => n.clone(),
                    _ => {
                        return Err(RuntimeError::new(
                            "padrão de `for` não suportado (só nome simples)",
                            *span,
                        ))
                    }
                };
                let (start, end, inclusive) = match &iter.kind {
                    ExprKind::Range {
                        start,
                        end,
                        inclusive,
                    } => {
                        let s = self.eval_expr(start, env)?;
                        let e = self.eval_expr(end, env)?;
                        match (s, e) {
                            (Value::Int(s), Value::Int(e)) => (s, e, *inclusive),
                            _ => {
                                return Err(RuntimeError::new(
                                    "`for` só itera ranges de int no MVP",
                                    *span,
                                ))
                            }
                        }
                    }
                    _ => return Err(RuntimeError::new("`for` só itera ranges no MVP", iter.span)),
                };
                let last = if inclusive { end + 1 } else { end };
                let mut i = start;
                while i < last {
                    let scope = Env::child(env);
                    scope.borrow_mut().define(var.clone(), Value::Int(i));
                    match self.run_block_in(body, &scope)? {
                        Outcome::Break => break,
                        Outcome::Return(v) => return Ok(Outcome::Return(v)),
                        _ => {}
                    }
                    i += 1;
                }
                Ok(Outcome::Normal(Value::Unit))
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
    use copper_syntax::program::parse_program;

    fn run_prog(src: &str) -> Value {
        let prog = parse_program(src);
        assert!(prog.errors.is_empty(), "prog errs: {:?}", prog.errors);
        Interpreter::new()
            .run_program(&prog)
            .expect("erro de runtime")
    }

    #[test]
    fn user_function_with_return() {
        let v = run_prog("func int add(a: int, b: int) { return a + b }\nmut r = add(2, 3)\nr");
        assert_eq!(v, Value::Int(5));
    }

    #[test]
    fn recursion_factorial() {
        let src = "func int fac(n: int) { if n <= 1 { return 1 }; return n * fac(n - 1) }\nfac(5)";
        assert_eq!(run_prog(src), Value::Int(120));
    }

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

    #[test]
    fn while_loop_accumulates() {
        let v = run_block("mut i = 0\nmut sum = 0\nwhile i < 5 { sum += i; i++ }\nsum");
        assert_eq!(v, Value::Int(10)); // 0+1+2+3+4
    }

    #[test]
    fn for_range_and_break() {
        assert_eq!(
            run_block("mut s = 0\nfor n in 1..4 { s += n }\ns"),
            Value::Int(6)
        ); // 1+2+3
        assert_eq!(
            run_block("mut s = 0\nfor n in 0..100 { if n == 3 { break }; s += n }\ns"),
            Value::Int(3)
        ); // 0+1+2
    }

    #[test]
    fn if_else_branches() {
        assert_eq!(
            run_block("mut x = 0\nif 1 < 2 { x += 10 } else { x += 20 }\nx"),
            Value::Int(10)
        );
    }
}
