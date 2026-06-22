//! Tree-walking interpreter.

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

/// Internal result of executing a sequence of statements.
enum Outcome {
    /// Continued normally, with the accumulated block value.
    Normal(Value),
    Return(Value),
    Break,
    Continue,
}

#[derive(Clone)]
struct FuncDef {
    params: Vec<String>,
    body: Block,
    /// Declared return type (`func <ret> name(...)`), for return-type checking.
    return_type: Option<copper_syntax::expr::Type>,
    /// Name, for clearer error messages.
    name: String,
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
    /// type -> (method/associated-function name -> def). Methods have `self`
    /// as the first parameter; associated functions (e.g. `Rect::new`) do not.
    methods: HashMap<String, HashMap<String, FuncDef>>,
    /// imported symbol -> source module (e.g. "input" -> "cstd").
    imports: HashMap<String, String>,
    /// classes that have a constructor (`Class::new` creates the instance and runs the body).
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
                    name,
                    params,
                    body,
                    return_type,
                    ..
                } => {
                    self.funcs.insert(
                        name.clone(),
                        FuncDef {
                            params: params.iter().map(|p| p.name.clone()).collect(),
                            body: body.clone(),
                            return_type: return_type.clone(),
                            name: name.clone(),
                        },
                    );
                }
                Item::Impl { target, items, .. } => {
                    let table = self.methods.entry(target.clone()).or_default();
                    for it in items {
                        if let Item::Function {
                            name,
                            params,
                            body,
                            return_type,
                            ..
                        } = it
                        {
                            table.insert(
                                name.clone(),
                                FuncDef {
                                    params: params.iter().map(|p| p.name.clone()).collect(),
                                    body: body.clone(),
                                    return_type: return_type.clone(),
                                    name: name.clone(),
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
                                return_type,
                                ..
                            } => {
                                table.insert(
                                    mname.clone(),
                                    FuncDef {
                                        params: params.iter().map(|p| p.name.clone()).collect(),
                                        body: body.clone(),
                                        return_type: return_type.clone(),
                                        name: mname.clone(),
                                    },
                                );
                            }
                            ClassMember::Constructor { params, body, .. } => {
                                // `Class::new(...)` constructs an instance.
                                self.constructors.insert(name.clone());
                                table.insert(
                                    "new".into(),
                                    FuncDef {
                                        params: params.iter().map(|p| p.name.clone()).collect(),
                                        body: body.clone(),
                                        return_type: None,
                                        name: format!("{name}::new"),
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
            .ok_or_else(|| RuntimeError::new(format!("function `{name}` not defined"), span))?;
        if def.params.len() != args.len() {
            return Err(RuntimeError::new(
                format!(
                    "`{name}` expects {} args, got {}",
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
        let ret = match self.run_block_in(&def.body, &scope)? {
            Outcome::Return(v) | Outcome::Normal(v) => v,
            Outcome::Break | Outcome::Continue => {
                return Err(RuntimeError::new("break/continue outside of loop", span))
            }
        };
        check_return_type(&def, &ret, span)?;
        Ok(ret)
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
                // Nullary enum constructors built in.
                match name.as_str() {
                    "None" => Ok(Value::none()),
                    _ => Err(RuntimeError::new(
                        format!("variable `{name}` not defined"),
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
                        format!("ternary condition is not bool (got {})", c.type_name()),
                        cond.span,
                    )),
                }
            }
            ExprKind::Assign { target, op, value } => {
                let rhs = self.eval_expr(value, env)?;
                // Unwrap `*x`/`&x` (pointers are identity here).
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
                            // first assignment to a free name → define it
                            env.borrow_mut().define(name.clone(), rhs);
                        }
                        Ok(Value::Unit)
                    }
                    // `obj.field = v` / `self.field = v`.
                    ExprKind::Member { base, field, .. } => {
                        let recv = self.eval_expr(base, env)?;
                        if let Value::Struct { fields, .. } = recv {
                            let cur = fields.borrow().get(field).cloned().unwrap_or(Value::Unit);
                            let nv = compound(self, cur, expr.span)?;
                            fields.borrow_mut().insert(field.clone(), nv);
                            Ok(Value::Unit)
                        } else {
                            Err(RuntimeError::new(
                                "field assignment on non-struct value",
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
                        Err(RuntimeError::new("invalid assignment index", target.span))
                    }
                    // Destructuring: `(a, b) = (1, 2)`, aninhado também.
                    ExprKind::Tuple(targets) => {
                        self.destructure(targets, rhs, env, target.span)?;
                        Ok(Value::Unit)
                    }
                    _ => Err(RuntimeError::new(
                        "unsupported assignment target",
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
            ExprKind::Tuple(items) => {
                let vals = items
                    .iter()
                    .map(|e| self.eval_expr(e, env))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Value::Tuple(vals))
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
            // `as` cast: in the interpreter we treat it as identity (no static
            // type checking), except for obvious numeric conversions.
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
                Err(RuntimeError::new("no match arm matched", expr.span))
            }
            ExprKind::If { cond, then, els } => {
                let c = self.eval_expr(cond, env)?;
                match c.as_bool() {
                    Some(true) => self.eval_expr(then, env),
                    Some(false) => match els {
                        Some(e) => self.eval_expr(e, env),
                        None => Ok(Value::Unit),
                    },
                    None => Err(RuntimeError::new("`if` condition is not bool", cond.span)),
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
                // `expr?`: Ok(v)/Some(v) → v; Err/None propagates as a runtime
                // error (simplified model, no early-return from function).
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
                    // `vec![..]` arrives as Call{callee: Ident("vec"), args:[Array]}.
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
                    // Option/Result constructors.
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
                        // Imported stdlib function?
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
                    // Method call: `recv.method(args)`.
                    ExprKind::Member {
                        base,
                        field,
                        optional,
                    } => {
                        let recv = self.eval_expr(base, env)?;
                        // `recv?.method()` on None → None.
                        if *optional {
                            if let Value::Enum { variant, .. } = &recv {
                                if variant == "None" {
                                    return Ok(Value::none());
                                }
                            }
                        }
                        self.call_method(recv, field, arg_vals, expr.span)
                    }
                    // Associated function: `Type::func(args)` or enum variant.
                    ExprKind::Path { segments } => self.call_path(segments, arg_vals, expr.span),
                    _ => Err(RuntimeError::new("unsupported call target", callee.span)),
                }
            }
            _ => Err(RuntimeError::new(
                "construct not yet supported by the VM",
                expr.span,
            )),
        }
    }

    /// `base[index]` — indexing into a `Vec` (by int) or tuple (by int).
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
                        "index must be a non-negative int, got {}",
                        other.type_name()
                    ),
                    span,
                ))
            }
        };
        match base {
            Value::Vec(items) => items
                .borrow()
                .get(idx)
                .cloned()
                .ok_or_else(|| RuntimeError::new(format!("index {idx} out of bounds"), span)),
            Value::Tuple(items) => items
                .get(idx)
                .cloned()
                .ok_or_else(|| RuntimeError::new(format!("index {idx} out of tuple bounds"), span)),
            other => Err(RuntimeError::new(
                format!("cannot index into {}", other.type_name()),
                span,
            )),
        }
    }

    /// Tuple destructuring: binds each target (ident or nested tuple) to
    /// the corresponding element of `val`.
    fn destructure(
        &mut self,
        targets: &[Expr],
        val: Value,
        env: &Rc<RefCell<Env>>,
        span: copper_syntax::ast::Span,
    ) -> Result<(), RuntimeError> {
        let elems = match val {
            Value::Tuple(e) => e,
            other => {
                return Err(RuntimeError::new(
                    format!("destructuring expects a tuple, got {}", other.type_name()),
                    span,
                ))
            }
        };
        if elems.len() != targets.len() {
            return Err(RuntimeError::new(
                format!(
                    "tuple of {} elements for {} targets",
                    elems.len(),
                    targets.len()
                ),
                span,
            ));
        }
        for (t, v) in targets.iter().zip(elems) {
            // Unwrap `mut`/`&` if they appear as Unary on the target.
            let mut tk = t;
            while let ExprKind::Unary { expr: inner, .. } = &tk.kind {
                tk = inner;
            }
            match &tk.kind {
                ExprKind::Ident(name) => {
                    if !env.borrow_mut().set(name, v.clone()) {
                        env.borrow_mut().define(name.clone(), v);
                    }
                }
                ExprKind::Tuple(inner) => self.destructure(inner, v, env, span)?,
                _ => {
                    return Err(RuntimeError::new(
                        "unsupported destructuring target",
                        tk.span,
                    ))
                }
            }
        }
        Ok(())
    }

    /// `base[key]` by string — for JSON objects (`Value::Struct`).
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
                format!("cannot index {} by string", other.type_name()),
                span,
            )),
        }
    }

    /// `base.field` (and `base?.field`). Covers struct fields, `.0`/`.1` on
    /// tuples, and `None` propagation through `?.`.
    fn member_value(
        &mut self,
        base: Value,
        field: &str,
        optional: bool,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        // `?.` on `None` → stays `None`.
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
                    RuntimeError::new(format!("`{name}` has no field `{field}`"), span)
                })
            }
            Value::Tuple(items) => {
                // `.0.0` may arrive as the field "0.0" (the tokenizer merged the
                // number). Treat each segment as a chained index.
                let mut cur = Value::Tuple(items);
                for seg in field.split('.') {
                    let idx: usize = seg.parse().map_err(|_| {
                        RuntimeError::new(format!("invalid tuple index `.{seg}`"), span)
                    })?;
                    cur = match cur {
                        Value::Tuple(ref t) => t.get(idx).cloned().ok_or_else(|| {
                            RuntimeError::new(format!("tuple has no `.{idx}`"), span)
                        })?,
                        other => {
                            return Err(RuntimeError::new(
                                format!("`.{idx}` on {}", other.type_name()),
                                span,
                            ))
                        }
                    };
                }
                Ok(cur)
            }
            other => Err(RuntimeError::new(
                format!("cannot access `.{field}` on {}", other.type_name()),
                span,
            )),
        }
    }

    /// Tries to match `pat` against `val`, binding names in `scope`.
    /// Returns `true` if it matched.
    fn match_pattern(&self, pat: &Pattern, val: &Value, scope: &Rc<RefCell<Env>>) -> bool {
        match pat {
            Pattern::Wildcard => true,
            Pattern::Ident(name) => {
                // Nullary variant (e.g. `None`) matches by variant equality;
                // otherwise it is a binding that matches any value.
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

    /// Invokes a `FuncDef` (function, method, or associated). If `receiver` is
    /// `Some`, it is bound to the first `self` parameter; the remaining
    /// parameters receive `args` in order.
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
            // Skip the `self` parameter (if declared) and bind it.
            if def.params.first().map(|p| p.as_str()) == Some("self") {
                params.next();
            }
            scope.borrow_mut().define("self".to_string(), recv);
        }
        let rest: Vec<&String> = params.collect();
        if rest.len() != args.len() {
            return Err(RuntimeError::new(
                format!("function expects {} args, got {}", rest.len(), args.len()),
                span,
            ));
        }
        for (p, a) in rest.into_iter().zip(args) {
            scope.borrow_mut().define(p.clone(), a);
        }
        let ret = match self.run_block_in(&def.body, &scope)? {
            Outcome::Return(v) | Outcome::Normal(v) => v,
            Outcome::Break | Outcome::Continue => {
                return Err(RuntimeError::new("break/continue outside of loop", span))
            }
        };
        check_return_type(def, &ret, span)?;
        Ok(ret)
    }

    /// Applies a closure to arguments.
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

    /// `recv.method(args)` — tries built-in methods first, then user-defined methods.
    fn call_method(
        &mut self,
        recv: Value,
        name: &str,
        args: Vec<Value>,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        // Closure iterator adapters (need the interpreter).
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
        // `Response` methods from the http module.
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
            format!("method `{name}` not found on {}", recv.type_name()),
            span,
        ))
    }

    /// `Type::func(args)` — associated function, or enum variant construction.
    fn call_path(
        &mut self,
        segments: &[String],
        args: Vec<Value>,
        span: copper_syntax::ast::Span,
    ) -> Result<Value, RuntimeError> {
        if segments.len() == 2 {
            let (ty, name) = (&segments[0], &segments[1]);
            // Class constructor: creates the instance, runs the body (which does
            // `self.field = ...`) and returns the instance.
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
            // Not a known associated function → enum variant.
            return Ok(Value::Enum {
                ty: ty.clone(),
                variant: name.clone(),
                payload: args,
            });
        }
        Err(RuntimeError::new(
            format!("path `{}` not supported", segments.join("::")),
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
                "break/continue outside of a loop",
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

    /// Like `run_block`, but without creating an extra child scope (the caller
    /// already created one — used by `for`, which injects the loop variable).
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
                    _ => return Err(RuntimeError::new("invalid ++/-- target", *span)),
                };
                let cur = env.borrow().get(&name).ok_or_else(|| {
                    RuntimeError::new(format!("variable `{name}` not defined"), *span)
                })?;
                let next = match cur {
                    Value::Int(n) => Value::Int(if *inc { n + 1 } else { n - 1 }),
                    other => {
                        return Err(RuntimeError::new(
                            format!("++/-- requires int, got {}", other.type_name()),
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
                    None => Err(RuntimeError::new("`if` condition is not bool", cond.span)),
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
                                "`while` condition is not bool",
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
                            "`for` pattern not supported (simple name only)",
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
                                    "`for` only iterates int ranges in the MVP",
                                    *span,
                                ))
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "`for` only iterates ranges in the MVP",
                            iter.span,
                        ))
                    }
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
            // `unsafe { ... }` — no special semantics in the interpreter.
            Stmt::Unsafe { body, .. } => self.run_block(body, env),
            _ => Err(RuntimeError::new(
                "statement not yet supported by the VM",
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
                .ok_or_else(|| RuntimeError::new("overflow in integer negation", span)),
            (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
            (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
            // Ref/deref: in the interpreter these are identity (there is no real
            // pointer model; `unsafe`/raw-ptr execute without aliasing).
            (UnOp::Ref, v) | (UnOp::RefMut, v) | (UnOp::Deref, v) => Ok(v),
            (op, v) => Err(RuntimeError::new(
                format!("unary operator {op:?} invalid for {}", v.type_name()),
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
                .ok_or_else(|| RuntimeError::new("integer addition overflow", span)),
            (Sub, Int(a), Int(b)) => a
                .checked_sub(b)
                .map(Int)
                .ok_or_else(|| RuntimeError::new("integer subtraction overflow", span)),
            (Mul, Int(a), Int(b)) => a
                .checked_mul(b)
                .map(Int)
                .ok_or_else(|| RuntimeError::new("integer multiplication overflow", span)),
            (Div, Int(a), Int(b)) => a
                .checked_div(b)
                .map(Int)
                .ok_or_else(|| RuntimeError::new("division by zero or overflow", span)),
            (Rem, Int(a), Int(b)) => a
                .checked_rem(b)
                .map(Int)
                .ok_or_else(|| RuntimeError::new("remainder by zero or overflow", span)),
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
                    "operator {op:?} invalid for {} and {}",
                    a.type_name(),
                    b.type_name()
                ),
                span,
            )),
        }
    }
}

/// Built-in methods (Rust style) on values. Returns `None` when the method
/// is not built-in (the caller then tries user-defined methods). Never
/// panics: errors become `RuntimeError`.
fn builtin_method(
    recv: &Value,
    name: &str,
    args: &[Value],
    span: copper_syntax::ast::Span,
) -> Option<Result<Value, RuntimeError>> {
    // Methods valid on any value.
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
                    Err(_) => Value::err(Value::Str(format!("could not parse `{s}`"))),
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
            // `into_iter` returns an independent cursor (clone) so that
            // `.next()` consumes without affecting the original vec.
            "into_iter" => Some(Ok(Value::Vec(Rc::new(RefCell::new(
                items.borrow().clone(),
            ))))),
            // Other adapters: eager model, return the vec itself.
            "iter" | "copied" | "cloned" | "collect" => Some(Ok(Value::Vec(Rc::clone(items)))),
            // Consume the first element (drains the front of the cursor).
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

/// Converts a pattern literal to a `Value` for comparison.
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
                    return None; // interpolation in patterns is not supported
                }
            }
            Some(Value::Str(s))
        }
    }
}

