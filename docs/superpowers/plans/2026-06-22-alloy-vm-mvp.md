# Alloy VM — MVP (Interpretador Tree-Walking) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Criar o crate `alloy-vm` e o binário `alloy` capaz de interpretar um `.crs` (`alloy run file.crs`) cobrindo expressões, variáveis, controle de fluxo, funções definidas pelo usuário e `println`, executando direto sobre o `Program` AST de `copper-syntax`.

**Architecture:** Interpretador tree-walking sobre `copper_syntax::program::parse_program`. Sem bytecode, sem JIT. O crate expõe uma lib (`Interpreter`) + um binário (`alloy`). Reaproveita 100% da AST existente (`expr.rs` + `program.rs`); nenhum re-parse.

**Tech Stack:** Rust (edition 2021), workspace Cargo já existente, dependência de path em `crates/copper-syntax`. `clap` para o CLI do binário (já usado pelo `cforge`).

## Global Constraints

- Edition: `2021` (igual aos outros crates do workspace).
- O crate `alloy-vm` deve ser **isolado**: `cforge`, `mui-*` e `copper-parser` NÃO podem depender dele.
- Toolchain pin: `stable` (`rust-toolchain.toml`). Código deve passar `cargo fmt --check` e `cargo clippy -- -D warnings`.
- A AST é fonte de verdade e **não deve ser modificada** neste plano — apenas consumida. Gaps de cobertura (`ExprKind::Raw`) são fora de escopo do MVP: ao encontrar `Raw`, o interpretador retorna um erro de runtime claro ("construção ainda não suportada pela VM"), nunca panica.
- Tipos consumidos de `copper_syntax`: `program::{parse_program, Program, Item, Param, Block}`, `expr::{Expr, ExprKind, Stmt, Literal, StrPart, StrTemplate, BinOp, UnOp, AssignOp, Pattern}`.
- A VM **nunca panica** em entrada malformada: erros viram `RuntimeError` com `Span`.

---

### Task 1: Scaffold do crate `alloy-vm` + tipo `Value`

**Files:**
- Create: `crates/alloy-vm/Cargo.toml`
- Create: `crates/alloy-vm/src/lib.rs`
- Create: `crates/alloy-vm/src/value.rs`
- Modify: `Cargo.toml:2` (raiz — adicionar membro ao workspace)
- Test: `crates/alloy-vm/src/value.rs` (módulo `#[cfg(test)]`)

**Interfaces:**
- Produces: `alloy_vm::value::Value` (enum `Int(i64)`, `Float(f64)`, `Bool(bool)`, `Str(String)`, `Unit`), `Value::type_name(&self) -> &'static str`, `impl std::fmt::Display for Value`.

- [ ] **Step 1: Criar `crates/alloy-vm/Cargo.toml`**

```toml
[package]
name = "alloy-vm"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "alloy"
path = "src/bin/alloy.rs"

[lib]
name = "alloy_vm"
path = "src/lib.rs"

[dependencies]
copper-syntax = { path = "../copper-syntax" }
clap = { version = "4", features = ["derive"] }
```

- [ ] **Step 2: Adicionar o crate ao workspace**

Em `Cargo.toml` da raiz, na linha `members = [...]`, acrescentar `"crates/alloy-vm"`:

```toml
members = [".", "crates/copper-syntax", "crates/copper-parser", "crates/copper-lsp", "crates/mui-syntax", "crates/mui-codegen", "crates/mui-lsp", "crates/alloy-vm"]
```

- [ ] **Step 3: Escrever `crates/alloy-vm/src/value.rs` com o teste falhando**

```rust
//! Valores em runtime da VM Alloy.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
}

impl Value {
    /// Nome do tipo para mensagens de erro.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::Str(_) => "str",
            Value::Unit => "unit",
        }
    }

    /// Verdade de um valor em contexto booleano (só `Bool` é válido).
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Unit => write!(f, "()"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_type_name() {
        assert_eq!(Value::Int(42).to_string(), "42");
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::Str("oi".into()).to_string(), "oi");
        assert_eq!(Value::Int(1).type_name(), "int");
        assert_eq!(Value::Bool(false).as_bool(), Some(false));
        assert_eq!(Value::Int(1).as_bool(), None);
    }
}
```

- [ ] **Step 4: Escrever `crates/alloy-vm/src/lib.rs`**

