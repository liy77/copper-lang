//! Component AST for MUI (`.mui` / `.crm`).
//!
//! A document is a set of `view` functions. Each view's body is a tree of
//! [`Node`]s — elements (mocida widgets) plus reactive glue (`let signal`,
//! `effect`) and control flow (`if` / `for` / `match`). Embedded Copper —
//! prop expressions, handler bodies, `${...}` interpolation — is parsed into
//! [`copper_syntax::expr`] trees so the same typed AST powers both the dev
//! interpreter/JIT and the release codegen.

use copper_syntax::ast::Span;
use copper_syntax::expr::{Block, Expr};

/// A parsed `.mui` / `.crm` document.
#[derive(Debug, Clone)]
pub struct Document {
    pub views: Vec<View>,
    /// Component imports: `import { Card, Row } from "./widgets.mui"`.
    pub imports: Vec<Import>,
    /// Optional top-level `app { ... }` configuration block.
    pub app: Option<AppConfig>,
    pub errors: Vec<MuiError>,
}

/// What kind of file an `import` points at, decided by the path's extension.
/// MUI imports bring component *views* into scope; Copper/Rust imports bring
/// *logic* (functions, types) that the generated crate links in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// `.mui` / `.crm` — component views, resolved into the registry.
    Mui,
    /// `.crs` — Copper source, transpiled into the generated crate as a module.
    Copper,
    /// `.rs` — raw Rust, included into the generated crate as a module.
    Rust,
}

impl ImportKind {
    /// Classify by file extension (case-insensitive). Unknown/blank extensions
    /// default to [`ImportKind::Mui`] (the historical behaviour).
    pub fn from_path(path: &str) -> Self {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "rs" => ImportKind::Rust,
            "crs" => ImportKind::Copper,
            _ => ImportKind::Mui, // .mui, .crm, or unspecified
        }
    }

    /// True for component (view) imports — the only kind the loader resolves
    /// into the registry.
    pub fn is_mui(self) -> bool {
        matches!(self, ImportKind::Mui)
    }
}

/// `import { Name, Other } from "path"` — pulls views from another MUI file so
/// they can be used as elements (`Card(...)`), or functions/types from a Copper
/// (`.crs`) / Rust (`.rs`) file so they can be used in handlers and expressions.
#[derive(Debug, Clone)]
pub struct Import {
    /// Names brought into scope (view names, or Copper/Rust item names).
    pub names: Vec<String>,
    /// The module path, as written (e.g. `"./card.mui"`). Resolved relative to
    /// the importing file by the loader.
    pub path: String,
    /// What the path points at (decided by its extension).
    pub kind: ImportKind,
    pub span: Span,
}

impl Import {
    /// The Rust module name a Copper/Rust import is materialized under in the
    /// generated crate (`./logic.crs` → `logic`, `../ui/native.rs` → `native`).
    /// Derived from the file stem, sanitized to a valid identifier. Codegen and
    /// cforge must agree on this, so it lives here.
    pub fn module_name(&self) -> String {
        let stem = std::path::Path::new(&self.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported");
        let mut name: String = stem
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        if name.is_empty() {
            name.push_str("imported");
        }
        if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            name.insert(0, '_');
        }
        name
    }
}

