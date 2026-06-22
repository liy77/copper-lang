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
enum Sink {
    #[default]
    Stdout,
    Buffer(Rc<RefCell<String>>),
}

#[derive(Default)]
pub struct Interpreter {
    funcs: HashMap<String, FuncDef>,
    /// tipo -> (nome do método/função associada -> def). Métodos têm `self`
    /// como primeiro parâmetro; funções associadas (ex.: `Rect::new`) não.
    methods: HashMap<String, HashMap<String, FuncDef>>,
    /// símbolo importado -> módulo de origem (ex.: "input" -> "cstd").
    imports: HashMap<String, String>,
    /// classes que têm construtor (`Class::new` cria a instância e roda o corpo).
    constructors: std::collections::HashSet<String>,
    globals: Option<Rc<RefCell<Env>>>,
    out: Sink,
}

impl Interpreter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build an interpreter that writes program output into `buf` instead of
    /// stdout. The caller keeps the `Rc` to read captured output.
    pub fn with_output(buf: Rc<RefCell<String>>) -> Self {
        Self {
            out: Sink::Buffer(buf),
            ..Self::default()
        }
    }

    fn emit(&mut self, text: &str, newline: bool) {
        match &self.out {
            Sink::Stdout => {
                if newline {
                    println!("{text}");
                } else {
                    print!("{text}");
                }
            }
            Sink::Buffer(buf) => {
                let mut b = buf.borrow_mut();
                b.push_str(text);
                if newline {
                    b.push('\n');
                }
            }
        }
    }

    pub fn load_program(&mut self, prog: &Program) {
        use copper_syntax::program::ClassMember;
        for item in &prog.items {
            match item {
                Item::Function {
                    name, params, body, ..
                } => {
                    self.funcs.insert(
                        name.clone(),
                        FuncDef {
                            params: params.iter().map(|p| p.name.clone()).collect(),
                            body: body.clone(),
                        },
                    );
                }
                Item::Impl { target, items, .. } => {
                    let table = self.methods.entry(target.clone()).or_default();
                    for it in items {
                        if let Item::Function {
                            name, params, body, ..
                        } = it
                        {
                            table.insert(
                                name.clone(),
                                FuncDef {
                                    params: params.iter().map(|p| p.name.clone()).collect(),
                                    body: body.clone(),
                                },
                            );
                        }
                    }
                }
                Item::Class { name, members, .. } => {
                    let table = self.methods.entry(name.clone()).or_default();
                    for m in members {
                        match m {
                            ClassMember::Method {
                                name: mname,
                                params,
                                body,
                                ..
                            } => {
                                table.insert(
                                    mname.clone(),
                                    FuncDef {
                                        params: params.iter().map(|p| p.name.clone()).collect(),
                                        body: body.clone(),
                                    },
                                );
                            }
                            ClassMember::Constructor { params, body, .. } => {
                                // `Class::new(...)` constrói uma instância.
                                self.constructors.insert(name.clone());
                                table.insert(
                                    "new".into(),
                                    FuncDef {
                                        params: params.iter().map(|p| p.name.clone()).collect(),
                                        body: body.clone(),
                                    },
                                );
                            }
                            ClassMember::Field(_) => {}
                        }
                    }
                }
                Item::Struct { name, .. } => {
                    self.methods.entry(name.clone()).or_default();
                }
                Item::Import { kind, path, .. } => {
                    use copper_syntax::program::ImportKind;
                    if let ImportKind::Items(names) = kind {
                        for n in names {
                            self.imports.insert(n.clone(), path.clone());
                        }
                    }
                }
                _ => {}
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
            ExprKind::Ident(name) => {
                if let Some(v) = env.borrow().get(name) {
                    return Ok(v);
                }
                // Construtores nulares de enum embutidos.
                match name.as_str() {
                    "None" => Ok(Value::none()),
                    _ => Err(RuntimeError::new(
                        format!("variável `{name}` não definida"),
                        expr.span,
                    )),
                }
            }
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
                let rhs = self.eval_expr(value, env)?;
                // Desembrulha `*x`/`&x` (ponteiros são identidade aqui).
                let mut place = &**target;
                while let ExprKind::Unary {
                    op: UnOp::Deref | UnOp::Ref | UnOp::RefMut,
                    expr: inner,
                } = &place.kind
                {
                    place = inner;
                }
                let compound = |me: &mut Self, cur: Value, span| -> Result<Value, RuntimeError> {
                    let binop = match op {
                        AssignOp::Plain => return Ok(rhs.clone()),
                        AssignOp::Add => BinOp::Add,
                        AssignOp::Sub => BinOp::Sub,
                        AssignOp::Mul => BinOp::Mul,
                        AssignOp::Div => BinOp::Div,
                        AssignOp::Rem => BinOp::Rem,
                        AssignOp::BitAnd => BinOp::BitAnd,
                        AssignOp::BitOr => BinOp::BitOr,
                        AssignOp::BitXor => BinOp::BitXor,
                    };
                    me.eval_binary(binop, cur, rhs.clone(), span)
                };
                match &place.kind {
                    ExprKind::Ident(name) => {
                        let cur = env.borrow().get(name).unwrap_or(Value::Unit);
                        let nv = compound(self, cur, expr.span)?;
                        if !env.borrow_mut().set(name, nv) {
                            // primeira atribuição a um nome livre → define
                            env.borrow_mut().define(name.clone(), rhs);
                        }
                        Ok(Value::Unit)
                    }
                    // `obj.campo = v` / `self.campo = v`.
                    ExprKind::Member { base, field, .. } => {
                        let recv = self.eval_expr(base, env)?;
                        if let Value::Struct { fields, .. } = recv {
                            let cur = fields.borrow().get(field).cloned().unwrap_or(Value::Unit);
                            let nv = compound(self, cur, expr.span)?;
                            fields.borrow_mut().insert(field.clone(), nv);
                            Ok(Value::Unit)
                        } else {
                            Err(RuntimeError::new(
                                "atribuição de campo em valor não-struct",
                                target.span,
                            ))
                        }
                    }
                    // `vec[i] = v`.
                    ExprKind::Index { base, index } => {
                        let recv = self.eval_expr(base, env)?;
                        let idx = self.eval_expr(index, env)?;
                        if let (Value::Vec(items), Value::Int(i)) = (&recv, &idx) {
                            let i = *i as usize;
                            let cur = items.borrow().get(i).cloned().unwrap_or(Value::Unit);
                            let nv = compound(self, cur, expr.span)?;
                            if i < items.borrow().len() {
                                items.borrow_mut()[i] = nv;
                                return Ok(Value::Unit);
                            }
                        }
                        Err(RuntimeError::new(
                            "índice de atribuição inválido",
                            target.span,
                        ))
                    }
                    _ => Err(RuntimeError::new(
                        "alvo de atribuição não suportado",
                        target.span,
                    )),
                }
            }
            ExprKind::Array(items) => {
                let vals = items
                    .iter()
                    .map(|e| self.eval_expr(e, env))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Value::Vec(Rc::new(RefCell::new(vals))))
            }
            ExprKind::Index { base, index } => {
                let b = self.eval_expr(base, env)?;
                let i = self.eval_expr(index, env)?;
                match i {
                    Value::Str(k) => self.index_str(b, &k, expr.span),
                    other => self.index_value(b, other, expr.span),
                }
            }
            ExprKind::Member {
                base,
                field,
                optional,
            } => {
                let b = self.eval_expr(base, env)?;
                self.member_value(b, field, *optional, expr.span)
            }
            ExprKind::StructLit { name, fields, .. } => {
                let mut map = HashMap::new();
                for (fname, fexpr) in fields {
                    map.insert(fname.clone(), self.eval_expr(fexpr, env)?);
                }
                Ok(Value::Struct {
                    name: name.clone(),
                    fields: Rc::new(RefCell::new(map)),
                })
            }
            // `as` cast: no interpretador tratamos como identidade (sem checagem
            // estática de tipos), exceto conversões numéricas óbvias.
            ExprKind::Cast { expr: inner, ty } => {
                let v = self.eval_expr(inner, env)?;
                Ok(cast_value(v, ty))
            }
            ExprKind::Match { scrutinee, arms } => {
                let val = self.eval_expr(scrutinee, env)?;
                for arm in arms {
                    let scope = Env::child(env);
                    if !self.match_pattern(&arm.pattern, &val, &scope) {
                        continue;
                    }
                    if let Some(guard) = &arm.guard {
                        match self.eval_expr(guard, &scope)?.as_bool() {
                            Some(true) => {}
                            _ => continue,
                        }
                    }
                    return self.eval_expr(&arm.body, &scope);
                }
                Err(RuntimeError::new("nenhum braço do match casou", expr.span))
            }
            ExprKind::If { cond, then, els } => {
                let c = self.eval_expr(cond, env)?;
                match c.as_bool() {
                    Some(true) => self.eval_expr(then, env),
                    Some(false) => match els {
                        Some(e) => self.eval_expr(e, env),
                        None => Ok(Value::Unit),
                    },
                    None => Err(RuntimeError::new("condição de `if` não é bool", cond.span)),
                }
            }
            ExprKind::Closure { params, body } => {
                Ok(Value::Closure(Rc::new(crate::value::ClosureData {
                    params: params.clone(),
                    body: (**body).clone(),
                    env: Rc::clone(env),
                })))
            }
            ExprKind::Block(block) => self.eval_block(block, env),
            ExprKind::Try { expr: inner } => {
                // `expr?`: Ok(v)/Some(v) → v; Err/None propaga como erro de
                // runtime (modelo simplificado, sem early-return de função).
                let v = self.eval_expr(inner, env)?;
                match &v {
                    Value::Enum {
                        variant, payload, ..
                    } if matches!(variant.as_str(), "Ok" | "Some") => {
                        Ok(payload.first().cloned().unwrap_or(Value::Unit))
                    }
                    Value::Enum { variant, .. } => {
                        Err(RuntimeError::new(format!("`?` em `{variant}`"), expr.span))
                    }
                    _ => Ok(v),
                }
            }
            ExprKind::Call { callee, args, .. } => {
                let arg_vals: Vec<Value> = args
                    .iter()
                    .map(|a| self.eval_expr(a, env))
                    .collect::<Result<_, _>>()?;
                match &callee.kind {
                    // `vec![..]` chega como Call{callee: Ident("vec"), args:[Array]}.
                    ExprKind::Ident(name) if name == "vec" => {
                        let items = if arg_vals.len() == 1 {
                            match &arg_vals[0] {
                                Value::Vec(v) => v.borrow().clone(),
                                _ => arg_vals.clone(),
                            }
                        } else {
                            arg_vals.clone()
                        };
                        Ok(Value::Vec(Rc::new(RefCell::new(items))))
                    }
                    ExprKind::Ident(name) if name == "println" || name == "print" => {
                        let line = render_print(&arg_vals);
                        let newline = name == "println";
                        self.emit(&line, newline);
                        Ok(Value::Unit)
                    }
                    // Construtores de Option/Result.
                    ExprKind::Ident(name) if name == "Some" => Ok(Value::some(
                        arg_vals.into_iter().next().unwrap_or(Value::Unit),
                    )),
                    ExprKind::Ident(name) if name == "Ok" => Ok(Value::ok(
                        arg_vals.into_iter().next().unwrap_or(Value::Unit),
                    )),
                    ExprKind::Ident(name) if name == "Err" => Ok(Value::err(
                        arg_vals.into_iter().next().unwrap_or(Value::Unit),
                    )),
                    ExprKind::Ident(name) => {
                        let name = name.clone();
                        // Função de stdlib importada?
                        if let Some(module) = self.imports.get(&name).cloned() {
                            if let Some(res) =
                                crate::stdlib::dispatch(&module, &name, &arg_vals, expr.span)
                            {
                                return res;
                            }
                            if let Some(res) =
                                crate::stdlib_ext::dispatch(&module, &name, &arg_vals, expr.span)
                            {
                                return res;
                            }
                        }
                        self.call_user(&name, arg_vals, expr.span)
                    }
                    // Chamada de método: `recv.metodo(args)`.
                    ExprKind::Member {
                        base,
                        field,
                        optional,
                    } => {
                        let recv = self.eval_expr(base, env)?;
                        // `recv?.metodo()` em None → None.
                        if *optional {
                            if let Value::Enum { variant, .. } = &recv {
                                if variant == "None" {
                                    return Ok(Value::none());
                                }
                            }
                        }
                        self.call_method(recv, field, arg_vals, expr.span)
                    }
                    // Função associada: `Tipo::func(args)` ou variante de enum.
                    ExprKind::Path { segments } => self.call_path(segments, arg_vals, expr.span),
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

    /// `base[index]` — indexação de `Vec` (por int) e tupla (por int).
    fn index_value(
        &mut self,
        base: Value,
        index: Value,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        let idx = match index {
            Value::Int(n) if n >= 0 => n as usize,
            other => {
                return Err(RuntimeError::new(
                    format!(
                        "índice deve ser int não-negativo, achou {}",
                        other.type_name()
                    ),
                    span,
                ))
            }
        };
        match base {
            Value::Vec(items) => {
                items.borrow().get(idx).cloned().ok_or_else(|| {
                    RuntimeError::new(format!("índice {idx} fora dos limites"), span)
                })
            }
            Value::Tuple(items) => items
                .get(idx)
                .cloned()
                .ok_or_else(|| RuntimeError::new(format!("índice {idx} fora da tupla"), span)),
            other => Err(RuntimeError::new(
                format!("não dá para indexar {}", other.type_name()),
                span,
            )),
        }
    }

    /// `base[key]` por string — para objetos JSON (`Value::Struct`).
    fn index_str(
        &mut self,
        base: Value,
        key: &str,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        match base {
            Value::Struct { fields, .. } => {
                Ok(fields.borrow().get(key).cloned().unwrap_or(Value::Unit))
            }
            other => Err(RuntimeError::new(
                format!("não dá para indexar {} por string", other.type_name()),
                span,
            )),
        }
    }

    /// `base.field` (e `base?.field`). Cobre campo de struct, `.0`/`.1` de
    /// tupla, e propagação de `None` no `?.`.
    fn member_value(
        &mut self,
        base: Value,
        field: &str,
        optional: bool,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        // `?.` em `None` → continua `None`.
        if optional {
            if let Value::Enum {
                variant, payload, ..
            } = &base
            {
                if variant == "None" {
                    return Ok(Value::none());
                }
                if variant == "Some" {
                    let inner = payload.first().cloned().unwrap_or(Value::Unit);
                    return self.member_value(inner, field, false, span);
                }
            }
        }
        match base {
            Value::Struct { fields, name } => {
                fields.borrow().get(field).cloned().ok_or_else(|| {
                    RuntimeError::new(format!("`{name}` não tem o campo `{field}`"), span)
                })
            }
            Value::Tuple(items) => field
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i).cloned())
                .ok_or_else(|| {
                    RuntimeError::new(format!("tupla não tem o elemento `.{field}`"), span)
                }),
            other => Err(RuntimeError::new(
                format!("não dá para acessar `.{field}` em {}", other.type_name()),
                span,
            )),
        }
    }

    /// Tenta casar `pat` contra `val`, vinculando bindings em `scope`.
    /// Retorna `true` se casou.
    fn match_pattern(&self, pat: &Pattern, val: &Value, scope: &Rc<RefCell<Env>>) -> bool {
        match pat {
            Pattern::Wildcard => true,
            Pattern::Ident(name) => {
                // Variante nulária (ex.: `None`) casa por igualdade de variante;
                // senão é um binding que casa com qualquer valor.
                if let Value::Enum {
                    variant, payload, ..
                } = val
                {
                    if variant == name && payload.is_empty() {
                        return true;
                    }
                }
                scope.borrow_mut().define(name.clone(), val.clone());
                true
            }
            Pattern::Literal(lit) => pattern_literal(lit).as_ref() == Some(val),
            Pattern::TupleStruct { name, elems } => {
                if let Value::Enum {
                    variant, payload, ..
                } = val
                {
                    if variant == name && payload.len() >= elems.len() {
                        return elems
                            .iter()
                            .zip(payload.iter())
                            .all(|(p, v)| self.match_pattern(p, v, scope));
                    }
                }
                false
            }
            Pattern::Or(pats) => pats.iter().any(|p| self.match_pattern(p, val, scope)),
        }
    }

    /// Invoca um `FuncDef` (função, método ou associada). Se `receiver` for
    /// `Some`, ele é vinculado ao primeiro parâmetro `self`; os demais
    /// parâmetros recebem `args` em ordem.
    fn invoke(
        &mut self,
        def: &FuncDef,
        receiver: Option<Value>,
        args: Vec<Value>,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        let base = self.globals.clone().unwrap_or_default();
        let scope = Env::child(&base);
        let mut params = def.params.iter();
        if let Some(recv) = receiver {
            // Pula o parâmetro `self` (se declarado) e o vincula.
            if def.params.first().map(|p| p.as_str()) == Some("self") {
                params.next();
            }
            scope.borrow_mut().define("self".to_string(), recv);
        }
        let rest: Vec<&String> = params.collect();
        if rest.len() != args.len() {
            return Err(RuntimeError::new(
                format!("função espera {} args, recebeu {}", rest.len(), args.len()),
                span,
            ));
        }
        for (p, a) in rest.into_iter().zip(args) {
            scope.borrow_mut().define(p.clone(), a);
        }
        match self.run_block_in(&def.body, &scope)? {
            Outcome::Return(v) | Outcome::Normal(v) => Ok(v),
            Outcome::Break | Outcome::Continue => {
                Err(RuntimeError::new("break/continue fora de loop", span))
            }
        }
    }

    /// Aplica uma closure a argumentos.
    fn call_closure(
        &mut self,
        cl: &crate::value::ClosureData,
        args: Vec<Value>,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        let scope = Env::child(&cl.env);
        for (p, a) in cl.params.iter().zip(args) {
            scope.borrow_mut().define(p.clone(), a);
        }
        let _ = span;
        self.eval_expr(&cl.body, &scope)
    }

    /// `recv.metodo(args)` — tenta métodos built-in, depois métodos de usuário.
    fn call_method(
        &mut self,
        recv: Value,
        name: &str,
        args: Vec<Value>,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        // Adaptadores de iterador com closure (precisam do interpretador).
        if let Value::Vec(items) = &recv {
            if let Some(Value::Closure(cl)) = args.first() {
                let cl = Rc::clone(cl);
                let src = items.borrow().clone();
                match name {
                    "map" => {
                        let mut out = Vec::with_capacity(src.len());
                        for v in src {
                            out.push(self.call_closure(&cl, vec![v], span)?);
                        }
                        return Ok(Value::Vec(Rc::new(RefCell::new(out))));
                    }
                    "filter" => {
                        let mut out = Vec::new();
                        for v in src {
                            if self.call_closure(&cl, vec![v.clone()], span)?.as_bool()
                                == Some(true)
                            {
                                out.push(v);
                            }
                        }
                        return Ok(Value::Vec(Rc::new(RefCell::new(out))));
                    }
                    "for_each" => {
                        for v in src {
                            self.call_closure(&cl, vec![v], span)?;
                        }
                        return Ok(Value::Unit);
                    }
                    "any" => {
                        for v in src {
                            if self.call_closure(&cl, vec![v], span)?.as_bool() == Some(true) {
                                return Ok(Value::Bool(true));
                            }
                        }
                        return Ok(Value::Bool(false));
                    }
                    "all" => {
                        for v in src {
                            if self.call_closure(&cl, vec![v], span)?.as_bool() != Some(true) {
                                return Ok(Value::Bool(false));
                            }
                        }
                        return Ok(Value::Bool(true));
                    }
                    _ => {}
                }
            }
        }
        // Métodos do `Response` do módulo http.
        if let Value::Struct { name: ty, fields } = &recv {
            if ty == "Response" {
                let field = |k: &str| fields.borrow().get(k).cloned().unwrap_or(Value::Unit);
                match name {
                    "is_ok" => return Ok(field("ok")),
                    "status" => return Ok(field("status")),
                    "text" | "body" => return Ok(field("body")),
                    "json" => {
                        let body = field("body").to_string();
                        let j: serde_json::Value =
                            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                        return Ok(crate::stdlib_ext::json_to_value(&j));
                    }
                    _ => {}
                }
            }
        }
        if let Some(res) = builtin_method(&recv, name, &args, span) {
            return res;
        }
        if let Value::Struct { name: ty, .. } = &recv {
            if let Some(def) = self.methods.get(ty).and_then(|m| m.get(name)).cloned() {
                return self.invoke(&def, Some(recv), args, span);
            }
        }
        Err(RuntimeError::new(
            format!("método `{name}` não encontrado em {}", recv.type_name()),
            span,
        ))
    }

    /// `Tipo::func(args)` — função associada, ou construção de variante de enum.
    fn call_path(
        &mut self,
        segments: &[String],
        args: Vec<Value>,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        if segments.len() == 2 {
            let (ty, name) = (&segments[0], &segments[1]);
            // Construtor de classe: cria a instância, roda o corpo (que faz
            // `self.campo = ...`) e devolve a instância.
            if name == "new" && self.constructors.contains(ty) {
                if let Some(def) = self.methods.get(ty).and_then(|m| m.get("new")).cloned() {
                    let inst = Value::Struct {
                        name: ty.clone(),
                        fields: Rc::new(RefCell::new(HashMap::new())),
                    };
                    self.invoke(&def, Some(inst.clone()), args, span)?;
                    return Ok(inst);
                }
            }
            if let Some(def) = self.methods.get(ty).and_then(|m| m.get(name)).cloned() {
                return self.invoke(&def, None, args, span);
            }
            // Não é função associada conhecida → variante de enum.
            return Ok(Value::Enum {
                ty: ty.clone(),
                variant: name.clone(),
                payload: args,
            });
        }
        Err(RuntimeError::new(
            format!("caminho `{}` não suportado", segments.join("::")),
            span,
        ))
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
                if let Some(pat) = let_pattern {
                    let v = self.eval_expr(cond, env)?;
                    let scope = Env::child(env);
                    if self.match_pattern(pat, &v, &scope) {
                        return self.run_block_in(then, &scope);
                    }
                    return match els {
                        Some(s) => self.exec_stmt(s, env),
                        None => Ok(Outcome::Normal(Value::Unit)),
                    };
                }
                let _ = span;
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
                let _ = span;
                if let Some(pat) = let_pattern {
                    loop {
                        let v = self.eval_expr(cond, env)?;
                        let scope = Env::child(env);
                        if self.match_pattern(pat, &v, &scope) {
                            match self.run_block_in(body, &scope)? {
                                Outcome::Break => break,
                                Outcome::Return(v) => return Ok(Outcome::Return(v)),
                                _ => {}
                            }
                        } else {
                            break;
                        }
                    }
                    return Ok(Outcome::Normal(Value::Unit));
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
                    match i.checked_add(1) {
                        Some(n) => i = n,
                        None => break,
                    }
                }
                Ok(Outcome::Normal(Value::Unit))
            }
            Stmt::BlockStmt(b) => self.run_block(b, env),
            // `unsafe { ... }` — sem semântica especial no interpretador.
            Stmt::Unsafe { body, .. } => self.run_block(body, env),
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
            // Referência/deref: no interpretador são identidade (não há um
            // modelo de ponteiros real; `unsafe`/raw-ptr executam sem aliasing).
            (UnOp::Ref, v) | (UnOp::RefMut, v) | (UnOp::Deref, v) => Ok(v),
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

/// Métodos built-in (estilo Rust) sobre os valores. Retorna `None` quando o
/// método não é built-in (aí o chamador tenta métodos de usuário). Nunca
/// panica: erros viram `RuntimeError`.
fn builtin_method(
    recv: &Value,
    name: &str,
    args: &[Value],
    span: copper_syntax::ast::Span,
) -> Option<Result<Value, RuntimeError>> {
    // Métodos válidos em qualquer valor.
    match name {
        "to_string" => return Some(Ok(Value::Str(recv.to_string()))),
        "clone" => return Some(Ok(recv.clone())),
        _ => {}
    }
    match recv {
        Value::Str(s) => match name {
            "len" => Some(Ok(Value::Int(s.chars().count() as i64))),
            "is_empty" => Some(Ok(Value::Bool(s.is_empty()))),
            "to_uppercase" => Some(Ok(Value::Str(s.to_uppercase()))),
            "to_lowercase" => Some(Ok(Value::Str(s.to_lowercase()))),
            "trim" => Some(Ok(Value::Str(s.trim().to_string()))),
            "chars" => Some(Ok(Value::Vec(Rc::new(RefCell::new(
                s.chars().map(|c| Value::Str(c.to_string())).collect(),
            ))))),
            "count" => Some(Ok(Value::Int(s.chars().count() as i64))),
            "parse" => Some(Ok(match s.trim().parse::<i64>() {
                Ok(n) => Value::ok(Value::Int(n)),
                Err(_) => match s.trim().parse::<f64>() {
                    Ok(x) => Value::ok(Value::Float(x)),
                    Err(_) => Value::err(Value::Str(format!("não consegui parsear `{s}`"))),
                },
            })),
            "contains" => Some(Ok(Value::Bool(match args.first() {
                Some(Value::Str(p)) => s.contains(p.as_str()),
                _ => false,
            }))),
            _ => None,
        },
        Value::Vec(items) => match name {
            "len" | "count" => Some(Ok(Value::Int(items.borrow().len() as i64))),
            "is_empty" => Some(Ok(Value::Bool(items.borrow().is_empty()))),
            // `into_iter` devolve um cursor independente (clone) para que
            // `.next()` consuma sem afetar o vec original.
            "into_iter" => Some(Ok(Value::Vec(Rc::new(RefCell::new(
                items.borrow().clone(),
            ))))),
            // Demais adaptadores: modelo eager, retornam o próprio vec.
            "iter" | "copied" | "cloned" | "collect" => Some(Ok(Value::Vec(Rc::clone(items)))),
            // Consome o primeiro elemento (drena a frente do cursor).
            "next" => {
                let mut b = items.borrow_mut();
                if b.is_empty() {
                    Some(Ok(Value::none()))
                } else {
                    Some(Ok(Value::some(b.remove(0))))
                }
            }
            "rev" => {
                let mut v = items.borrow().clone();
                v.reverse();
                Some(Ok(Value::Vec(Rc::new(RefCell::new(v)))))
            }
            "sum" => {
                let b = items.borrow();
                if b.iter().all(|v| matches!(v, Value::Int(_))) {
                    let s: i64 = b
                        .iter()
                        .map(|v| if let Value::Int(n) = v { *n } else { 0 })
                        .sum();
                    Some(Ok(Value::Int(s)))
                } else {
                    let s: f64 = b
                        .iter()
                        .map(|v| match v {
                            Value::Int(n) => *n as f64,
                            Value::Float(x) => *x,
                            _ => 0.0,
                        })
                        .sum();
                    Some(Ok(Value::Float(s)))
                }
            }
            "push" => {
                if let Some(a) = args.first() {
                    items.borrow_mut().push(a.clone());
                }
                Some(Ok(Value::Unit))
            }
            "first" => Some(Ok(items
                .borrow()
                .first()
                .cloned()
                .map(Value::some)
                .unwrap_or_else(Value::none))),
            "last" => Some(Ok(items
                .borrow()
                .last()
                .cloned()
                .map(Value::some)
                .unwrap_or_else(Value::none))),
            _ => None,
        },
        Value::Int(n) => match name {
            "abs" => Some(Ok(Value::Int(n.abs()))),
            "to_float" => Some(Ok(Value::Float(*n as f64))),
            _ => None,
        },
        Value::Float(x) => match name {
            "abs" => Some(Ok(Value::Float(x.abs()))),
            "recip" => Some(Ok(Value::Float(x.recip()))),
            "sqrt" => Some(Ok(Value::Float(x.sqrt()))),
            "round" => Some(Ok(Value::Float(x.round()))),
            "floor" => Some(Ok(Value::Float(x.floor()))),
            "ceil" => Some(Ok(Value::Float(x.ceil()))),
            _ => None,
        },
        Value::Enum {
            ty,
            variant,
            payload,
        } => match name {
            "is_some" => Some(Ok(Value::Bool(variant == "Some"))),
            "is_none" => Some(Ok(Value::Bool(variant == "None"))),
            "is_ok" => Some(Ok(Value::Bool(variant == "Ok"))),
            "is_err" => Some(Ok(Value::Bool(variant == "Err"))),
            "unwrap" | "expect" => {
                if matches!(variant.as_str(), "Some" | "Ok") {
                    Some(Ok(payload.first().cloned().unwrap_or(Value::Unit)))
                } else {
                    Some(Err(RuntimeError::new(
                        format!("unwrap em `{variant}` de {ty}"),
                        span,
                    )))
                }
            }
            "unwrap_or" => {
                if matches!(variant.as_str(), "Some" | "Ok") {
                    Some(Ok(payload.first().cloned().unwrap_or(Value::Unit)))
                } else {
                    Some(Ok(args.first().cloned().unwrap_or(Value::Unit)))
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// Converte um literal de pattern em `Value` para comparação.
fn pattern_literal(lit: &Literal) -> Option<Value> {
    match lit {
        Literal::Int(n) => Some(Value::Int(*n)),
        Literal::Float(x) => Some(Value::Float(*x)),
        Literal::Bool(b) => Some(Value::Bool(*b)),
        Literal::Str(tpl) => {
            let mut s = String::new();
            for part in &tpl.parts {
                if let StrPart::Lit(t) = part {
                    s.push_str(t);
                } else {
                    return None; // interpolação em pattern não é suportada
                }
            }
            Some(Value::Str(s))
        }
    }
}

/// `expr as Tipo` — conversões numéricas; o resto é identidade.
fn cast_value(v: Value, ty: &copper_syntax::expr::Type) -> Value {
    use copper_syntax::expr::Type;
    match (ty, &v) {
        (Type::Int, Value::Float(x)) => Value::Int(*x as i64),
        (Type::Int, Value::Bool(b)) => Value::Int(*b as i64),
        (Type::Float, Value::Int(n)) => Value::Float(*n as f64),
        (Type::Named(name, _), Value::Float(x))
            if matches!(
                name.as_str(),
                "i8" | "i16" | "i32" | "i64" | "u8" | "u32" | "u64" | "usize" | "isize"
            ) =>
        {
            Value::Int(*x as i64)
        }
        (Type::Named(name, _), Value::Int(n)) if matches!(name.as_str(), "f32" | "f64") => {
            Value::Float(*n as f64)
        }
        _ => v,
    }
}

/// Renderiza os argumentos de `println!`/`print!`. Se o primeiro argumento for
/// uma string com placeholders `{...}` e houver mais argumentos, faz
/// substituição posicional estilo `format!` (o spec interno — `{}`, `{:?}`,
/// `{:.2}` — é ignorado, usa-se `Display`). Senão, junta tudo por espaço.
fn render_print(args: &[Value]) -> String {
    if let Some(Value::Str(fmt)) = args.first() {
        if args.len() > 1 && fmt.contains('{') {
            return format_with(fmt, &args[1..]);
        }
    }
    args.iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Substitui cada `{...}` por `args[i].to_string()`, na ordem. `{{`/`}}` são
/// chaves literais. Placeholders extras sem argumento viram vazio.
fn format_with(fmt: &str, args: &[Value]) -> String {
    let mut out = String::new();
    let mut next = 0usize;
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                // Consome até o '}' de fechamento (ignora o spec de formato).
                for n in chars.by_ref() {
                    if n == '}' {
                        break;
                    }
                }
                if let Some(v) = args.get(next) {
                    out.push_str(&v.to_string());
                }
                next += 1;
            }
            other => out.push(other),
        }
    }
    out
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
    fn arrays_index_and_vec_macro() {
        assert_eq!(eval("[10, 20, 30][1]"), Value::Int(20));
        assert_eq!(eval("vec![1, 2, 3][2]"), Value::Int(3));
        assert_eq!(eval("[1, 2, 3]").to_string(), "[1, 2, 3]");
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

    #[test]
    fn if_else_takes_else_branch() {
        // condition false → else block must run (was crashing before BlockStmt arm was added)
        assert_eq!(
            run_block("mut x = 0\nif 1 > 2 { x += 10 } else { x += 20 }\nx"),
            Value::Int(20)
        );
    }

    #[test]
    fn with_output_captures_println_and_print() {
        let buf = Rc::new(RefCell::new(String::new()));
        let prog = parse_program("println(\"a\")\nprint(\"b\")");
        assert!(prog.errors.is_empty(), "prog errs: {:?}", prog.errors);
        Interpreter::with_output(Rc::clone(&buf))
            .run_program(&prog)
            .expect("erro de runtime");
        assert_eq!(buf.borrow().as_str(), "a\nb");
    }
}