```rust
//! Alloy: interpretador tree-walking para Copper.

pub mod value;
```

- [ ] **Step 5: Rodar o teste e verificar que passa**

Run: `cargo test -p alloy-vm value::tests::display_and_type_name`
Expected: PASS (1 test)

- [ ] **Step 6: Commit**

```bash
git add crates/alloy-vm/Cargo.toml crates/alloy-vm/src/lib.rs crates/alloy-vm/src/value.rs Cargo.toml Cargo.lock
git commit -m "feat(alloy): scaffold alloy-vm crate with Value type"
```

---

### Task 2: Ambiente de escopos (`Env`) + erro de runtime

**Files:**
- Create: `crates/alloy-vm/src/error.rs`
- Create: `crates/alloy-vm/src/env.rs`
- Modify: `crates/alloy-vm/src/lib.rs`
- Test: `crates/alloy-vm/src/env.rs` (módulo `#[cfg(test)]`)

**Interfaces:**
- Consumes: `Value` (Task 1), `copper_syntax::ast::Span`.
- Produces:
  - `alloy_vm::error::RuntimeError { message: String, span: Span }` + `RuntimeError::new(msg, span)`.
  - `alloy_vm::error::Flow` (enum de controle: `Err(RuntimeError)`, `Return(Value)`, `Break`, `Continue`).
  - `alloy_vm::env::Env` com `new()`, `child(&Rc<RefCell<Env>>) -> Rc<RefCell<Env>>`, `define(&mut self, name, value)`, `get(&self, name) -> Option<Value>`, `set(&mut self, name, value) -> bool` (atualiza no escopo onde o nome existe; `false` se não existe).

- [ ] **Step 1: Escrever `crates/alloy-vm/src/error.rs`**

```rust
//! Erros e fluxo de controle não-local da VM.

use crate::value::Value;
use copper_syntax::ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeError {
    pub message: String,
    pub span: Span,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

/// Resultado de avaliar um statement: ou seguiu normalmente, ou disparou
/// controle de fluxo não-local (return/break/continue), ou erro.
#[derive(Debug, Clone, PartialEq)]
pub enum Flow {
    /// Seguiu normal.
    Normal,
    Return(Value),
    Break,
    Continue,
    Err(RuntimeError),
}
```

- [ ] **Step 2: Escrever `crates/alloy-vm/src/env.rs` com o teste falhando**

```rust
//! Ambiente de variáveis: cadeia de escopos com pai compartilhado.

use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Default)]
pub struct Env {
    vars: HashMap<String, Value>,
    parent: Option<Rc<RefCell<Env>>>,
}

impl Env {
    pub fn new() -> Rc<RefCell<Env>> {
        Rc::new(RefCell::new(Env::default()))
    }

    /// Cria um escopo filho que enxerga o pai.
    pub fn child(parent: &Rc<RefCell<Env>>) -> Rc<RefCell<Env>> {
        Rc::new(RefCell::new(Env {
            vars: HashMap::new(),
            parent: Some(Rc::clone(parent)),
        }))
    }

    pub fn define(&mut self, name: impl Into<String>, value: Value) {
        self.vars.insert(name.into(), value);
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.vars.get(name) {
            Some(v.clone())
        } else if let Some(p) = &self.parent {
            p.borrow().get(name)
        } else {
            None
        }
    }

    /// Atualiza uma variável existente no escopo onde ela foi definida.
    /// Retorna `false` se o nome não existe em nenhum escopo.
    pub fn set(&mut self, name: &str, value: Value) -> bool {
        if self.vars.contains_key(name) {
            self.vars.insert(name.to_string(), value);
            true
        } else if let Some(p) = &self.parent {
            p.borrow_mut().set(name, value)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_sees_parent_and_set_updates_origin() {
        let root = Env::new();
        root.borrow_mut().define("x", Value::Int(1));

        let child = Env::child(&root);
        // filho enxerga o pai
        assert_eq!(child.borrow().get("x"), Some(Value::Int(1)));
        // set atualiza no escopo de origem (o pai)
        assert!(child.borrow_mut().set("x", Value::Int(9)));
        assert_eq!(root.borrow().get("x"), Some(Value::Int(9)));
        // set em nome inexistente falha
        assert!(!child.borrow_mut().set("y", Value::Int(0)));
    }
}
```

