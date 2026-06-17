# Reactive `if`/`for`/`match` in the MUI codegen — design

- **Date:** 2026-06-17
- **Repo:** `copper-lang` (`crates/mui-codegen`, `crates/mui-syntax`)
- **Status:** approved design, pre-implementation

## Problem

The MUI codegen (`crates/mui-codegen`) lowers a declarative widget tree into
standalone Rust that links against `mocida-rs`. Today the generated code builds
the widget tree **once** and only updates *reactive props* (e.g. a `${count}`
text label) in place, via raw-pointer subscriptions on `Signal<i32>`.

Structural control flow in node position — `if`/`for`/`match` that add, remove,
or choose **which widgets exist** — is parsed but never executed. The three
placeholders in `crates/mui-codegen/src/lib.rs:435-447`:

- `Node::If` always renders the `then` branch; the condition is never evaluated,
  the `else` branch is dropped.
- `Node::For` renders exactly one iteration of the body; the iterable is never
  iterated.
- `Node::Match` emits a comment only; arms are discarded (the parser itself,
  `crates/mui-syntax/src/lib.rs:946`, returns `arms: Vec::new()`).

The real target use case — `for item in items` where `items` is a signal-backed
list that grows at runtime — produces a single static widget instead of a live
list.

## Goal

Make `if`/`for`/`match` in node position **reactive**: when a signal read by the
control-flow expression changes, the affected widgets are rebuilt to match the
new state. Mirror the model already proven in `mocida-rs/mui-runtime` (the
interpreter that drives OndaEngine), ported into the codegen as emitted Rust.

## Non-goals (tracked as follow-ups)

- General Rust generics / turbofish in Copper syntax (`Rc<RefCell<Signal<T>>>`,
  `foo::<T>()`, `app.add_event::<E>()`). The user's north-star — "everything you
  can do in Rust must be doable in Copper" — is bigger than this feature and gets
  its own spec.
- Arbitrary `Signal<Struct>` of user types. This design generalizes signals only
  to the types `mui-runtime` already supports (`Int`, `Str`, `List<Str>`).
- In-place region rebuild (mutating just the affected container via
  `UIChildren_Clear/Add/Relayout`). Rejected for now: no Rust wrapper exists,
  it spans two repos, and it carries use-after-free risk. Whole-view rebuild is
  chosen instead (see Approach).
- `match` patterns with binds/destructuring (only literal patterns and `_`).

## Approach: whole-view rebuild (ported from `mui-runtime`)

Chosen over in-place region mutation (Approach B) and a hybrid (Approach C).
Rationale: it is identical to the already-validated `mui-runtime` model, is safe
(a full rebuild drops every old subscription, so no dangling widget pointers),
and uses only mocida APIs that already exist (`App::on_tick`, `App::set_children`
/ `UIApp_SetChildren`). The accepted cost is a one-frame flicker and loss of
input focus/caret/scroll inside the rebuilt view — the same documented tradeoff
as the runtime (CLAUDE.md §3).

### Feasibility facts (verified against the codebases)

- `App::on_tick(FnMut() + 'static)` fires once per frame before events/render
  (`mocida-rs/mocida/src/app.rs:412`; C side `UIApp_OnTick`, `app.h`).
- `App::set_children` is just `UIApp_SetChildren(self.ptr, raw)`
  (`mocida-rs/mocida/src/app.rs:363`). `App::as_ptr() -> *mut sys::UIApp`
  (`app.rs:154`) exposes the raw pointer, so the tick closure can call
  `sys::UIApp_SetChildren(app_ptr, children_raw)` directly and sidestep the
  `&mut app` borrow it cannot hold.
- The C `UIApp_SetChildren` destroys the previously installed tree atomically;
  dropping the old `BuiltView` drops its `_subs`, so no subscription outlives the
  widget it targets — no use-after-free.
- `mui-runtime` does exactly this: structural signals flip a `dirty:
  Rc<Cell<bool>>`; the host polls it each frame, snapshots signal values, rebuilds
  the view with that snapshot as a seed, and swaps the tree
  (`mocida-rs/mui-runtime/src/lib.rs:109-129, 530-536, 601-635`).

## Design

### 1. Rebuild engine

The generated view function gains a seed and a shared dirty flag:

```rust
fn view_App(title: String, __seed: &Seed, __dirty: Rc<Cell<bool>>)
    -> MuiResult<BuiltView> { ... }
```

- `Seed = HashMap<String, SeedVal>`, `SeedVal ∈ { Int(i32), Str(String),
  List(Vec<String>) }`. `declare_signals` initializes each signal from the seed
  when present, else from its literal default. A rebuild therefore **preserves
  live state** (counters, typed-in text, list contents).