/// Top-level `app { name:, id:, title:, width:, height:, background: }` block.
/// Configures the window + bundle identity for a runnable MUI app. All fields
/// optional; an `app.bundle` manifest can supply name/id too (this block wins
/// when both are present). The `entry:` names which view to mount (defaults to
/// the first view).
#[derive(Debug, Clone, Default)]
pub struct AppConfig {
    /// App / bundle display name (also the default window title).
    pub name: Option<String>,
    /// Bundle identifier (e.g. `net.liy77.myapp`).
    pub id: Option<String>,
    /// Window title (falls back to `name`, then the entry view name).
    pub title: Option<String>,
    /// Initial window width / height in logical px.
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Optional min/max window size in logical px (desktop only — clamps how far
    /// the user can resize the window). `None` = unconstrained.
    pub min_width: Option<i32>,
    pub min_height: Option<i32>,
    pub max_width: Option<i32>,
    pub max_height: Option<i32>,
    /// Render tuning (desktop). `renderer` = SDL backend
    /// (`d3d11`/`opengl`/`vulkan`/`d3d12`/`d3d9`/`metal`/`software`/`gpu`);
    /// `msaa` = coverage samples (1/2/4/8); `aa` = pipeline
    /// (`none`/`coverage`/`ssaa2x`/`ssaa4x`/`fxaa`/`taa`); `render_quality` =
    /// preset (`low`/`medium`/`high`/`ultra`); `taa_blend` = TAA history weight.
    pub renderer: Option<String>,
    pub msaa: Option<i32>,
    pub aa: Option<String>,
    pub render_quality: Option<String>,
    pub taa_blend: Option<f32>,
    /// Window background color (`#rrggbb` / `rgba()`), as 0-255 RGBA.
    pub background: Option<(u8, u8, u8, u8)>,
    /// Name of the view to mount as the root (defaults to the first view).
    pub entry: Option<String>,
    /// Optional native host crate (sibling directory) that `cforge run` launches
    /// instead of the generic `mui-dev` — for a `.mui` that needs a real backend
    /// (foreign `.rs`/`.crs` imports, effects, threads). E.g. `host: "copper-installer"`.
    pub host: Option<String>,
    pub span: Span,
}

/// `view Name(params) { body }`.
#[derive(Debug, Clone)]
pub struct View {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Node>,
    pub span: Span,
}

/// A view parameter: `name: Type = default`.
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Option<String>,
    pub default: Option<Expr>,
    pub span: Span,
}

/// A node in a view body / element children list.
#[derive(Debug, Clone)]
pub enum Node {
    Element(Element),
    /// `let count = signal(0)` (state) / `let x = computed { ... }`.
    Let {
        name: String,
        mutable: bool,
        value: Option<Expr>,
        span: Span,
    },
    /// `effect { ... }` — a reactive side effect. `raw` is the captured body
    /// text, interpreted into [`HandlerAction`]s the runtime runs (on mount, and
    /// — for the subset it understands — when referenced signals change).
    Effect {
        body: Block,
        raw: String,
        span: Span,
    },
    /// `if cond { ... } else { ... }` in children position. `cond_raw` is the
    /// captured condition text the runtime/codegen evaluate to pick a branch
    /// (`cond` stays a placeholder until full expression lowering lands).
    If {
        cond: Expr,
        cond_raw: String,
        then: Vec<Node>,
        els: Option<Vec<Node>>,
        span: Span,
    },
    /// `for item in iter { ... }`.
    For {
        pattern: String,
        iter: Expr,
        body: Vec<Node>,
        span: Span,
    },
    /// `match expr { pat => node, ... }`.
    Match {
        scrutinee: Expr,
        arms: Vec<MatchArm>,
        span: Span,
    },
    /// A bare Copper expression in node position (rare; e.g. a computed view).
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    /// Raw pattern source (parsed into copper patterns in a later pass).
    pub pattern: String,
    pub body: Vec<Node>,
    pub span: Span,
}