- [ ] **Step 3: Registrar os módulos em `lib.rs`**

```rust
//! Alloy: interpretador tree-walking para Copper.

pub mod env;
pub mod error;
pub mod value;
```

- [ ] **Step 4: Rodar o teste e verificar que passa**

Run: `cargo test -p alloy-vm env::tests::child_sees_parent_and_set_updates_origin`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/alloy-vm/src/env.rs crates/alloy-vm/src/error.rs crates/alloy-vm/src/lib.rs
git commit -m "feat(alloy): scoped Env and RuntimeError/Flow"
```

---

### Task 3: Avaliar expressões puras (literais, binário, unário, ternário)

**Files:**
- Create: `crates/alloy-vm/src/interp.rs`
- Modify: `crates/alloy-vm/src/lib.rs`
- Test: `crates/alloy-vm/src/interp.rs` (módulo `#[cfg(test)]`)

**Interfaces:**
- Consumes: `Value`, `Env`, `RuntimeError` (Tasks 1-2); `copper_syntax::expr::{Expr, ExprKind, Literal, StrPart, BinOp, UnOp, parse_expr}`.
- Produces: `alloy_vm::interp::Interpreter` com `Interpreter::new()` e `eval_expr(&mut self, expr: &Expr, env: &Rc<RefCell<Env>>) -> Result<Value, RuntimeError>`. (Neste passo cobre só literais escalares, `Ident`, `Binary`, `Unary` Neg/Not, `Ternary`. Demais variantes retornam `RuntimeError` "não suportado".)

- [ ] **Step 1: Escrever `crates/alloy-vm/src/interp.rs` com testes falhando**

```rust
//! Interpretador tree-walking.

use crate::env::Env;
use crate::error::RuntimeError;
use crate::value::Value;
use copper_syntax::expr::{BinOp, Expr, ExprKind, Literal, StrPart, UnOp};
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
            ExprKind::Ident(name) => env
                .borrow()
                .get(name)
                .ok_or_else(|| RuntimeError::new(format!("variável `{name}` não definida"), expr.span)),
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
            _ => Err(RuntimeError::new(
                "construção ainda não suportada pela VM",
                expr.span,
            )),
        }
    }

    fn eval_literal(&mut self, lit: &Literal, env: &Rc<RefCell<Env>>) -> Result<Value, RuntimeError> {
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

    fn eval_unary(&mut self, op: UnOp, v: Value, span: copper_syntax::ast::Span) -> Result<Value, RuntimeError> {
        match (op, v) {
            (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
            (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
            (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
            (op, v) => Err(RuntimeError::new(
                format!("operador unário {op:?} inválido para {}", v.type_name()),
                span,
            )),
        }
    }

    fn eval_binary(&mut self, op: BinOp, l: Value, r: Value, span: copper_syntax::ast::Span) -> Result<Value, RuntimeError> {
        use BinOp::*;
        use Value::*;
        match (op, l, r) {
            (Add, Int(a), Int(b)) => Ok(Int(a + b)),
            (Sub, Int(a), Int(b)) => Ok(Int(a - b)),
            (Mul, Int(a), Int(b)) => Ok(Int(a * b)),
            (Div, Int(a), Int(b)) if b != 0 => Ok(Int(a / b)),
            (Div, Int(_), Int(_)) => Err(RuntimeError::new("divisão por zero", span)),
            (Rem, Int(a), Int(b)) if b != 0 => Ok(Int(a % b)),
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
                format!("operador {op:?} inválido para {} e {}", a.type_name(), b.type_name()),
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
        Interpreter::new().eval_expr(&expr, &env).expect("erro de runtime")
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

    #[test]
    fn ternary_and_string_interp() {
        assert_eq!(eval("1 < 2 ? 10 : 20"), Value::Int(10));
        // interpolação simples
        assert_eq!(eval("\"v=${1 + 1}\""), Value::Str("v=2".into()));
    }
}
```

- [ ] **Step 2: Registrar `interp` em `lib.rs`**

```rust
//! Alloy: interpretador tree-walking para Copper.

pub mod env;
pub mod error;
pub mod interp;
pub mod value;
```

- [ ] **Step 3: Rodar os testes e verificar que passam**

Run: `cargo test -p alloy-vm interp::tests`
Expected: PASS (3 tests). Se algum literal de interpolação falhar no parse, ajustar a string de teste mantendo a asserção de valor — a forma exata do template vem de `copper-syntax`.

