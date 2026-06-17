# Reactive `if`/`for`/`match` in the MUI codegen — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `if`/`for`/`match` in MUI node position reactive — the generated Rust re-chooses branches / re-iterates lists when a signal read by the control-flow expression changes.

**Architecture:** The control-flow expression is transpiled to a Rust expression (`Expr → String`) and emitted *inside* the view function, so an `if`/`for`/`match` re-evaluates every time the view fn re-runs. Reactivity comes from a whole-view rebuild: structural signals flip a shared `dirty: Rc<Cell<bool>>`; an `App::on_tick` closure polls it each frame, snapshots signal values into a `Seed`, re-runs the view fn seeded with those values, and swaps the tree via `UIApp_SetChildren`. This mirrors the proven `mocida-rs/mui-runtime` model.

**Tech Stack:** Rust; crates `mui-codegen`, `mui-syntax`, `copper-syntax`; runtime `mocida-rs` (`mocida` crate). No new external dependencies.

## Global Constraints

- No new external crate dependencies. Reuse `copper_syntax::expr::parse_expr` and existing `mocida` APIs (`App::on_tick`, `App::as_ptr`, `App::set_children` / `sys::UIApp_SetChildren`).
- Signal kinds are limited to `Int` (`Signal<i32>`), `Str` (`Signal<String>`), and `List` (`Rc<RefCell<Vec<String>>>`). No arbitrary `Signal<Struct>`.
- `match` patterns: literals and `_` wildcard only. No binds/destructuring.
- Unsupported operators/expressions in a control-flow expression must produce a **clear codegen-time error** (a `Result` error or a generated `compile_error!`), never a silent wrong render.
- The existing test suite (`cargo test`, currently 131 tests, 0 failures) must stay green at every commit.
- Commit messages: no `Co-Authored-By` trailer.
- Generated Rust must compile under `rustc --edition 2021 --crate-type bin` (use `--crate-type lib` only when the entry view is literally named `main`, a known pre-existing quirk).

---

## File Structure

- `crates/mui-syntax/src/lib.rs` — `parse_match_node` rewritten to parse real arms (Task 1).
- `crates/mui-syntax/src/ast.rs` — `HandlerAction` gains string/list variants; `parse_actions` extended (Task 3).
- `crates/mui-codegen/src/eval.rs` — **new** module: `expr_to_rust` (Expr→Rust string) + `collect_reads` (signal names referenced). Foundation for Tasks 5–7 (Task 2).
- `crates/mui-codegen/src/lib.rs` — `SignalScope`/`declare_signals` gain `SigKind` (Task 3); `header()`/`BuiltView`/`generate_view_fn`/`generate_main` gain the seed + dirty + rebuild engine (Task 4); `emit_node` lowers `if`/`for`/`match` (Tasks 5–7).

---

## Task 1: Parse real `match` arms in `mui-syntax`

**Files:**
- Modify: `crates/mui-syntax/src/lib.rs` (`parse_match_node`, ~lines 946-970)
- Test: `crates/mui-syntax/src/lib.rs` (inline `#[cfg(test)]`) or `crates/mui-syntax/tests/`

**Interfaces:**
- Consumes: existing `Parser` helpers (`eat`, `peek_val`, `bump`, `parse_block`, `expect_ident`, `span_of`), `copper_syntax::expr::parse_expr`.
- Produces: `Node::Match { scrutinee: Expr, arms: Vec<MatchArm>, span }` where `MatchArm { pattern: String, body: Vec<Node>, span }` is filled with the real scrutinee expr and one entry per `pattern => body` arm.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn parses_match_arms_with_bodies() {
    let src = r#"
app { name: "M" width: 100 height: 100 entry: V }
view V() {
  match status {
    "on" => { Text("ON") }
    _ => { Text("OFF") }
  }
}
"#;
    let doc = crate::loader::load_str(src).expect("parse");
    let view = doc.views.iter().find(|v| v.name == "V").unwrap();
    let m = view.body.iter().find_map(|n| match n {
        crate::ast::Node::Match { arms, scrutinee, .. } => Some((arms.clone(), scrutinee.clone())),
        _ => None,
    }).expect("match node");
    assert_eq!(m.0.len(), 2, "two arms");
    assert_eq!(m.0[0].pattern, "\"on\"");
    assert_eq!(m.0[1].pattern, "_");
    assert_eq!(m.0[0].body.len(), 1, "first arm has one node");
}
```

> Note: confirm the loader entry point name (`crate::loader::load_str` / `load_from`) against the file; use whatever the other parser tests in this crate use.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mui-syntax parses_match_arms_with_bodies`
Expected: FAIL — `arms.len()` is 0 (current stub returns `arms: Vec::new()`).

- [ ] **Step 3: Rewrite `parse_match_node`**

Replace the body-skipping stub. Capture the scrutinee tokens up to `{` (like `parse_if_node` captures `cond_raw`), parse it via `copper_syntax::expr::parse_expr`, then parse arms until the closing `}`:

```rust
fn parse_match_node(&mut self) -> Option<Node> {
    let start = span_of(self.toks.get(self.pos)?);
    self.eat("match");
    // Scrutinee: tokens up to the opening `{`.
    let mut scrut_parts: Vec<String> = Vec::new();
    while let Some(v) = self.peek_val() {
        if v == "{" { break; }
        let tok = self.bump().unwrap().value;
        if !tok.trim().is_empty() { scrut_parts.push(tok); }
    }
    let scrut_raw = scrut_parts.join(" ");
    let scrutinee = copper_syntax::expr::parse_expr(&scrut_raw).0.unwrap_or_else(raw_expr);
    let mut arms: Vec<MatchArm> = Vec::new();
    if self.eat("{") {
        while let Some(v) = self.peek_val() {
            if v == "}" { self.eat("}"); break; }
            // Pattern: tokens up to `=>` (space-joined, preserves quotes).
            let arm_start = span_of(self.toks.get(self.pos)?);
            let mut pat_parts: Vec<String> = Vec::new();
            while let Some(pv) = self.peek_val() {
                if pv == "=>" { break; }
                let tok = self.bump().unwrap().value;
                if !tok.trim().is_empty() { pat_parts.push(tok); }
            }
            let pattern = pat_parts.join(" ");
            self.eat("=>");
            // Body: a `{ ... }` block, or a single node.
            let body = if self.peek_val() == Some("{") {
                self.parse_block()
            } else {
                self.parse_node().map(|n| vec![n]).unwrap_or_default()
            };
            // Optional trailing comma.
            self.eat(",");
            let arm_end = self.toks.get(self.pos.saturating_sub(1)).map(span_of).unwrap_or(arm_start);
            arms.push(MatchArm { pattern, body, span: Span::merge(arm_start, arm_end) });
        }
    }
    let end = self.toks.get(self.pos.saturating_sub(1)).map(span_of).unwrap_or(start);
    Some(Node::Match { scrutinee, arms, span: Span::merge(start, end) })
}
```

> Verify against the file: the single-node parse helper may be named `parse_node` / `parse_child` — use the same one `parse_block` calls internally. If `MatchArm` has no `span` field, drop it from the literal.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p mui-syntax parses_match_arms_with_bodies`
Expected: PASS.

- [ ] **Step 5: Run the crate suite**

Run: `cargo test -p mui-syntax`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/mui-syntax/src/lib.rs
git commit -m "feat(mui-syntax): parse match arms (pattern => body) instead of skipping"
```

---

## Task 2: Expression→Rust transpiler (`eval.rs`)

**Files:**
- Create: `crates/mui-codegen/src/eval.rs`
- Modify: `crates/mui-codegen/src/lib.rs` (add `mod eval;`)
- Test: inline `#[cfg(test)]` in `eval.rs`

**Interfaces:**
- Consumes: `copper_syntax::expr::{Expr, ExprKind, Literal, BinOp, UnOp}`; a `SigKind` lookup `Fn(&str) -> Option<SigKind>` (defined in Task 3 but stub a local copy here — see note).
- Produces:
  - `pub fn expr_to_rust(expr: &Expr, sig: &dyn Fn(&str) -> Option<SigKind>, env: &dyn Fn(&str) -> Option<String>) -> Result<String, String>` — returns a Rust expression string evaluating the Copper expr at runtime; `Err` describes the first unsupported construct.
  - `pub fn collect_reads(expr: &Expr, out: &mut Vec<String>)` — appends every bare `Ident` name referenced (so the lowering can subscribe those that are signals).
  - `pub enum SigKind { Int, Str, List }` lives here (re-used by lib.rs).

> Note on ordering: define `SigKind` in `eval.rs` and have lib.rs `use crate::eval::SigKind;`. This keeps Task 2 independently compilable/testable before Task 3.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use copper_syntax::expr::parse_expr;

    fn sig(name: &str) -> Option<SigKind> {
        match name { "count" => Some(SigKind::Int), "status" => Some(SigKind::Str),
                     "items" => Some(SigKind::List), _ => None }
    }
    fn env(_: &str) -> Option<String> { None }

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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mui-codegen --lib eval`
Expected: FAIL — `eval` module/functions don't exist.

- [ ] **Step 3: Implement `eval.rs`**

```rust
//! Build-time transpiler: lowers a Copper control-flow `Expr` into a Rust
//! expression string, emitted inside the view fn so it re-evaluates against
//! live signal values each rebuild. Reactivity is whole-view rebuild, not
//! const-folding — nothing here is evaluated at codegen time.

use copper_syntax::expr::{BinOp, Expr, ExprKind, Literal, UnOp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigKind { Int, Str, List }

fn sig_var(name: &str) -> String { format!("__sig_{}", super::sanitize_ident(name)) }

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
            None => env(name).ok_or_else(|| format!("unknown name `{name}` in control-flow expr")),
        },
        ExprKind::Unary { op, expr } => {
            let inner = expr_to_rust(expr, sig, env)?;
            let o = match op { UnOp::Not => "!", UnOp::Neg => "-" };
            Ok(format!("({o}{inner})"))
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let l = expr_to_rust(lhs, sig, env)?;
            let mut r = expr_to_rust(rhs, sig, env)?;
            // `<str signal> == "lit"`: rhs literal already `.to_string()`-suffixed
            // by the Str-literal arm, so the comparison is String == String.
            let o = match op {
                BinOp::Eq => "==", BinOp::Ne => "!=", BinOp::Lt => "<",
                BinOp::Gt => ">", BinOp::And => "&&", BinOp::Or => "||",
                other => return Err(format!("unsupported operator {other:?} in control-flow expr")),
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
        ExprKind::Index { base, index } => {
            let b = expr_to_rust(base, sig, env)?;
            let i = expr_to_rust(index, sig, env)?;
            Ok(format!("{b}[({i}) as usize].clone()"))
        }
        ExprKind::Array(items) => {
            let parts: Result<Vec<_>, _> = items.iter().map(|x| expr_to_rust(x, sig, env)).collect();
            Ok(format!("vec![{}]", parts?.join(", ")))
        }
        ExprKind::Range { start, end, inclusive } => {
            let s = expr_to_rust(start, sig, env)?;
            let en = expr_to_rust(end, sig, env)?;
            Ok(format!("({s}..{}{en})", if *inclusive { "=" } else { "" }))
        }
        other => Err(format!("unsupported expression {other:?} in control-flow position")),
    }
}

/// Append every bare identifier referenced in `expr` (caller filters to signals).
pub fn collect_reads(expr: &Expr, out: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::Ident(n) => out.push(n.clone()),
        ExprKind::Unary { expr, .. } => collect_reads(expr, out),
        ExprKind::Binary { lhs, rhs, .. } => { collect_reads(lhs, out); collect_reads(rhs, out); }
        ExprKind::Call { callee, args, .. } => { collect_reads(callee, out); args.iter().for_each(|a| collect_reads(a, out)); }
        ExprKind::Member { base, .. } => collect_reads(base, out),
        ExprKind::Index { base, index } => { collect_reads(base, out); collect_reads(index, out); }
        ExprKind::Array(xs) => xs.iter().for_each(|x| collect_reads(x, out)),
        ExprKind::Range { start, end, .. } => { collect_reads(start, out); collect_reads(end, out); }
        _ => {}
    }
}
```

Add to `crates/mui-codegen/src/lib.rs` near the other `mod` lines:

```rust
mod eval;
```

> The helper `str_template_literal` (returns `Some(String)` for a non-interpolated `StrTemplate`, else `None`) and `sanitize_ident` must be `pub(crate)` in lib.rs. `sanitize_ident` already exists — make it `pub(crate)`; add `str_template_literal` if not present (a tiny function over `StrTemplate.parts`: one `StrPart::Lit(s)` → `Some(s)`, anything else → `None`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p mui-codegen --lib eval`
Expected: PASS (all 5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/mui-codegen/src/eval.rs crates/mui-codegen/src/lib.rs
git commit -m "feat(mui-codegen): add Expr->Rust transpiler for control-flow exprs"
```

---

## Task 3: Generalize signals to Int / Str / List

**Files:**
- Modify: `crates/mui-codegen/src/lib.rs` (`SignalScope`, `declare_signals`, `signal_init`, handler emission)
- Modify: `crates/mui-syntax/src/ast.rs` (`HandlerAction`, `parse_actions`)
- Test: inline `#[cfg(test)]` in `mui-codegen/src/lib.rs`

**Interfaces:**
- Consumes: `crate::eval::SigKind`.
- Produces:
  - `SignalScope` maps `name -> (var: String, kind: SigKind)`; `fn var(&self, name) -> Option<&str>` kept; add `fn kind(&self, name) -> Option<SigKind>`.
  - `HandlerAction` gains `SetStr { name, value: String }`, `SetList { name, values: Vec<String> }`, `PushList { name, value: String }`, `ClearList { name }`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn declares_string_and_list_signals() {
    let src = r#"
app { name: "S" width: 200 height: 200 entry: V }
view V() {
  mut status = signal("on")
  mut items = signal([])
  Text("${status}")
}
"#;
    let code = crate::generate_from_str(src).expect("codegen");
    assert!(code.contains("Signal::<String>::new"), "string signal:\n{code}");
    assert!(code.contains("Rc::new(RefCell::new(Vec::<String>::new()))"), "list signal:\n{code}");
}
```

> Use whatever the existing codegen tests call to generate from a string (search the test module for the helper, e.g. `generate_program` / a test-only wrapper). Match its name.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mui-codegen --lib declares_string_and_list_signals`
Expected: FAIL — only `Signal::<i32>` is emitted today.

- [ ] **Step 3: Add `SigKind` to `SignalScope` and classify in `declare_signals`**

```rust
#[derive(Default, Clone)]
struct SignalScope {
    vars: std::collections::HashMap<String, (String, crate::eval::SigKind)>,
}
impl SignalScope {
    fn var(&self, name: &str) -> Option<&str> { self.vars.get(name).map(|(v, _)| v.as_str()) }
    fn kind(&self, name: &str) -> Option<crate::eval::SigKind> { self.vars.get(name).map(|(_, k)| *k) }
}
```

In `declare_signals`, classify the initializer and emit the matching binding:

```rust
use crate::eval::SigKind;
// inside the loop, replacing the i32-only branch:
let var = format!("__sig_{}", sanitize_ident(name));
match signal_kind(expr) {
    SigKind::Int => {
        let init = signal_init(expr, env).unwrap_or_else(|| "0".to_string());
        e.line(&format!("let {var} = Rc::new(RefCell::new(Signal::<i32>::new(__seed.int({name:?}).unwrap_or({init}))?));"));
        e.line(&format!("__keep.push({var}.clone());"));
    }
    SigKind::Str => {
        let init = signal_str_init(expr).unwrap_or_default();
        e.line(&format!("let {var} = Rc::new(RefCell::new(Signal::<String>::new(__seed.str({name:?}).unwrap_or_else(|| {init:?}.to_string()))?));"));
        e.line(&format!("__keep_str.push({var}.clone());"));
    }
    SigKind::List => {
        e.line(&format!("let {var} = Rc::new(RefCell::new(__seed.list({name:?}).unwrap_or_default()));"));
        e.line(&format!("__keep_list.push({var}.clone());"));
    }
}
scope.vars.insert(name.clone(), (var, signal_kind(expr)));
```

Add `signal_kind(&Expr) -> SigKind` (int literal → Int; string literal → Str; `Array`/`[]` → List; default Int) and `signal_str_init(&Expr) -> Option<String>` (the plain literal text of a string initializer). `__seed`, `__keep_str`, `__keep_list` are introduced in Task 4 — for this task, add them to the view-fn preamble too (a small forward dependency; emit `let mut __keep_str: Vec<Rc<RefCell<Signal<String>>>> = Vec::new();` and `let mut __keep_list: Vec<Rc<RefCell<Vec<String>>>> = Vec::new();`, and accept `__seed` as a param — see Task 4 for the full signature; if executing strictly in order, temporarily seed with `Seed::new()` default and a no-op `Seed` shim, then Task 4 wires the real plumbing).

> Recommended: execute Task 4 **before** finalizing Task 3's seed references, or do Tasks 3 and 4 in one branch. They share the view-fn preamble. The plan keeps them separate for review granularity; the implementer may merge their commits if cleaner.

- [ ] **Step 4: Extend `HandlerAction` + `parse_actions` (`mui-syntax/src/ast.rs`)**

```rust
pub enum HandlerAction {
    AddAssign { name: String, delta: i64 },
    SetInt { name: String, value: i64 },
    SetStr { name: String, value: String },
    SetList { name: String, values: Vec<String> },
    PushList { name: String, value: String },
    ClearList { name: String },
}
```

In `parse_actions`, recognize `name = "literal"` → `SetStr`; `name = []` → `ClearList`/`SetList`; `name.push(x)` → `PushList`. Keep `fn name(&self) -> &str` covering all variants.

- [ ] **Step 5: Run tests**

Run: `cargo test -p mui-codegen --lib declares_string_and_list_signals && cargo test -p mui-syntax`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/mui-codegen/src/lib.rs crates/mui-syntax/src/ast.rs
git commit -m "feat(mui-codegen): generalize signals to Int/Str/List with seedable init"
```

---

## Task 4: Seed + dirty + rebuild engine

**Files:**
- Modify: `crates/mui-codegen/src/lib.rs` (`header`, `BuiltView`, `generate_view_fn`, `generate_main`)
- Test: inline `#[cfg(test)]` in `mui-codegen/src/lib.rs`

**Interfaces:**
- Consumes: `crate::eval::SigKind`; the `SignalScope` from Task 3.
- Produces:
  - View fn signature: `fn build_<view>(<params>, __seed: &Seed, __dirty: Rc<Cell<bool>>) -> MuiResult<BuiltView>`.
  - `Seed` type with `fn new() -> Self`, `fn int(&self, &str) -> Option<i32>`, `fn str(&self, &str) -> Option<String>`, `fn list(&self, &str) -> Option<Vec<String>>`.
  - `BuiltView` gains `snapshot: Box<dyn Fn() -> Seed>` plus `_signals_str`, `_signals_list` keep-alive vecs.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn main_installs_rebuild_tick() {
    let src = r#"
app { name: "R" width: 200 height: 200 entry: V }
view V() {
  mut count = signal(0)
  if count > 0 { Text("pos") } else { Text("zero") }
  Button("inc", onClick: { count = count + 1 })
}
"#;
    let code = crate::generate_from_str(src).expect("codegen");
    assert!(code.contains("Rc::new(Cell::new(false))"), "dirty flag:\n{code}");
    assert!(code.contains(".on_tick("), "tick installed:\n{code}");
    assert!(code.contains("UIApp_SetChildren"), "swap via raw ptr:\n{code}");
    assert!(code.contains("__seed"), "seed threaded:\n{code}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mui-codegen --lib main_installs_rebuild_tick`
Expected: FAIL — no dirty flag / on_tick today.

- [ ] **Step 3: Add `Seed` + new `BuiltView` + scaffolding to `header()`**

Append to the generated header string:

```rust
use std::cell::Cell;

#[derive(Clone, Default)]
struct Seed {
    ints: std::collections::HashMap<String, i32>,
    strs: std::collections::HashMap<String, String>,
    lists: std::collections::HashMap<String, Vec<String>>,
}
impl Seed {
    fn new() -> Self { Self::default() }
    fn int(&self, k: &str) -> Option<i32> { self.ints.get(k).copied() }
    fn str(&self, k: &str) -> Option<String> { self.strs.get(k).cloned() }
    fn list(&self, k: &str) -> Option<Vec<String>> { self.lists.get(k).cloned() }
}

struct BuiltView {
    children: Children,
    snapshot: Box<dyn Fn() -> Seed>,
    _signals: Vec<Rc<RefCell<Signal<i32>>>>,
    _signals_str: Vec<Rc<RefCell<Signal<String>>>>,
    _signals_list: Vec<Rc<RefCell<Vec<String>>>>,
    _subs: Vec<Subscription>,
    _sounds: Vec<Sound>,
}
```

- [ ] **Step 4: Update `generate_view_fn`**

- Change the signature to add `__seed: &Seed, __dirty: Rc<Cell<bool>>` after the view params.
- Emit the new keep-alive vecs in the preamble (`__keep`, `__keep_str`, `__keep_list`).
- After all signals are declared, build the snapshot closure capturing clones of every signal Rc:

```rust
// emitted after declare_signals, before node emission is fine; closure reads at call time
e.line("let __snap = {");
e.indent();
// clone each signal Rc into the closure
for (name, (var, kind)) in sigs.vars.iter() {
    e.line(&format!("let {var} = {var}.clone();"));
}
e.line("move || {");
e.indent();
e.line("let mut __s = Seed::new();");
for (name, (var, kind)) in sigs.vars.iter() {
    match kind {
        SigKind::Int => e.line(&format!("__s.ints.insert({name:?}.to_string(), {var}.borrow().get());")),
        SigKind::Str => e.line(&format!("__s.strs.insert({name:?}.to_string(), {var}.borrow().get());")),
        SigKind::List => e.line(&format!("__s.lists.insert({name:?}.to_string(), {var}.borrow().clone());")),
    }
}
e.line("__s");
e.dedent(); e.line("}");
e.dedent(); e.line("};");
```

> `Seed.ints/strs/lists` must be reachable from generated code — they are private fields of a struct defined in the same generated module, so direct field access is fine.

- Update the return line to include the new fields:

```rust
e.line("Ok(BuiltView { children: __children, snapshot: Box::new(__snap), _signals: __keep, _signals_str: __keep_str, _signals_list: __keep_list, _subs: __subs, _sounds: __sounds })");
```

- [ ] **Step 5: Update `generate_main` to install the rebuild tick**

Replace the static mount (`let view = build_V(args)?; app.set_children(view.children);`) with:

```rust
e.line("let __dirty = Rc::new(Cell::new(false));");
e.line("let mut __seed = Seed::new();");
e.line(&format!("let mut __view = {}({}__seed_arg)?;", view_fn_name(&entry.name), main_args_prefix));
// where the call passes: <entry params...>, &__seed, __dirty.clone()
e.line("let __app_ptr = app.as_ptr();");
e.line("app.set_children(__view.children);");
e.line("{");
e.indent();
e.line("let __dirty2 = __dirty.clone();");
e.line(&format!("let __seed_cell = std::rc::Rc::new(std::cell::RefCell::new(({entry_args_owned}, __seed)));"));
// Simpler: capture by move into the closure (see note).
e.dedent(); e.line("}");
```

The borrow-safe shape (recommended, avoids RefCell gymnastics): move `__view`, `__seed`, the entry args, and `__dirty` into the `on_tick` closure; rebuild and swap via the raw app pointer:

```rust
e.line(&format!("let mut __view = {}(/*args*/, &__seed, __dirty.clone())?;"));
e.line("let __app_ptr = app.as_ptr();");
e.line("app.set_children(__view.children);"); // NOTE: children moved; see below
```

Because `Children` is moved by `set_children`, restructure so the first mount also goes through the raw pointer to keep `__view` owning nothing that's been moved. Concretely emit:

```rust
e.line("let __dirty = Rc::new(Cell::new(false));");
e.line("let mut __seed = Seed::new();");
e.line(&format!("let mut __view = {}({}, &__seed, __dirty.clone())?;", view_fn_name(&entry.name), entry_args));
e.line("let __app_ptr = app.as_ptr();");
e.line("unsafe { mocida_sys::UIApp_SetChildren(__app_ptr, __view.children_take()); }");
e.line("app.on_tick(move || {");
e.indent();
e.line("if __dirty.replace(false) {");
e.indent();
e.line("__seed = (__view.snapshot)();");
e.line(&format!("if let Ok(__v) = {}({}, &__seed, __dirty.clone()) {{", view_fn_name(&entry.name), entry_args));
e.indent();
e.line("__view = __v;");
e.line("unsafe { mocida_sys::UIApp_SetChildren(__app_ptr, __view.children_take()); }");
e.dedent(); e.line("}");
e.dedent(); e.line("}");
e.dedent(); e.line("});");
e.line("app.show().run();");
e.line("drop(__view);");
```

Add a `children_take(&mut self) -> *mut mocida_sys::UIChildren` method to the generated `BuiltView` (in `header()`), which `std::mem::replace`s `self.children` with an empty `Children::new(0)` and returns `into_raw()` of the old one. This avoids the partial-move problem and gives a raw pointer for `UIApp_SetChildren`.

> Confirm the sys crate path the generated code uses for `UIApp_SetChildren` — it is re-exported; check an existing generated sample or `mocida`'s prelude. If `mocida_sys` is not a generated-code dependency, add a thin `mocida::app::set_children_raw(ptr, children)` wrapper in the `mocida` crate instead (small, one-repo change) and call that. Prefer the wrapper if `mocida_sys` isn't already linkable from generated bins.

- [ ] **Step 6: Run test + compile a sample**

Run: `cargo test -p mui-codegen --lib main_installs_rebuild_tick`
Expected: PASS.
Then build and rustc-check a sample:
```bash
cargo run -p cforge -- -c -i examples/mui/condtest/condtest.mui
rustc --edition 2021 --crate-type bin dist/rust/src/main.rs -o /tmp/cond_check 2>&1 | head
```
Expected: compiles (the `if` still renders only `then` until Task 5, but the scaffolding compiles).

- [ ] **Step 7: Commit**

```bash
git add crates/mui-codegen/src/lib.rs
git commit -m "feat(mui-codegen): seed/dirty/on_tick whole-view rebuild engine"
```

---

## Task 5: Lower reactive `if`

**Files:**
- Modify: `crates/mui-codegen/src/lib.rs` (`emit_node`, `Node::If` arm)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes: `crate::eval::{expr_to_rust, collect_reads, SigKind}`, `SignalScope::{var,kind}`, `__dirty` in scope, `cond_raw` on the `If` node.
- Produces: emitted `if <rust_cond> { <then> } else { <els> }` + structural subscriptions on `cond`'s signals.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn lowers_reactive_if() {
    let src = r#"
app { name: "I" width: 200 height: 200 entry: V }
view V() {
  mut count = signal(0)
  if count > 0 { Text("pos") } else { Text("zero") }
  Button("inc", onClick: { count = count + 1 })
}
"#;
    let code = crate::generate_from_str(src).expect("codegen");
    assert!(code.contains("if (__sig_count.borrow().get() > 0) {"), "real cond:\n{code}");
    assert!(code.contains("} else {"), "else branch:\n{code}");
    // structural subscribe marks dirty:
    assert!(code.contains("__dirty"), "dirty subscribe:\n{code}");
    assert!(!code.contains("static lowering renders the `then` branch"), "placeholder gone");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mui-codegen --lib lowers_reactive_if`
Expected: FAIL — placeholder still emitted.

- [ ] **Step 3: Implement the `Node::If` arm**

```rust
Node::If { cond_raw, then, els, .. } => {
    let parsed = copper_syntax::expr::parse_expr(cond_raw).0;
    let sig_lookup = |n: &str| sigs.kind(n);
    let env_lookup = |n: &str| env.get(n).map(|s| s.to_string());
    match parsed.as_ref().map(|c| crate::eval::expr_to_rust(c, &sig_lookup, &env_lookup)) {
        Some(Ok(rust_cond)) => {
            // Subscribe structural signals to dirty (once each).
            if let Some(c) = &parsed {
                let mut reads = Vec::new();
                crate::eval::collect_reads(c, &mut reads);
                for r in reads.iter().filter(|r| sigs.var(r).is_some()) {
                    emit_structural_subscribe(e, sigs, r);
                }
            }
            e.line(&format!("if {rust_cond} {{"));
            e.indent();
            for n in then { emit_node(e, n, env, sigs, comps, sink); }
            e.dedent();
            if let Some(els) = els {
                e.line("} else {");
                e.indent();
                for n in els { emit_node(e, n, env, sigs, comps, sink); }
                e.dedent();
            }
            e.line("}");
        }
        _ => {
            // Unsupported condition: fail loudly in generated code.
            e.line(&format!("compile_error!(\"mui: unsupported if condition: {}\");", cond_raw.replace('"', "'")));
        }
    }
}
```

Add the shared helper near `emit_node`:

```rust
/// Emit a one-shot subscription that flips `__dirty` when signal `name` changes.
fn emit_structural_subscribe(e: &mut Emitter, sigs: &SignalScope, name: &str) {
    let var = sigs.var(name).unwrap();
    let kind = sigs.kind(name).unwrap();
    e.line("{");
    e.indent();
    e.line("let __d = __dirty.clone();");
    match kind {
        crate::eval::SigKind::List => {
            // Lists are Rc<RefCell<Vec>>, not Signal — handler mutation already
            // sets dirty explicitly (see handler emission); nothing to subscribe.
            e.line("// list structural dep — dirty set by its mutating handler");
        }
        _ => e.line(&format!(
            "if let Ok(__sub) = {var}.borrow_mut().subscribe(move |_| __d.set(true)) {{ __subs.push(__sub); }}"
        )),
    }
    e.dedent();
    e.line("}");
}
```

> List signals are plain `Rc<RefCell<Vec<String>>>` (no `subscribe`). For a `for` over a list (Task 6) and `if items.len() > 0`, the dirtiness must be set by the **handler** that mutates the list (`PushList`/`SetList`/`ClearList` emit `__dirty.set(true)`). Ensure Task 3's handler emission for list actions appends `__dirty.set(true);`. Add that now if not already: when emitting a `PushList`/`SetList`/`ClearList`, also emit `__d_handler.set(true)` using the dirty clone captured in the handler. (The button-handler emission must capture a `__dirty` clone — thread it through like the signal Rcs.)

- [ ] **Step 4: Run test + compile condtest**

Run: `cargo test -p mui-codegen --lib lowers_reactive_if`
Expected: PASS.
```bash
cargo run -p cforge -- -c -i examples/mui/condtest/condtest.mui
rustc --edition 2021 --crate-type bin dist/rust/src/main.rs -o /tmp/cond_check 2>&1 | head
```
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/mui-codegen/src/lib.rs
git commit -m "feat(mui-codegen): lower reactive if/else with structural subscribe"
```

---

## Task 6: Lower reactive `for`

**Files:**
- Modify: `crates/mui-codegen/src/lib.rs` (`emit_node` `Node::For`, plus the reactive env so the loop var resolves in `${item}`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes: `eval::expr_to_rust` for `iter`; `For { pattern, iter, body }`.
- Produces: emitted `for <pattern> in <rust_iter> { <body with pattern bound> }`, plus structural dirtiness on the iterable.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn lowers_reactive_for() {
    let src = r#"
app { name: "F" width: 200 height: 200 entry: App }
view App(title: string = "Todos") {
  mut items = signal([])
  Stack(orientation: vertical) {
    for item in items {
      Text("${item}")
    }
  }
}
"#;
    let code = crate::generate_from_str(src).expect("codegen");
    assert!(code.contains("for item in __sig_items.borrow().clone() {"), "dynamic loop:\n{code}");
    assert!(!code.contains("renders one iteration"), "placeholder gone:\n{code}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mui-codegen --lib lowers_reactive_for`
Expected: FAIL.

- [ ] **Step 3: Implement the `Node::For` arm**

```rust
Node::For { pattern, iter, body, .. } => {
    let sig_lookup = |n: &str| sigs.kind(n);
    let env_lookup = |n: &str| env.get(n).map(|s| s.to_string());
    match crate::eval::expr_to_rust(iter, &sig_lookup, &env_lookup) {
        Ok(rust_iter) => {
            // Iterable signal made structural (list dirtiness via handler; Signal via subscribe).
            let mut reads = Vec::new();
            crate::eval::collect_reads(iter, &mut reads);
            for r in reads.iter().filter(|r| sigs.var(r).is_some()) {
                emit_structural_subscribe(e, sigs, r);
            }
            let var = sanitize_ident(pattern);
            e.line(&format!("for {var} in {rust_iter} {{"));
            e.indent();
            // Bind the loop var in the reactive env so `${item}` resolves to the
            // Rust local (a String). Use a child env that maps pattern -> the var.
            let mut child_env = env.clone();
            child_env.vars.insert(pattern.clone(), format!("{{{var}}}")); // see note
            for n in body { emit_node(e, n, &child_env, sigs, comps, sink); }
            e.dedent();
            e.line("}");
        }
        Err(msg) => e.line(&format!("compile_error!(\"mui: unsupported for iterable: {}\");", msg.replace('"', "'"))),
    }
}
```

> The loop var binding is the subtle part: `${item}` in a `Text` must lower to the Rust local `item`, not a literal. The existing reactive-text path builds `format!(...)` args from `reads`/`env`. The cleanest approach: treat the loop var as a known local string in a small per-loop binding map that `reactive_format` consults — i.e. when a `${name}` interpolation names the loop var, emit the bare Rust identifier `item` as a `format!` arg instead of resolving via a signal. Implement by passing the set of in-scope loop vars down to `reactive_format` / the text emitter and handling them as direct `{}` args bound to the Rust local. Verify the exact `reactive_format` signature (lib.rs ~2237) and thread a `&[String] loop_vars` param through `emit_text`/`reactive_format`. Keep the change minimal: a loop var resolves to its own identifier.

- [ ] **Step 4: Run test + compile crm example**

Run: `cargo test -p mui-codegen --lib lowers_reactive_for`
Expected: PASS.
```bash
cargo run -p cforge -- -c -i examples/mui/crm/app.crm
rustc --edition 2021 --crate-type bin dist/rust/src/main.rs -o /tmp/crm_check 2>&1 | head
```
Expected: compiles; generated `main.rs` contains the dynamic `for ... in __sig_items...` loop.

- [ ] **Step 5: Commit**

```bash
git add crates/mui-codegen/src/lib.rs
git commit -m "feat(mui-codegen): lower reactive for over list signals"
```

---

## Task 7: Lower reactive `match`

**Files:**
- Modify: `crates/mui-codegen/src/lib.rs` (`emit_node` `Node::Match`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes: `Match { scrutinee, arms }` (now populated by Task 1), `eval::expr_to_rust`.
- Produces: emitted `match <rust_scrutinee> { <pat> => { body }, _ => {} }`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn lowers_reactive_match() {
    let src = r#"
app { name: "M" width: 200 height: 200 entry: V }
view V() {
  mut status = signal("on")
  match status {
    "on" => { Text("ON") }
    _ => { Text("OFF") }
  }
}
"#;
    let code = crate::generate_from_str(src).expect("codegen");
    assert!(code.contains("match (__sig_status.borrow().get()).as_str() {")
         || code.contains("match __sig_status.borrow().get() {"), "match scrutinee:\n{code}");
    assert!(code.contains("\"on\" =>"), "literal arm:\n{code}");
    assert!(code.contains("_ =>"), "wildcard arm:\n{code}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mui-codegen --lib lowers_reactive_match`
Expected: FAIL.

- [ ] **Step 3: Implement the `Node::Match` arm**

```rust
Node::Match { scrutinee, arms, .. } => {
    let sig_lookup = |n: &str| sigs.kind(n);
    let env_lookup = |n: &str| env.get(n).map(|s| s.to_string());
    match crate::eval::expr_to_rust(scrutinee, &sig_lookup, &env_lookup) {
        Ok(rust_scrut) => {
            // Structural subscribe on the scrutinee's signals.
            let mut reads = Vec::new();
            crate::eval::collect_reads(scrutinee, &mut reads);
            for r in reads.iter().filter(|r| sigs.var(r).is_some()) {
                emit_structural_subscribe(e, sigs, r);
            }
            // String scrutinee: match on &str for literal-string arms.
            let scrut_is_str = reads.iter().any(|r| sigs.kind(r) == Some(crate::eval::SigKind::Str));
            if scrut_is_str {
                e.line(&format!("match ({rust_scrut}).as_str() {{"));
            } else {
                e.line(&format!("match {rust_scrut} {{"));
            }
            e.indent();
            let mut saw_wildcard = false;
            for arm in arms {
                let pat = arm.pattern.trim();
                if pat == "_" { saw_wildcard = true; }
                e.line(&format!("{pat} => {{"));
                e.indent();
                for n in &arm.body { emit_node(e, n, env, sigs, comps, sink); }
                e.dedent();
                e.line("}");
            }
            if !saw_wildcard {
                e.line("_ => {}"); // Rust requires exhaustiveness for str matches
            }
            e.dedent();
            e.line("}");
        }
        Err(msg) => e.line(&format!("compile_error!(\"mui: unsupported match scrutinee: {}\");", msg.replace('"', "'"))),
    }
}
```

> For a string scrutinee, literal arm patterns are already source like `"on"`, which match a `&str` arm directly. For an int scrutinee, patterns are integer literals. Mixed/other patterns beyond literals + `_` are out of scope; if a pattern isn't a literal or `_`, emit `compile_error!`.

- [ ] **Step 4: Run test + compile**

Run: `cargo test -p mui-codegen --lib lowers_reactive_match`
Expected: PASS.
Write a tiny `match` example to `/tmp/m.mui` and:
```bash
cargo run -p cforge -- -c -i /tmp/m.mui && rustc --edition 2021 --crate-type bin dist/rust/src/main.rs -o /tmp/m_check 2>&1 | head
```
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/mui-codegen/src/lib.rs
git commit -m "feat(mui-codegen): lower reactive match with literal + wildcard arms"
```

---

## Task 8: Integration verification

**Files:**
- Test: existing examples + suite (no new source unless a gap surfaces).

- [ ] **Step 1: Full workspace suite**

Run: `cargo test`
Expected: all pass (≥131 tests; the new tests add to the count).

- [ ] **Step 2: Compile every MUI/copper example**

```bash
for f in examples/mui/*/*.mui examples/mui/*/*.crm; do
  cargo run -q -p cforge -- -c -i "$f" || { echo "TRANSPILE FAIL: $f"; continue; }
  rustc --edition 2021 --crate-type bin dist/rust/src/main.rs -o /tmp/ex_check 2>/dev/null \
    || rustc --edition 2021 --crate-type lib dist/rust/src/main.rs -o /tmp/ex_check.rlib 2>&1 | head -3
done
```
Expected: each transpiles and compiles (lib fallback only for entry views literally named `main`).

- [ ] **Step 3: Confirm the `for` example is now dynamic**

```bash
cargo run -q -p cforge -- -c -i examples/mui/crm/app.crm
grep -n "for item in __sig_items" dist/rust/src/main.rs
grep -n "on_tick" dist/rust/src/main.rs
```
Expected: both grep hits present (dynamic loop + rebuild tick), and NO `renders one iteration` / `static lowering` comments remain.

- [ ] **Step 4: Commit any fixes**

```bash
git add -A
git commit -m "test(mui-codegen): verify reactive if/for/match across examples"
```

---

## Self-Review (filled by the plan author)

- **Spec coverage:** §1 rebuild engine → Task 4; §2 signal generalization → Task 3; §3 expression evaluator → Task 2; §4 lowering if/for/match → Tasks 5/6/7; §5 match parser fix → Task 1; §6 testing → folded into each task + Task 8. All spec sections mapped.
- **Type consistency:** `SigKind` defined once in `eval.rs`, used by lib.rs; `Seed` methods (`int`/`str`/`list`) consistent between `header()` definition and `declare_signals`/`generate_main` use; `BuiltView` field list consistent between `header()` and the view-fn return line; `emit_structural_subscribe` signature consistent across Tasks 5/6/7.
- **Ordering caveat:** Tasks 3 and 4 share the view-fn preamble (`__seed`, `__keep_str`, `__keep_list`). They are split for review granularity; the implementer may combine their commits. This is called out in Task 3's notes.
- **Verification-required notes:** loader entry name (Task 1), `generate_from_str` test helper name (Tasks 3–7), `mocida_sys` vs a `mocida` raw wrapper for `UIApp_SetChildren` (Task 4), and `reactive_format` signature for loop-var binding (Task 6) must be confirmed against the live source during implementation — each is flagged inline.