- Signals read by a control-flow expression ("structural" signals) additionally
  subscribe `move |_| __dirty.set(true)`. They are recreated on every rebuild, so
  the closures never capture a freed widget.
- `main()`:

  ```rust
  let dirty = Rc::new(Cell::new(false));
  let mut seed = Seed::new();
  let mut view = view_App(arg, &seed, dirty.clone())?;
  let app_ptr = app.as_ptr();
  app.set_children(view.children);            // first mount
  app.on_tick(move || {
      if dirty.replace(false) {
          snapshot(&view, &mut seed);         // current signal values -> seed
          if let Ok(v) = view_App(arg, &seed, dirty.clone()) {
              let raw = v.children.into_raw();
              view = v;                        // drop old BuiltView -> drops old _subs
              unsafe { sys::UIApp_SetChildren(app_ptr, raw); }
          }
      }
  });
  app.show().run();
  ```

`snapshot` reads each live signal (`.borrow().get()` / list clone) into the seed
map keyed by signal name.

### 2. Signal generalization (beyond `i32`)

`SignalScope` currently maps `name -> var` for `Signal<i32>` only. Extend to
`name -> (var, SigKind)` with `SigKind ∈ { Int, Str, List }`, matching the
runtime's ceiling:

- `declare_signals` infers the kind from the initializer: int literal → `Int`;
  string / `"${...}"` literal → `Str`; `[]` / array literal → `List` (backed by
  `Signal<Vec<String>>` / `Rc<RefCell<Vec<String>>>` as in the runtime).
- `HandlerAction` gains `SetStr`, `SetList`, `PushList`, `ClearList` — enough for
  the `for item in items` examples (`items = v`, push). Arbitrary `Signal<Struct>`
  is out of scope.

### 3. Expression evaluator (the "reactive evaluator" port)

A small evaluator over Copper's `Expr`, run at build time inside the view fn:

- Input: `cond_raw` / `iter` / `scrutinee`, parsed via
  `copper_syntax::expr::parse_expr`.
- Resolves against the current env: live signal values (`.borrow().get()`),
  view params, literals.
- Produces a `bool` (if), a `Vec<item>` (for), or a comparable value (match).
- Operator coverage: `== != < > <= >=`, `&& || !`, signal/param refs, literals,
  basic list `.len()` and indexing. Anything outside this set is a **clear
  codegen-time error**, never a silent wrong render.

### 4. Lowering in `emit_node`

Replace the three placeholders (`lib.rs:435-447`):

- **`if cond { then } else { els }`**: evaluate `cond` at build; emit `then` or
  `els`. Mark `cond`'s signals structural (subscribe `dirty`).
- **`for item in iter { body }`**: evaluate `iter` to a list; emit a real Rust
  loop, each iteration building `body` with `item` bound into the reactive env as
  a string. Mark `iter`'s signal structural.
- **`match scrutinee { arms }`**: evaluate `scrutinee`, select the first matching
  arm (literal equality or `_` wildcard), build that arm's body. Mark the
  scrutinee's signal structural.

### 5. `match` parser fix

`crates/mui-syntax/src/lib.rs:946` (`parse_match_node`) currently `skip_balanced`s
the body and returns `arms: Vec::new()`. Replace with real arm parsing:
`pattern => { nodes }` (or `pattern => node`), filling `MatchArm { pattern, body }`
and the real `scrutinee`. Pattern scope: literals and `_` (binds/destructuring are
a follow-up).

### 6. Testing

- `mui-codegen` unit tests: snapshot the generated Rust for `if`/`for`/`match`
  (presence of the dirty flag, seed param, structural `subscribe`, and the
  `on_tick`/`set_children` swap).
- Compile the generated output (`rustc --edition 2021 --crate-type bin`) for
  `examples/mui/condtest/condtest.mui` and `examples/mui/crm/app.crm`.
- Reactive vector: regenerate `app.crm` and assert the Rust contains the dynamic
  loop + `on_tick` rebuild, not the single static iteration.
- The existing suite (`cargo test`, currently 131 tests, 0 failures) stays green.

## Risks

- **Flicker / lost input focus on rebuild** — inherent to whole-view rebuild;
  accepted, documented, and mitigated long-term by the deferred in-place Approach
  B.
- **Borrow/lifetime of the tick closure** — handled by capturing the raw
  `*mut UIApp` and calling `sys::UIApp_SetChildren` directly.
- **Seed/snapshot drift** — a signal present in one build but not the next must
  not crash the snapshot; `snapshot` only reads signals that currently exist.
- **Expression coverage gaps** — unsupported operators error at codegen time
  rather than miscompiling.

## Out of scope (explicit)

General generics/turbofish, `Signal<Struct>`, `add_event::<E>()`, in-place region
rebuild (Approach B), and `match` binds/destructuring.