- [ ] **Step 4: Commit**

```bash
git add crates/alloy-vm/src/interp.rs crates/alloy-vm/src/lib.rs
git commit -m "feat(alloy): eval scalar exprs (literals, binary, unary, ternary)"
```

---

### Task 4: Avaliar statements de variáveis (let/mut, assign, expr-stmt, ++/--)

**Files:**
- Modify: `crates/alloy-vm/src/interp.rs`
- Test: `crates/alloy-vm/src/interp.rs` (novos testes)

**Interfaces:**
- Consumes: `copper_syntax::expr::{Stmt, AssignOp, Block}`.
- Produces:
  - `Interpreter::eval_block(&mut self, block: &Block, env: &Rc<RefCell<Env>>) -> Result<Value, RuntimeError>` — executa stmts num escopo filho; retorna o valor do `tail` (ou `Unit`). Propaga erro. (Controle de fluxo return/break/continue entra na Task 5.)
  - `Interpreter::eval_stmt(&mut self, stmt: &Stmt, env: &Rc<RefCell<Env>>) -> Result<Value, RuntimeError>` — cobre `Let`, `Expr`, `IncDec` e `Expr(ExprKind::Assign)`. Demais variantes retornam erro "não suportado" por enquanto.

- [ ] **Step 1: Adicionar `eval_block`, `eval_stmt` e suporte a `Assign` em `eval_expr`**

Adicionar ao `match` de `eval_expr` (antes do braço `_`):

```rust
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
                            RuntimeError::new(format!("variável `{name}` não definida"), target.span)
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
```

Adicionar o import de `AssignOp` e `Stmt`/`Block` no topo:

```rust
use copper_syntax::expr::{AssignOp, BinOp, Block, Expr, ExprKind, Literal, Stmt, StrPart, UnOp};
```

Adicionar os dois métodos ao `impl Interpreter`:

```rust
    pub fn eval_block(&mut self, block: &Block, env: &Rc<RefCell<Env>>) -> Result<Value, RuntimeError> {
        let scope = Env::child(env);
        for stmt in &block.stmts {
            self.eval_stmt(stmt, &scope)?;
        }
        match &block.tail {
            Some(e) => self.eval_expr(e, &scope),
            None => Ok(Value::Unit),
        }
    }

    pub fn eval_stmt(&mut self, stmt: &Stmt, env: &Rc<RefCell<Env>>) -> Result<Value, RuntimeError> {
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
                    other => return Err(RuntimeError::new(
                        format!("++/-- requer int, achou {}", other.type_name()),
                        *span,
                    )),
                };
                env.borrow_mut().set(&name, next);
                Ok(Value::Unit)
            }
            _ => Err(RuntimeError::new("statement ainda não suportado pela VM", stmt.span())),
        }
    }
```

- [ ] **Step 2: Escrever os testes falhando**

Adicionar ao `mod tests`:

```rust
    use copper_syntax::expr::parse_stmts;

    fn run_block(src: &str) -> Value {
        let (block, errs) = parse_stmts(src);
        assert!(errs.is_empty(), "parse errs: {errs:?}");
        let env = Env::new();
        Interpreter::new().eval_block(&block, &env).expect("erro de runtime")
    }

    #[test]
    fn let_assign_incdec() {
        assert_eq!(run_block("mut x = 1\nx = x + 4\nx"), Value::Int(5));
        assert_eq!(run_block("mut y = 10\ny += 5\ny"), Value::Int(15));
        assert_eq!(run_block("mut c = 0\nc++\nc++\nc"), Value::Int(2));
    }
```

- [ ] **Step 3: Rodar os testes**

Run: `cargo test -p alloy-vm interp::tests::let_assign_incdec`
Expected: PASS. Se o parser de `parse_stmts` exigir um `tail` separado, manter a última linha (`x`/`y`/`c`) como expressão final — é o valor retornado por `eval_block`.

- [ ] **Step 4: Commit**

```bash
git add crates/alloy-vm/src/interp.rs
git commit -m "feat(alloy): eval let/assign/incdec statements and blocks"
```

---

### Task 5: Controle de fluxo (if / while / loop / for / break / continue)

**Files:**
- Modify: `crates/alloy-vm/src/interp.rs`
- Test: `crates/alloy-vm/src/interp.rs` (novos testes)

