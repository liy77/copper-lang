//! Shared style + anchor extraction for `.mui` / `.crm` elements.
//!
//! Both the dev runtime (`mui-runtime`) and the release codegen (`mui-codegen`)
//! read element styling through these helpers, so the two paths can't drift on
//! which props exist or how they're interpreted. Each getter pulls a typed
//! value out of an [`Element`]'s props; `None` means "not set, use the default".

use std::collections::HashMap;

use crate::ast::{Element, MuiValue, Node, Prop, PropValue};
use copper_syntax::expr::{BinOp, Expr, ExprKind, Literal, StrPart};

/// An RGBA color in 0-255 channels (alpha 0-255). The single representation
/// both `#rrggbb[aa]` and `rgba(...)` resolve to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// Alpha as a 0.0–1.0 float (mocida's `Color::rgba` alpha convention).
    pub fn alpha_f32(self) -> f32 {
        self.a as f32 / 255.0
    }
}

/// A drop-shadow descriptor parsed from a CSS-like `shadow:` string
/// (`"0 1px 3px rgba(0,0,0,0.1)"`). Maps directly onto mocida's `Shadow`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSpec {
    /// Horizontal offset in px (positive = right).
    pub dx: f32,
    /// Vertical offset in px (positive = down).
    pub dy: f32,
    /// Blur radius in px (0 = hard edge).
    pub blur: f32,
    /// Pre-blur spread in px.
    pub spread: f32,
    /// Shadow tint (alpha gates intensity).
    pub color: Rgba,
}

impl ShadowSpec {
    /// A sensible material-ish default, used when `shadow:` is set but its
    /// value can't be parsed (e.g. `shadow: true`).
    pub const DEFAULT: Self = Self {
        dx: 0.0,
        dy: 4.0,
        blur: 12.0,
        spread: 0.0,
        color: Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 64,
        },
    };
}

/// Anchor edges for the mocida alignment system. `None` on an axis means
/// "unspecified" (leave the widget's own position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Anchor {
    pub vertical: Option<VAnchor>,
    pub horizontal: Option<HAnchor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAnchor {
    Top,
    Center,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAnchor {
    Left,
    Center,
    Right,
}

impl Anchor {
    /// True when at least one axis is anchored.
    pub fn is_set(self) -> bool {
        self.vertical.is_some() || self.horizontal.is_some()
    }
}

fn find<'a>(el: &'a Element, name: &str) -> Option<&'a Prop> {
    el.props.iter().find(|p| p.name == name)
}

/// Runtime-evaluable env: a snapshot of the live signal table at evaluation
/// time. Values are stringly-typed (matching how signals flow through the
/// mui-runtime); numeric evaluation parses them as f32 and bails to None on
/// a non-numeric value.
#[derive(Debug, Clone, Default)]
pub struct ReactiveEnv {
    pub signals: HashMap<String, String>,
}

/// A numeric prop, reactive to signals when `env` is Some. With `env = None`
/// only literals resolve (preserves the static-only contract of the old API).
pub fn f32_prop_reactive(
    el: &Element,
    name: &str,
    env: Option<&ReactiveEnv>,
) -> Option<f32> {
    let e = match &find(el, name)?.value {
        PropValue::Expr(e) => e,
        _ => return None,
    };
    // Literals are the common static case; resolve them regardless of env so
    // a literal prop never depends on a signal table it doesn't read.
    if let Some(v) = literal_f32(e) {
        return Some(v);
    }
    match env {
        Some(env) => {
            let (v, _deps) = eval_dim_reactive(e, env);
            v
        }
        None => None,
    }
}

/// Back-compat wrapper: static-only numeric prop (literal int/float).
pub fn f32_prop(el: &Element, name: &str) -> Option<f32> {
    f32_prop_reactive(el, name, None)
}