/// Verifies a function's returned value matches its declared return type.
/// Lenient for user-defined / unknown types (only flags clear scalar mismatches)
/// so valid programs aren't rejected.
fn check_return_type(
    def: &FuncDef,
    v: &Value,
    span: copper_syntax::ast::Span,
) -> Result<(), RuntimeError> {
    let Some(ty) = &def.return_type else {
        return Ok(());
    };
    if type_matches(ty, v) {
        Ok(())
    } else {
        Err(RuntimeError::new(
            format!(
                "`{}` declared to return `{}` but returned a `{}`",
                def.name,
                type_label(ty),
                v.type_name()
            ),
            span,
        ))
    }
}

/// Does runtime value `v` satisfy declared type `ty`? Strict for scalars;
/// lenient (true) for unknown/user types to avoid false positives.
fn type_matches(ty: &copper_syntax::expr::Type, v: &Value) -> bool {
    use copper_syntax::expr::Type as T;
    match ty {
        T::Int => matches!(v, Value::Int(_)),
        T::Float => matches!(v, Value::Float(_) | Value::Int(_)),
        T::Bool => matches!(v, Value::Bool(_)),
        T::Str => matches!(v, Value::Str(_)),
        T::Unit => matches!(v, Value::Unit),
        T::Option(_) => matches!(v, Value::Enum { ty, .. } if ty == "Option"),
        T::Vec(_) => matches!(v, Value::Vec(_)),
        T::Fn(_, _) => matches!(v, Value::Closure(_)),
        T::Named(name, _) => named_type_matches(name, v),
        T::Unknown => true,
    }
}