**Interfaces:**
- Consumes: `copper_syntax::expr::{Pattern}`, `ExprKind::{If, Range, Block}`.
- Produces: refatoração de `eval_block`/`eval_stmt` para propagar `Flow` (Break/Continue/Return) via um enum interno; `if`/`while`/`loop`/`for` implementados. `eval_block` passa a retornar `Result<BlockOutcome, RuntimeError>` onde `BlockOutcome` distingue valor normal de fluxo não-local. Manter a assinatura pública `eval_block -> Result<Value, RuntimeError>` como wrapper que converte `Return(v)`→`v` e `Break`/`Continue` no topo→erro "fora de loop".

- [ ] **Step 1: Introduzir `BlockOutcome` e reescrever a execução de stmts**

Adicionar o enum interno e refatorar. No topo do `impl`:

```rust
/// Resultado interno de executar uma sequência de statements.
enum Outcome {
    /// Continuou normal, com o valor de bloco acumulado.
    Normal(Value),
    Return(Value),
    Break,
    Continue,
}
```

Reescrever `eval_block` para usar um helper que executa stmts e respeita `Outcome`:

```rust
    pub fn eval_block(&mut self, block: &Block, env: &Rc<RefCell<Env>>) -> Result<Value, RuntimeError> {
        match self.run_block(block, env)? {
            Outcome::Normal(v) => Ok(v),
            Outcome::Return(v) => Ok(v),
            Outcome::Break | Outcome::Continue => {
                Err(RuntimeError::new("break/continue fora de um loop", block.span))
            }
        }
    }

    fn run_block(&mut self, block: &Block, env: &Rc<RefCell<Env>>) -> Result<Outcome, RuntimeError> {
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
```

Renomear o antigo `eval_stmt` para `exec_stmt` retornando `Outcome`, e adicionar os braços de controle de fluxo:

```rust
    fn exec_stmt(&mut self, stmt: &Stmt, env: &Rc<RefCell<Env>>) -> Result<Outcome, RuntimeError> {
        match stmt {
            Stmt::Let { name, value, .. } => {
                let v = match value { Some(e) => self.eval_expr(e, env)?, None => Value::Unit };
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
                    other => return Err(RuntimeError::new(format!("++/-- requer int, achou {}", other.type_name()), *span)),
                };
                env.borrow_mut().set(&name, next);
                Ok(Outcome::Normal(Value::Unit))
            }
            Stmt::Return { value, .. } => {
                let v = match value { Some(e) => self.eval_expr(e, env)?, None => Value::Unit };
                Ok(Outcome::Return(v))
            }
            Stmt::Break { .. } => Ok(Outcome::Break),
            Stmt::Continue { .. } => Ok(Outcome::Continue),
            Stmt::If { cond, let_pattern, then, els, span } => {
                if let_pattern.is_some() {
                    return Err(RuntimeError::new("`if let` ainda não suportado pela VM", *span));
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
            Stmt::While { cond, let_pattern, body, span } => {
                if let_pattern.is_some() {
                    return Err(RuntimeError::new("`while let` ainda não suportado pela VM", *span));
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
                        None => return Err(RuntimeError::new("condição de `while` não é bool", cond.span)),
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
            Stmt::For { pattern, iter, body, span } => {
                let var = match pattern {
                    Pattern::Ident(n) => n.clone(),
                    _ => return Err(RuntimeError::new("padrão de `for` não suportado (só nome simples)", *span)),
                };
                let (start, end, inclusive) = match &iter.kind {
                    ExprKind::Range { start, end, inclusive } => {
                        let s = self.eval_expr(start, env)?;
                        let e = self.eval_expr(end, env)?;
                        match (s, e) {
                            (Value::Int(s), Value::Int(e)) => (s, e, *inclusive),
                            _ => return Err(RuntimeError::new("`for` só itera ranges de int no MVP", *span)),
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
            _ => Err(RuntimeError::new("statement ainda não suportado pela VM", stmt.span())),
        }
    }

    /// Como `run_block`, mas sem criar um escopo filho extra (o chamador já
    /// criou um — usado pelo `for`, que injeta a variável de laço).
    fn run_block_in(&mut self, block: &Block, scope: &Rc<RefCell<Env>>) -> Result<Outcome, RuntimeError> {
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
```