/// The window / screen size and the literal sizes of `id:`-tagged widgets,
/// used to resolve dimension expressions like `Window.width - 520` or
/// `left_panel.width`. The dev runtime fills the metrics from the live window;
/// the codegen fills them from the `app { }` block (a static evaluation).
#[derive(Debug, Clone, Default)]
pub struct DimEnv {
    pub window_w: Option<f32>,
    pub window_h: Option<f32>,
    pub screen_w: Option<f32>,
    pub screen_h: Option<f32>,
    /// `id → (width, height)` literal sizes collected from the view.
    pub ids: HashMap<String, (Option<f32>, Option<f32>)>,
}

/// Resolve `<ns>.<field>` — a window/screen metric (`Window.width`) or another
/// widget's declared size (`left_panel.height`).
pub fn resolve_metric(env: &DimEnv, ns: &str, field: &str) -> Option<f32> {
    match (ns, field) {
        ("Window" | "App", "width") => env.window_w,
        ("Window" | "App", "height") => env.window_h,
        ("Screen", "width") => env.screen_w,
        ("Screen", "height") => env.screen_h,
        (id, "width") => env.ids.get(id).and_then(|d| d.0),
        (id, "height") => env.ids.get(id).and_then(|d| d.1),
        _ => None,
    }
}