fn named_type_matches(name: &str, v: &Value) -> bool {
    // Use the base name without generic args: `Result<i32, E>` → `Result`.
    let name = name.split('<').next().unwrap_or(name).trim();
    match name {
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
        | "usize" | "int" => matches!(v, Value::Int(_)),
        "f32" | "f64" | "float" => matches!(v, Value::Float(_) | Value::Int(_)),
        "bool" => matches!(v, Value::Bool(_)),
        "String" | "str" | "string" => matches!(v, Value::Str(_)),
        "void" | "unit" | "()" => matches!(v, Value::Unit),
        "Option" => matches!(v, Value::Enum { ty, .. } if ty == "Option"),
        "Result" => matches!(v, Value::Enum { ty, .. } if ty == "Result"),
        "Vec" => matches!(v, Value::Vec(_)),
        other => match v {
            Value::Struct { name, .. } => name == other,
            Value::Enum { ty, .. } => ty == other,
            _ => true,
        },
    }
}

/// Human-readable label for a declared type, for error messages.
fn type_label(ty: &copper_syntax::expr::Type) -> String {
    use copper_syntax::expr::Type as T;
    match ty {
        T::Int => "int".into(),
        T::Float => "float".into(),
        T::Bool => "bool".into(),
        T::Str => "str".into(),
        T::Unit => "void".into(),
        T::Option(_) => "Option".into(),
        T::Vec(_) => "Vec".into(),
        T::Fn(_, _) => "closure".into(),
        T::Named(n, _) => n.clone(),
        T::Unknown => "?".into(),
    }
}