Adicionar `Pattern` ao import:

```rust
use copper_syntax::expr::{AssignOp, BinOp, Block, Expr, ExprKind, Literal, Pattern, Stmt, StrPart, UnOp};
```

Atualizar o teste antigo `let_assign_incdec` se referenciava `eval_stmt` diretamente — ele usa `eval_block`, então segue válido.

- [ ] **Step 2: Escrever os testes falhando**

```rust
    #[test]
    fn while_loop_accumulates() {
        let v = run_block("mut i = 0\nmut sum = 0\nwhile i < 5 { sum += i; i++ }\nsum");
        assert_eq!(v, Value::Int(10)); // 0+1+2+3+4
    }

    #[test]
    fn for_range_and_break() {
        assert_eq!(run_block("mut s = 0\nfor n in 1..4 { s += n }\ns"), Value::Int(6)); // 1+2+3
        assert_eq!(run_block("mut s = 0\nfor n in 0..100 { if n == 3 { break }; s += n }\ns"), Value::Int(3)); // 0+1+2
    }

    #[test]
    fn if_else_branches() {
        assert_eq!(run_block("mut x = 0\nif 1 < 2 { x = 10 } else { x = 20 }\nx"), Value::Int(10));
    }
```

- [ ] **Step 3: Rodar os testes**

Run: `cargo test -p alloy-vm interp::tests`
Expected: PASS (todos). Ajustar separadores (`;` vs newline) nas strings de teste se o `parse_stmts` exigir — a semântica do valor final é o que importa.

- [ ] **Step 4: Commit**

```bash
git add crates/alloy-vm/src/interp.rs
git commit -m "feat(alloy): control flow (if/while/loop/for, break/continue)"
```

---

### Task 6: Funções do usuário + chamadas + `println`

**Files:**
- Modify: `crates/alloy-vm/src/interp.rs`
- Test: `crates/alloy-vm/src/interp.rs` (novos testes)

**Interfaces:**
- Consumes: `copper_syntax::program::{parse_program, Program, Item, Param}`.
- Produces:
  - `Interpreter` ganha campo `funcs: HashMap<String, FuncDef>` onde `FuncDef { params: Vec<String>, body: Block }`.
  - `Interpreter::load_program(&mut self, prog: &Program)` — registra todas as `Item::Function` na tabela.
  - `Interpreter::run_program(&mut self, prog: &Program) -> Result<Value, RuntimeError>` — registra funções, executa os `Item::Stmt` de topo em ordem (escopo global), e se existir `func main`, chama-a ao fim.
  - `ExprKind::Call` resolvido em `eval_expr`: builtin `println`/`print` (imprime args separados por espaço, `println` com `\n`) ou função do usuário (cria escopo a partir do global, vincula params, executa corpo, retorna `Outcome::Return` value ou Unit).

- [ ] **Step 1: Adicionar tabela de funções e resolução de `Call`**

No topo do arquivo, adicionar imports e o struct auxiliar:

```rust
use copper_syntax::program::{Item, Program};
use std::collections::HashMap;

#[derive(Clone)]
struct FuncDef {
    params: Vec<String>,
    body: Block,
}
```

Trocar a definição de `Interpreter`:

```rust
#[derive(Default)]
pub struct Interpreter {
    funcs: HashMap<String, FuncDef>,
    globals: Option<Rc<RefCell<Env>>>,
}
```

Adicionar métodos:

```rust
    pub fn load_program(&mut self, prog: &Program) {
        for item in &prog.items {
            if let Item::Function { name, params, body, .. } = item {
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
        // statements de topo (Copper permite código solto → vira main implícita)
        for item in &prog.items {
            if let Item::Stmt(stmt) = item {
                if let Outcome::Return(v) = self.exec_stmt(stmt, &globals)? {
                    return Ok(v);
                }
            }
        }
        // se há `func main`, chama
        if self.funcs.contains_key("main") {
            return self.call_user("main", vec![], copper_syntax::ast::Span::default());
        }
        Ok(Value::Unit)
    }

    fn call_user(&mut self, name: &str, args: Vec<Value>, span: copper_syntax::ast::Span) -> Result<Value, RuntimeError> {
        let def = self.funcs.get(name).cloned().ok_or_else(|| {
            RuntimeError::new(format!("função `{name}` não definida"), span)
        })?;
        if def.params.len() != args.len() {
            return Err(RuntimeError::new(
                format!("`{name}` espera {} args, recebeu {}", def.params.len(), args.len()),
                span,
            ));
        }
        let base = self.globals.clone().unwrap_or_else(Env::new);
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
```