/// `Widget(positional?, prop: value, ...) { children }`.
#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    /// The optional first positional argument (a label / source / etc.).
    pub positional: Option<Expr>,
    pub props: Vec<Prop>,
    pub children: Vec<Node>,
    /// `key:` lifted out of `props` for reconciliation.
    pub key: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Prop {
    pub name: String,
    pub value: PropValue,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum PropValue {
    /// A lowered Copper expression (numbers, idents, arithmetic, calls, …).
    Expr(Expr),
    /// `{ |a| ... }` / `{ ... }` event handler.
    Handler(Handler),
    /// A MUI literal the Copper grammar doesn't model: a color (`#rrggbb`,
    /// `#rrggbbaa`, `rgba(...)`) or an enum access (`FontStyle.Bold`). Kept as
    /// a small typed value so the static runtime can apply it without an
    /// evaluator. The raw source is preserved for diagnostics / codegen.
    Mui(MuiValue),
}

/// MUI-specific literal values recognised structurally by the parser.
#[derive(Debug, Clone, PartialEq)]
pub enum MuiValue {
    /// `#rrggbb` or `#rrggbbaa` — already split into 0-255 channels + alpha.
    Color { r: u8, g: u8, b: u8, a: u8 },
    /// `Enum.Member`, e.g. `FontStyle.Bold` / `orientation: vertical`.
    Enum { ty: Option<String>, member: String },
}

/// `{ |params| body }` or `{ body }` supplied as an event-handler prop.
#[derive(Debug, Clone)]
pub struct Handler {
    pub params: Vec<String>,
    pub body: Block,
    /// Raw source of the handler body (between the braces, after the optional
    /// `|params|`). Kept verbatim so the runtime/codegen can interpret simple
    /// statements (`count = count + 1`) without a full Copper Block lowering.
    pub raw: String,
}

impl Handler {
    /// Interpret the handler body as a single reactive [`HandlerAction`] over a
    /// state variable, when it matches one of the common forms the M3 runtime
    /// supports. Returns `None` for anything more complex (those stay TODO).
    pub fn action(&self) -> Option<HandlerAction> {
        parse_handler_action(&self.raw)
    }
}

/// A recognised, executable handler effect on a state signal. Covers the
/// counter-style mutations MUI's reactivity needs first; richer bodies are
/// left to a fuller evaluator later.
#[derive(Debug, Clone, PartialEq)]
pub enum HandlerAction {
    /// `name = name + delta` / `name = name - delta` (delta defaults to 1 for
    /// `name++` / `name--`). `delta` is signed: `-1` for decrement.
    AddAssign { name: String, delta: i64 },
    /// `name = <int literal>`.
    SetInt { name: String, value: i64 },
}

/// Parse the raw handler body into a [`HandlerAction`], or `None`.
///
/// Recognised forms (token text is space-joined by the parser):
///   `count = count + 1`  / `count = count - 2`
///   `count += 1`         / `count -= 1`
///   `count++`            / `count--`
///   `count = 0`
fn parse_handler_action(raw: &str) -> Option<HandlerAction> {
    let s = raw.trim();
    // Strip a trailing statement separator the tokenizer may have left.
    let s = s.trim_end_matches([';']).trim();
    let toks: Vec<&str> = s.split_whitespace().collect();

    // `name ++` / `name --`
    if toks.len() == 2 && is_ident(toks[0]) {
        if toks[1] == "++" {
            return Some(HandlerAction::AddAssign {
                name: toks[0].to_string(),
                delta: 1,
            });
        }
        if toks[1] == "--" {
            return Some(HandlerAction::AddAssign {
                name: toks[0].to_string(),
                delta: -1,
            });
        }
    }

    // `name += N` / `name -= N`
    if toks.len() == 3 && is_ident(toks[0]) {
        if let Ok(n) = toks[2].parse::<i64>() {
            if toks[1] == "+=" {
                return Some(HandlerAction::AddAssign {
                    name: toks[0].to_string(),
                    delta: n,
                });
            }
            if toks[1] == "-=" {
                return Some(HandlerAction::AddAssign {
                    name: toks[0].to_string(),
                    delta: -n,
                });
            }
        }
    }

    // `name = ...`
    if toks.len() >= 3 && is_ident(toks[0]) && toks[1] == "=" {
        let rhs = &toks[2..];
        // `name = <int>`
        if rhs.len() == 1 {
            if let Ok(v) = rhs[0].parse::<i64>() {
                return Some(HandlerAction::SetInt {
                    name: toks[0].to_string(),
                    value: v,
                });
            }
        }
        // `name = name + N` / `name = name - N`
        if rhs.len() == 3 && rhs[0] == toks[0] {
            if let Ok(n) = rhs[2].parse::<i64>() {
                if rhs[1] == "+" {
                    return Some(HandlerAction::AddAssign {
                        name: toks[0].to_string(),
                        delta: n,
                    });
                }
                if rhs[1] == "-" {
                    return Some(HandlerAction::AddAssign {
                        name: toks[0].to_string(),
                        delta: -n,
                    });
                }
            }
        }
    }
    None
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse a multi-statement body (an `effect { … }` or a multi-line handler)
/// into the [`HandlerAction`]s it's made of. Statements are split on `;` and
/// newlines; each is parsed independently, and anything not recognised by the
/// current (int-signal) interpreter is skipped.
pub fn parse_actions(raw: &str) -> Vec<HandlerAction> {
    raw.split([';', '\n'])
        .filter_map(|stmt| {
            let s = stmt.trim();
            if s.is_empty() {
                None
            } else {
                parse_handler_action(s)
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct MuiError {
    pub span: Span,
    pub message: String,
}