/// `expr as Type` — numeric conversions; everything else is identity.
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

/// Renders the arguments of `println!`/`print!`. If the first argument is
/// a string with `{...}` placeholders and there are more arguments, performs
/// positional substitution in `format!` style (the inner spec — `{}`, `{:?}`,
/// `{:.2}` — is ignored; `Display` is used). Otherwise joins everything with spaces.
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

/// Replaces each `{...}` with `args[i].to_string()`, in order. `{{`/`}}` are
/// literal braces. Extra placeholders without an argument become empty.
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
                // Consume until the closing '}' (ignores the format spec).
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
        let expr = expr.expect("no expr");
        let env = Env::new();
        Interpreter::new()
            .eval_expr(&expr, &env)
            .expect("runtime error")
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
        let expr = expr.expect("no expr");
        let env = Env::new();
        Interpreter::new().eval_expr(&expr, &env)
    }

    #[test]
    fn div_by_zero_is_err() {
        assert!(eval_err("1 / 0").is_err(), "division by zero should be Err");
    }

    #[test]
    fn rem_by_zero_is_err() {
        assert!(
            eval_err("5 % 0").is_err(),
            "remainder by zero should be Err"
        );
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
        assert!(result.is_err(), "i64::MIN / -1 should be Err (overflow)");
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
        assert!(result.is_err(), "i64::MIN % -1 should be Err (overflow)");
    }

    #[test]
    fn undefined_variable_is_err() {
        assert!(
            eval_err("naoexiste").is_err(),
            "undefined variable should be Err"
        );
    }

    use copper_syntax::expr::parse_stmts;
    use copper_syntax::program::parse_program;

    fn run_prog(src: &str) -> Value {
        let prog = parse_program(src);
        assert!(prog.errors.is_empty(), "prog errs: {:?}", prog.errors);
        Interpreter::new()
            .run_program(&prog)
            .expect("runtime error")
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
            .expect("runtime error")
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
        // simple interpolation
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
            .expect("runtime error");
        assert_eq!(buf.borrow().as_str(), "a\nb");
    }
}