Adicionar o braço `Call` em `eval_expr` (antes do `_`):

```rust
            ExprKind::Call { callee, args, .. } => {
                let arg_vals: Vec<Value> = args
                    .iter()
                    .map(|a| self.eval_expr(a, env))
                    .collect::<Result<_, _>>()?;
                match &callee.kind {
                    ExprKind::Ident(name) if name == "println" || name == "print" => {
                        let line = arg_vals.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
                        if name == "println" {
                            println!("{line}");
                        } else {
                            print!("{line}");
                        }
                        Ok(Value::Unit)
                    }
                    ExprKind::Ident(name) => self.call_user(name, arg_vals, expr.span),
                    _ => Err(RuntimeError::new("alvo de chamada não suportado", callee.span)),
                }
            }
```

- [ ] **Step 2: Escrever os testes falhando**

```rust
    use copper_syntax::program::parse_program;

    fn run_prog(src: &str) -> Value {
        let prog = parse_program(src);
        assert!(prog.errors.is_empty(), "prog errs: {:?}", prog.errors);
        Interpreter::new().run_program(&prog).expect("erro de runtime")
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
```

Nota: `Interpreter::new()` agora vem do `#[derive(Default)]` — confirmar que `new()` retorna `Self::default()`. Se o `new()` da Task 1 retornava `Interpreter` unitário, trocar por:

```rust
impl Interpreter {
    pub fn new() -> Self {
        Self::default()
    }
```

- [ ] **Step 3: Rodar os testes**

Run: `cargo test -p alloy-vm interp::tests`
Expected: PASS (todos, incluindo os anteriores). O teste `run_prog` que termina numa expressão de topo (`r`, `fac(5)`) depende de `parse_program` emitir um `Item::Stmt(Stmt::Expr(..))` final; se ele exigir a expressão dentro de função, adaptar para `func int main() { ... return ... }` e ajustar a asserção via valor de retorno de `main`.

- [ ] **Step 4: Commit**

```bash
git add crates/alloy-vm/src/interp.rs
git commit -m "feat(alloy): user functions, calls, recursion and println"
```

---

### Task 7: Binário `alloy` com subcomando `run`

**Files:**
- Create: `crates/alloy-vm/src/bin/alloy.rs`
- Test: `crates/alloy-vm/tests/cli_run.rs`

**Interfaces:**
- Consumes: `alloy_vm::interp::Interpreter`, `copper_syntax::program::parse_program`.
- Produces: binário `alloy` com `alloy run <arquivo.crs>`. Em erro de parse ou runtime, imprime no stderr com o `Span` e sai com código 1.

- [ ] **Step 1: Escrever `crates/alloy-vm/src/bin/alloy.rs`**

```rust
//! CLI do Alloy: interpretador tree-walking de Copper.

use std::path::PathBuf;
use std::process::ExitCode;

use alloy_vm::interp::Interpreter;
use clap::{Parser, Subcommand};
use copper_syntax::program::parse_program;

#[derive(Parser)]
#[command(name = "alloy", about = "Interpretador Alloy para Copper")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Interpreta um arquivo .crs.
    Run { file: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run { file } => run(&file),
    }
}

fn run(file: &PathBuf) -> ExitCode {
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("alloy: não consegui ler {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    let prog = parse_program(&src);
    if !prog.errors.is_empty() {
        for err in &prog.errors {
            eprintln!(
                "alloy: erro de sintaxe @ {}..{}: {}",
                err.span.start, err.span.end, err.message
            );
        }
        return ExitCode::FAILURE;
    }
    match Interpreter::new().run_program(&prog) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!(
                "alloy: erro de runtime @ {}..{}: {}",
                e.span.start, e.span.end, e.message
            );
            ExitCode::FAILURE
        }
    }
}
```

- [ ] **Step 2: Escrever o teste de integração falhando**

`crates/alloy-vm/tests/cli_run.rs`:

```rust
use std::io::Write;
use std::process::Command;

/// Roda o binário `alloy run` sobre um fonte temporário e captura stdout.
fn alloy_run(src: &str) -> (String, bool) {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("alloy_test_{}.crs", std::process::id()));
    {
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(src.as_bytes()).unwrap();
    }
    let exe = env!("CARGO_BIN_EXE_alloy");
    let out = Command::new(exe).arg("run").arg(&path).output().unwrap();
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.success(),
    )
}

#[test]
fn hello_world_prints() {
    let (stdout, ok) = alloy_run("println(\"olá, alloy\")");
    assert!(ok, "alloy run falhou");
    assert_eq!(stdout.trim_end(), "olá, alloy");
}

#[test]
fn loop_and_function() {
    let src = "func int dobro(n: int) { return n * 2 }\nfor i in 1..4 { println(dobro(i)) }";
    let (stdout, ok) = alloy_run(src);
    assert!(ok);
    assert_eq!(stdout, "2\n4\n6\n");
}
```

- [ ] **Step 3: Rodar o teste**

Run: `cargo test -p alloy-vm --test cli_run`
Expected: PASS (2 tests). Se `parse_program` reportar erros para essas fontes, ajustar a sintaxe do fonte de teste (não o interpretador) até a AST aceitar — a invariante é só "o que a AST aceita, a VM executa".

- [ ] **Step 4: `cargo run` manual de fumaça**

Run: `echo 'println("ok")' > /tmp/smoke.crs && cargo run -p alloy-vm --bin alloy -- run /tmp/smoke.crs`
Expected: imprime `ok`, exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/alloy-vm/src/bin/alloy.rs crates/alloy-vm/tests/cli_run.rs
git commit -m "feat(alloy): alloy run CLI subcommand"
```

---

### Task 8: Exemplo executável + checagem de qualidade do workspace

**Files:**
- Create: `examples/copper/alloy-hello.crs`
- Test: comandos de verificação (fmt/clippy/test)

**Interfaces:**
- Consumes: tudo das tasks anteriores.
- Produces: um exemplo `.crs` que roda em `alloy run` e o crate passa fmt/clippy/test.

- [ ] **Step 1: Criar `examples/copper/alloy-hello.crs`**

```text
func int soma_ate(n: int) {
    mut total = 0
    for i in 1..n {
        total += i
    }
    return total
}

func main() {
    println("Alloy VM")
    mut s = soma_ate(5)
    println("soma 1..5 = ${s}")
}
```

- [ ] **Step 2: Rodar o exemplo**

Run: `cargo run -p alloy-vm --bin alloy -- run examples/copper/alloy-hello.crs`
Expected:
```
Alloy VM
soma 1..5 = 10
```
(Se a sintaxe `${s}` em `println` não parsear como `Call(println, [Str template])`, simplificar para `println(s)` e ajustar a saída esperada para `10`.)

- [ ] **Step 3: fmt + clippy + test do crate**

Run: `cargo fmt -p alloy-vm -- --check && cargo clippy -p alloy-vm -- -D warnings && cargo test -p alloy-vm`
Expected: tudo PASS, zero warnings. Corrigir qualquer lint inline.

- [ ] **Step 4: Verificar que o workspace inteiro ainda compila**

Run: `cargo check --workspace`
Expected: PASS — confirma que adicionar `alloy-vm` não quebrou `cforge`/`mui-*`.

- [ ] **Step 5: Commit**

```bash
git add examples/copper/alloy-hello.crs
git commit -m "docs(alloy): runnable alloy-hello.crs example"
```

---

## Notas de execução

- **Risco recorrente:** a forma exata que `parse_stmts`/`parse_program` produz para certas sintaxes (separadores `;` vs `\n`, se a última expressão vira `tail` ou `Item::Stmt`). Sempre que um teste falhar no *parse* (não na avaliação), ajuste o **fonte de teste** para algo que a AST aceita — nunca modifique `copper-syntax` neste plano. A invariante do MVP é "o que a AST aceita, a VM executa corretamente".
- **`ExprKind::Raw`:** se aparecer em qualquer teste, significa que o subset do Pratt parser não formou aquela construção — está fora do escopo do MVP e o interpretador deve devolver `RuntimeError` "construção ainda não suportada pela VM" (o braço `_` já cobre isso).
- **Próximos planos:** Fase 3 (struct/class/enum/impl/match/closures na VM), depois interop e `alloy build`. Cada um é um plano próprio.