/// Evaluate a dimension expression to a constant: numeric literals, `+ - * /`
/// arithmetic, and `Window.width` / `<id>.height` metric references. Returns
/// `None` for anything not statically resolvable (a live signal, say).
pub fn eval_dim(e: &Expr, env: &DimEnv) -> Option<f32> {
    match &e.kind {
        ExprKind::Literal(Literal::Int(i)) => Some(*i as f32),
        ExprKind::Literal(Literal::Float(f)) => Some(*f as f32),
        ExprKind::Member { base, field, .. } => match &base.kind {
            ExprKind::Ident(ns) => resolve_metric(env, ns, field),
            _ => None,
        },
        ExprKind::Binary { op, lhs, rhs } => {
            let a = eval_dim(lhs, env)?;
            let b = eval_dim(rhs, env)?;
            match op {
                BinOp::Add => Some(a + b),
                BinOp::Sub => Some(a - b),
                BinOp::Mul => Some(a * b),
                BinOp::Div if b != 0.0 => Some(a / b),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Reactive variant of [`eval_dim`]. Walks a Copper `Expr` against a snapshot
/// of the live signal table ([`ReactiveEnv`]) and returns the resolved numeric
/// value plus the set of signal names the expression actually read from the
/// env. The mui-runtime uses the dep list to register invalidation listeners
/// (so a change to `is_macos` re-evaluates the prop). Missing signals cause
/// the whole evaluation to return `None` — the prop falls back to its static
/// default until the missing signal is provided. Comparison ops (`==`, `!=`)
/// return `1.0` / `0.0` so they can drive `Ternary` conditions.
pub fn eval_dim_reactive(e: &Expr, env: &ReactiveEnv) -> (Option<f32>, Vec<String>) {
    let mut deps = Vec::new();
    let v = eval_dim_reactive_inner(e, env, &mut deps);
    (v, deps)
}

fn eval_dim_reactive_inner(
    e: &Expr,
    env: &ReactiveEnv,
    deps: &mut Vec<String>,
) -> Option<f32> {
    match &e.kind {
        ExprKind::Literal(Literal::Int(i)) => Some(*i as f32),
        ExprKind::Literal(Literal::Float(f)) => Some(*f as f32),
        ExprKind::Ident(name) => {
            // A bare name that's also a signal in the env is reactive; any
            // other ident (an unqualified function, a constant, a typo) resolves
            // to None rather than silently treating it as 0.
            let v = env.signals.get(name).and_then(|s| s.parse::<f32>().ok());
            if v.is_some() {
                deps.push(name.clone());
            }
            v
        }
        ExprKind::Ternary { cond, then, els } => {
            // Treat the condition as a truthy number: `0.0` → else, anything
            // else → then. (The comparison arms below produce `0.0`/`1.0`.)
            let c = eval_dim_reactive_inner(cond, env, deps)?;
            let pick = if c == 0.0 { els } else { then };
            eval_dim_reactive_inner(pick, env, deps)
        }
        ExprKind::Binary { op, lhs, rhs } => {
            // Allow either operand to come from a pure-literal expr so a
            // signal on one side of `==` and a literal string on the other
            // (`is_macos == "1"`) both resolve. The literal helper also
            // extracts numeric content from non-interpolated string literals.
            let a = eval_dim_reactive_inner(lhs, env, deps)
                .or_else(|| literal_f32_or_str(lhs))?;
            let b = eval_dim_reactive_inner(rhs, env, deps)
                .or_else(|| literal_f32_or_str(rhs))?;
            match op {
                BinOp::Add => Some(a + b),
                BinOp::Sub => Some(a - b),
                BinOp::Mul => Some(a * b),
                BinOp::Div if b != 0.0 => Some(a / b),
                // Equality yields 1.0 / 0.0 so it can be the condition of a
                // Ternary above. Strict equality only — `NaN != NaN` would
                // otherwise surprise a config author.
                BinOp::Eq => Some(if a == b { 1.0 } else { 0.0 }),
                BinOp::Ne => Some(if a != b { 1.0 } else { 0.0 }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Extract an `f32` from a pure-literal `Expr` (Int, Float, or a non-
/// interpolated `Str` whose body parses as a number). Used to coerce one
/// operand of a binary op — typically the right-hand side of `==` — when the
/// other side is a signal (and therefore reactive).
fn literal_f32_or_str(e: &Expr) -> Option<f32> {
    match &e.kind {
        ExprKind::Literal(Literal::Int(i)) => Some(*i as f32),
        ExprKind::Literal(Literal::Float(f)) => Some(*f as f32),
        ExprKind::Literal(Literal::Str(s)) => s.parts.iter().find_map(|p| match p {
            StrPart::Lit(t) => t.parse().ok(),
            StrPart::Expr(_) => None,
        }),
        _ => None,
    }
}

/// A dimension prop (`width` / `height` / `x` / `y`), honouring metric refs and
/// arithmetic — not just literals. `Window.width` with no arithmetic parses as
/// a `Type.Member` enum, so that case is resolved too.
pub fn dim_prop(el: &Element, name: &str, env: &DimEnv) -> Option<f32> {
    match &find(el, name)?.value {
        PropValue::Expr(e) => eval_dim(e, env),
        PropValue::Mui(MuiValue::Enum {
            ty: Some(ty),
            member,
        }) => resolve_metric(env, ty, member),
        _ => None,
    }
}

/// Reactive variant of [`dim_prop`]. Walks a Copper `Expr` against a
/// [`ReactiveEnv`] snapshot and returns the resolved numeric value. Window/
/// screen metric refs (`Window.width`, etc.) are intentionally out of scope
/// here — use the static [`dim_prop`] with a `DimEnv` for those. The
/// mui-runtime needs only the reactive signal path; metric refs are a
/// compile-time codegen concern, not a live-signal concern.
pub fn dim_prop_reactive(el: &Element, name: &str, env: &ReactiveEnv) -> Option<f32> {
    let e = match &find(el, name)?.value {
        PropValue::Expr(e) => e,
        _ => return None,
    };
    eval_dim_reactive(e, env).0
}

/// The `id:` of an element as a name — a bare ident (`id: left_panel`) or a
/// string (`id: "left_panel"`). `None` when there's no `id`.
pub fn id_of(el: &Element) -> Option<String> {
    match &find(el, "id")?.value {
        PropValue::Expr(e) => match &e.kind {
            ExprKind::Ident(n) => Some(n.clone()),
            ExprKind::Literal(Literal::Str(t)) => t.parts.iter().find_map(|p| match p {
                StrPart::Lit(s) => Some(s.clone()),
                _ => None,
            }),
            _ => None,
        },
        _ => None,
    }
}

/// Collect every `id:`-tagged widget's literal `width`/`height` (descending into
/// `if`/`for` bodies and children), so a sibling can size against it.
pub fn collect_id_dims(nodes: &[Node]) -> HashMap<String, (Option<f32>, Option<f32>)> {
    fn walk(nodes: &[Node], out: &mut HashMap<String, (Option<f32>, Option<f32>)>) {
        for node in nodes {
            match node {
                Node::Element(el) => {
                    if let Some(id) = id_of(el) {
                        out.insert(id, (f32_prop(el, "width"), f32_prop(el, "height")));
                    }
                    walk(&el.children, out);
                }
                Node::If { then, els, .. } => {
                    walk(then, out);
                    if let Some(e) = els {
                        walk(e, out);
                    }
                }
                Node::For { body, .. } => walk(body, out),
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    walk(nodes, &mut out);
    out
}

fn literal_f32(e: &Expr) -> Option<f32> {
    match &e.kind {
        ExprKind::Literal(Literal::Int(i)) => Some(*i as f32),
        ExprKind::Literal(Literal::Float(f)) => Some(*f as f32),
        _ => None,
    }
}

/// A boolean prop (`value: true`, `checked: false`, `indeterminate: true`).
pub fn bool_prop(el: &Element, name: &str) -> Option<bool> {
    match &find(el, name)?.value {
        PropValue::Expr(e) => match &e.kind {
            ExprKind::Literal(Literal::Bool(b)) => Some(*b),
            _ => None,
        },
        _ => None,
    }
}

/// A color prop (`color:`, `background:`/`bg:`, `borderColor:`, `tint:` …).
/// Resolves `#rrggbb` / `#rrggbbaa` (and, once the parser emits them, `rgba()`)
/// to [`Rgba`].
pub fn color_prop(el: &Element, name: &str) -> Option<Rgba> {
    match &find(el, name)?.value {
        PropValue::Mui(MuiValue::Color { r, g, b, a }) => Some(Rgba {
            r: *r,
            g: *g,
            b: *b,
            a: *a,
        }),
        _ => None,
    }
}

/// The fill/background color for a widget. Accepts `background`, `bg`, or
/// `color` (in that order) so authors can use whichever reads naturally for
/// the element (`color` on Text, `background` on a panel/Button).
pub fn background(el: &Element) -> Option<Rgba> {
    color_prop(el, "background")
        .or_else(|| color_prop(el, "bg"))
        .or_else(|| color_prop(el, "color"))
}

/// The background fill *without* the `color` fallback — for widgets where
/// `color` means the text/glyph color, not the fill (text inputs, labels).
/// Accepts only `background` / `bg`.
pub fn fill_only(el: &Element) -> Option<Rgba> {
    color_prop(el, "background").or_else(|| color_prop(el, "bg"))
}

// Font-style bit flags, mirroring mocida's `FontStyle` enum so the runtime and
// codegen build the same value from MUI typography props.
/// Bold weight flag.
pub const FONT_BOLD: i32 = 1 << 0;
/// Italic / oblique flag.
pub const FONT_ITALIC: i32 = 1 << 1;
/// Underline flag.
pub const FONT_UNDERLINE: i32 = 1 << 2;
/// Strikethrough flag.
pub const FONT_STRIKETHROUGH: i32 = 1 << 3;

fn add_style_token(bits: &mut i32, token: &str) {
    match token.trim().to_ascii_lowercase().as_str() {
        "bold" | "700" => *bits |= FONT_BOLD,
        "italic" | "oblique" => *bits |= FONT_ITALIC,
        "underline" => *bits |= FONT_UNDERLINE,
        "strikethrough" | "strike" | "linethrough" => *bits |= FONT_STRIKETHROUGH,
        _ => {}
    }
}

/// Collect font-style flags from an element's typography props, as a bitmask of
/// `FONT_*`. Recognises `weight: bold`, `fontStyle: italic` (a single member or
/// a `"bold italic"` / `bold|italic` combo string), and the boolean shorthands
/// `bold:` / `italic:` / `underline:` / `strikethrough:`. Returns `0` when no
/// styling prop is present (the caller can then skip emitting a `font_style`
/// call). Mirrored bit-for-bit by both the runtime and the codegen.
pub fn font_style_bits(el: &Element) -> i32 {
    let mut bits = 0;
    if let Some(w) = enum_member(el, "weight") {
        add_style_token(&mut bits, &w);
    }
    if let Some(s) = enum_member(el, "fontStyle") {
        add_style_token(&mut bits, &s);
    }
    if let Some(s) = string_prop(el, "fontStyle") {
        for tok in s.split([' ', '|', ',']) {
            add_style_token(&mut bits, tok);
        }
    }
    for (name, flag) in [
        ("bold", FONT_BOLD),
        ("italic", FONT_ITALIC),
        ("underline", FONT_UNDERLINE),
        ("strikethrough", FONT_STRIKETHROUGH),
    ] {
        if bool_prop(el, name) == Some(true) {
            bits |= flag;
        }
    }
    bits
}

/// The font-family *name* from `font:` / `fontFamily:` (e.g. `"Arial"`). The
/// caller resolves it to an on-disk path via `mocida::text::get_font` before
/// handing it to a widget's `font_family` setter.
pub fn font_family(el: &Element) -> Option<String> {
    string_prop(el, "fontFamily").or_else(|| string_prop(el, "font"))
}

/// A numeric array prop value, e.g. `padding: [10, 20]`. Returns the f32 values
/// when EVERY element is a number literal; `None` otherwise.
fn array_f32(el: &Element, name: &str) -> Option<Vec<f32>> {
    let PropValue::Expr(e) = &find(el, name)?.value else {
        return None;
    };
    let ExprKind::Array(items) = &e.kind else {
        return None;
    };
    let vals: Vec<f32> = items.iter().filter_map(literal_f32).collect();
    if !items.is_empty() && vals.len() == items.len() {
        Some(vals)
    } else {
        None
    }
}

/// Resolve box spacing (`padding` / `margin`) into `(left, top, right, bottom)`.
/// Accepts, with later forms overriding earlier ones:
///   - `base: N`                      — all four sides = N
///   - `base: [a]`                    — all four = a
///   - `base: [v, h]`                 — top/bottom = v, left/right = h (CSS 2-val)
///   - `base: [t, h, b]`              — top, left/right = h, bottom (CSS 3-val)
///   - `base: [t, r, b, l]`           — CSS clockwise
///   - `baseX: N` / `baseY: N`        — horizontal (l,r) / vertical (t,b)
///   - `baseTop/Right/Bottom/Left: N` — per-side
///
/// `base` is `"padding"` or `"margin"`. Returns `None` when nothing is set.
/// `env = None` falls back to static-only resolution (literals + array f32),
/// preserving the old contract for callers that don't have a [`ReactiveEnv`]
/// in scope.
pub fn box_spacing_reactive(
    el: &Element,
    base: &str,
    env: Option<&ReactiveEnv>,
) -> Option<(f32, f32, f32, f32)> {
    let mut set = false;
    let (mut l, mut t, mut r, mut b) = (0.0_f32, 0.0, 0.0, 0.0);

    if let Some(a) = array_f32(el, base) {
        set = true;
        match a.as_slice() {
            [x] => (l, t, r, b) = (*x, *x, *x, *x),
            [v, h] => (l, t, r, b) = (*h, *v, *h, *v),
            [tt, h, bb] => (l, t, r, b) = (*h, *tt, *h, *bb),
            [tt, rr, bb, ll, ..] => (l, t, r, b) = (*ll, *tt, *rr, *bb),
            [] => set = false,
        }
    } else if let Some(n) = f32_prop_reactive(el, base, env) {
        set = true;
        (l, t, r, b) = (n, n, n, n);
    }
    if let Some(x) = f32_prop_reactive(el, &format!("{base}X"), env) {
        set = true;
        l = x;
        r = x;
    }
    if let Some(y) = f32_prop_reactive(el, &format!("{base}Y"), env) {
        set = true;
        t = y;
        b = y;
    }
    if let Some(v) = f32_prop_reactive(el, &format!("{base}Top"), env) {
        set = true;
        t = v;
    }
    if let Some(v) = f32_prop_reactive(el, &format!("{base}Right"), env) {
        set = true;
        r = v;
    }
    if let Some(v) = f32_prop_reactive(el, &format!("{base}Bottom"), env) {
        set = true;
        b = v;
    }
    if let Some(v) = f32_prop_reactive(el, &format!("{base}Left"), env) {
        set = true;
        l = v;
    }

    if set {
        Some((l, t, r, b))
    } else {
        None
    }
}

/// Back-compat wrapper: static-only box spacing resolution (no signal env).
pub fn box_spacing(el: &Element, base: &str) -> Option<(f32, f32, f32, f32)> {
    box_spacing_reactive(el, base, None)
}

/// A string-literal prop value. Copper lowers a bare `"text"` either to a
/// `Literal::Str` or to a `format("text")` call, so both forms are accepted.
/// Interpolated parts (`${expr}`) are dropped — use the runtime's text
/// rendering for those; this is for static config (`src:`, `shadow:` …).
pub fn string_prop(el: &Element, name: &str) -> Option<String> {
    let PropValue::Expr(e) = &find(el, name)?.value else {
        return None;
    };
    str_from_expr(e)
}

fn str_from_expr(e: &Expr) -> Option<String> {
    if let ExprKind::Literal(Literal::Str(t)) = &e.kind {
        return Some(join_lit_parts(&t.parts));
    }
    // `"x"` with no interpolation is lowered to `format("x")`.
    if let ExprKind::Call { callee, args, .. } = &e.kind {
        if matches!(&callee.kind, ExprKind::Ident(n) if n == "format" || n == "format!") {
            if let Some(ExprKind::Literal(Literal::Str(t))) = args.first().map(|a| &a.kind) {
                return Some(join_lit_parts(&t.parts));
            }
        }
    }
    None
}

fn join_lit_parts(parts: &[StrPart]) -> String {
    parts
        .iter()
        .filter_map(|p| match p {
            StrPart::Lit(s) => Some(s.clone()),
            StrPart::Expr(_) => None,
        })
        .collect()
}

/// Parse the `shadow:` prop. Accepts a CSS-like string
/// `"<dx> <dy> <blur> [spread] <color>"` (px units optional), e.g.
/// `"0 1px 3px rgba(0,0,0,0.1)"`. A bare `shadow: true` (or any value whose
/// string can't be parsed) yields [`ShadowSpec::DEFAULT`]. Returns `None` when
/// there's no `shadow` prop, or it's explicitly `false`/`none`.
pub fn shadow(el: &Element) -> Option<ShadowSpec> {
    let prop = find(el, "shadow")?;
    // Explicit off switches.
    if let PropValue::Expr(e) = &prop.value {
        if matches!(&e.kind, ExprKind::Literal(Literal::Bool(false))) {
            return None;
        }
        if matches!(&e.kind, ExprKind::Ident(n) if n.eq_ignore_ascii_case("none")) {
            return None;
        }
        if matches!(&e.kind, ExprKind::Literal(Literal::Bool(true))) {
            return Some(ShadowSpec::DEFAULT);
        }
    }
    match string_prop(el, "shadow") {
        Some(s) => Some(parse_css_shadow(&s).unwrap_or(ShadowSpec::DEFAULT)),
        None => Some(ShadowSpec::DEFAULT),
    }
}

/// Parse a `"<dx> <dy> <blur> [spread] <color>"` shadow string. Numbers may
/// carry a `px` suffix; the color is any `rgb()/rgba()/#hex` token. Missing
/// numeric fields default to 0; a missing color defaults to 25% black.
pub fn parse_css_shadow(s: &str) -> Option<ShadowSpec> {
    let mut nums: Vec<f32> = Vec::new();
    let mut color: Option<Rgba> = None;
    for tok in s.split_whitespace() {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(c) = parse_css_color(t) {
            color = Some(c);
            continue;
        }
        let cleaned = t.trim_end_matches("px");
        if let Ok(n) = cleaned.parse::<f32>() {
            nums.push(n);
        }
    }
    if nums.is_empty() && color.is_none() {
        return None;
    }
    Some(ShadowSpec {
        dx: nums.first().copied().unwrap_or(0.0),
        dy: nums.get(1).copied().unwrap_or(0.0),
        blur: nums.get(2).copied().unwrap_or(0.0),
        spread: nums.get(3).copied().unwrap_or(0.0),
        color: color.unwrap_or(Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 64,
        }),
    })
}

/// Parse a CSS color token: `#rrggbb`, `#rrggbbaa`, `rgb(r,g,b)`, or
/// `rgba(r,g,b,a)` (alpha as 0–1 float or 0–255). Whitespace-insensitive.
pub fn parse_css_color(tok: &str) -> Option<Rgba> {
    let t = tok.trim();
    if let Some(hex) = t.strip_prefix('#') {
        return parse_hex(hex);
    }
    let lower = t.to_ascii_lowercase();
    let inner = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
    if parts.len() < 3 {
        return None;
    }
    let r = parts[0].parse::<f32>().ok()?.clamp(0.0, 255.0) as u8;
    let g = parts[1].parse::<f32>().ok()?.clamp(0.0, 255.0) as u8;
    let b = parts[2].parse::<f32>().ok()?.clamp(0.0, 255.0) as u8;
    let a = if parts.len() >= 4 {
        let raw = parts[3].parse::<f32>().ok()?;
        if raw <= 1.0 {
            (raw * 255.0).round().clamp(0.0, 255.0) as u8
        } else {
            raw.clamp(0.0, 255.0) as u8
        }
    } else {
        255
    };
    Some(Rgba { r, g, b, a })
}

fn parse_hex(hex: &str) -> Option<Rgba> {
    let h = hex.trim();
    let bytes = |s: &str| u8::from_str_radix(s, 16).ok();
    match h.len() {
        6 => Some(Rgba {
            r: bytes(&h[0..2])?,
            g: bytes(&h[2..4])?,
            b: bytes(&h[4..6])?,
            a: 255,
        }),
        8 => Some(Rgba {
            r: bytes(&h[0..2])?,
            g: bytes(&h[2..4])?,
            b: bytes(&h[4..6])?,
            a: bytes(&h[6..8])?,
        }),
        _ => None,
    }
}

/// The enum member of a prop, lowercased — either dotted `Type.Member`
/// (`cursor: Cursor.Pointer`) or a bare ident (`anchor: center`).
pub fn enum_member(el: &Element, name: &str) -> Option<String> {
    match &find(el, name)?.value {
        PropValue::Mui(MuiValue::Enum { member, .. }) => Some(member.to_lowercase()),
        PropValue::Expr(e) => match &e.kind {
            ExprKind::Ident(n) => Some(n.to_lowercase()),
            _ => None,
        },
        _ => None,
    }
}

/// Parse the `anchor:` prop. Accepts a single token (`center`, `top`,
/// `bottomRight`, `topLeft`, …) or a `vh` style — we recognise the common
/// named combinations plus single-axis values. Examples:
///   `anchor: center`        → both centered
///   `anchor: top`           → vertical top, horizontal unspecified
///   `anchor: topLeft`       → top + left
///   `anchor: bottomRight`   → bottom + right
pub fn anchor(el: &Element) -> Anchor {
    let Some(s) = enum_member(el, "anchor") else {
        return Anchor::default();
    };
    parse_anchor(&s)
}

/// Parse an anchor keyword into vertical/horizontal edges. Case-insensitive;
/// `center` alone centers both axes. Combined forms join a vertical word
/// (`top`/`bottom`/`center`) with a horizontal one (`left`/`right`/`center`).
pub fn parse_anchor(s: &str) -> Anchor {
    let s = s.to_lowercase();
    match s.as_str() {
        "center" | "middle" => {
            return Anchor {
                vertical: Some(VAnchor::Center),
                horizontal: Some(HAnchor::Center),
            }
        }
        "top" => {
            return Anchor {
                vertical: Some(VAnchor::Top),
                horizontal: None,
            }
        }
        "bottom" => {
            return Anchor {
                vertical: Some(VAnchor::Bottom),
                horizontal: None,
            }
        }
        "left" => {
            return Anchor {
                vertical: None,
                horizontal: Some(HAnchor::Left),
            }
        }
        "right" => {
            return Anchor {
                vertical: None,
                horizontal: Some(HAnchor::Right),
            }
        }
        _ => {}
    }
    // Combined `topleft` / `bottomright` / `centerright` / … — scan for the
    // vertical word then the horizontal word.
    let vertical = if s.contains("top") {
        Some(VAnchor::Top)
    } else if s.contains("bottom") {
        Some(VAnchor::Bottom)
    } else if s.contains("center") || s.contains("middle") {
        Some(VAnchor::Center)
    } else {
        None
    };
    let horizontal = if s.contains("left") {
        Some(HAnchor::Left)
    } else if s.contains("right") {
        Some(HAnchor::Right)
    } else if s.ends_with("center") || s.ends_with("middle") {
        Some(HAnchor::Center)
    } else {
        None
    };
    Anchor {
        vertical,
        horizontal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_parse() {
        assert_eq!(
            parse_anchor("center"),
            Anchor {
                vertical: Some(VAnchor::Center),
                horizontal: Some(HAnchor::Center)
            }
        );
        assert_eq!(
            parse_anchor("topLeft"),
            Anchor {
                vertical: Some(VAnchor::Top),
                horizontal: Some(HAnchor::Left)
            }
        );
        assert_eq!(
            parse_anchor("bottomRight"),
            Anchor {
                vertical: Some(VAnchor::Bottom),
                horizontal: Some(HAnchor::Right)
            }
        );
        assert_eq!(parse_anchor("top").vertical, Some(VAnchor::Top));
        assert_eq!(parse_anchor("top").horizontal, None);
    }

    #[test]
    fn css_color_forms() {
        assert_eq!(
            parse_css_color("#ff8000"),
            Some(Rgba {
                r: 255,
                g: 128,
                b: 0,
                a: 255
            })
        );
        assert_eq!(
            parse_css_color("#11223344"),
            Some(Rgba {
                r: 0x11,
                g: 0x22,
                b: 0x33,
                a: 0x44
            })
        );
        assert_eq!(
            parse_css_color("rgb(10, 20, 30)"),
            Some(Rgba {
                r: 10,
                g: 20,
                b: 30,
                a: 255
            })
        );
        // alpha as a 0–1 float (CSS convention).
        assert_eq!(
            parse_css_color("rgba(0,0,0,0.5)"),
            Some(Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 128
            })
        );
        assert_eq!(parse_css_color("nope"), None);
    }

    #[test]
    fn css_shadow_offsets_blur_and_color() {
        let s = parse_css_shadow("0 1px 3px rgba(0,0,0,0.1)").unwrap();
        assert_eq!(s.dx, 0.0);
        assert_eq!(s.dy, 1.0);
        assert_eq!(s.blur, 3.0);
        assert_eq!(
            s.color,
            Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 26
            }
        );
    }

    fn el(src: &str) -> Element {
        // Wrap the element in a view so the parser produces an Element node.
        let doc = crate::parse(&format!("view V() {{ {src} }}"));
        match doc
            .views
            .into_iter()
            .next()
            .and_then(|v| v.body.into_iter().next())
        {
            Some(crate::ast::Node::Element(e)) => e,
            _ => panic!("expected an element"),
        }
    }

    #[test]
    fn font_style_from_weight_and_flags() {
        // `weight: bold` → BOLD.
        assert_eq!(font_style_bits(&el("Text(\"x\", weight: bold)")), FONT_BOLD);
        // boolean shorthands combine.
        assert_eq!(
            font_style_bits(&el("Text(\"x\", italic: true, underline: true)")),
            FONT_ITALIC | FONT_UNDERLINE
        );
        // a combined fontStyle string.
        assert_eq!(
            font_style_bits(&el("Text(\"x\", fontStyle: \"bold italic\")")),
            FONT_BOLD | FONT_ITALIC
        );
        // nothing set → 0.
        assert_eq!(font_style_bits(&el("Text(\"x\")")), 0);
    }

    #[test]
    fn font_family_reads_font_or_fontfamily() {
        assert_eq!(
            font_family(&el("Text(\"x\", font: \"Arial\")")).as_deref(),
            Some("Arial")
        );
        assert_eq!(
            font_family(&el("Text(\"x\", fontFamily: \"Inter\")")).as_deref(),
            Some("Inter")
        );
        assert_eq!(font_family(&el("Text(\"x\")")), None);
    }
}
