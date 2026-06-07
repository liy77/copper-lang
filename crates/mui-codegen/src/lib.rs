//! Release codegen for MUI (`.mui` / `.crm`) — **M5** in
//! `mocida/mui/ARCHITECTURE.md`.
//!
//! Lowers the component AST ([`mui_syntax`]) into **Rust source** that builds
//! the mocida widget tree via `mocida-rs` (the same builders the dev runtime
//! calls, but emitted as readable code instead of walked in memory). The
//! output is a self-contained `main.rs`: one `fn <view>() -> Result<Children>`
//! per `view`, plus a `main()` that opens a window with the entry view.
//!
//! This crate is **pure AST→text**: it has no dependency on `mocida` or the
//! runtime, so it builds anywhere (portable CI) and cforge can call it without
//! dragging in the C library. The generated code is what *does* depend on
//! `mocida` — and it's meant to be read, reviewed, and `cargo build`-ed.
//!
//! What's covered (matches the static runtime surface): `Stack`, `Text`,
//! positional labels, `size:` / `color:` / `orientation:` / `gap:` /
//! `padding:` props, `${...}` interpolation with view-param defaults and
//! `signal(...)` initial values. Handlers and live reactivity are emitted as
//! TODOs for now (that's the M3/M4 evaluator work, ported into codegen later).

use copper_syntax::expr::{BinOp, Expr, ExprKind, Literal, StrPart, StrTemplate};
use mui_syntax::ast::{Document, Element, HandlerAction, Node, PropValue, View};
use mui_syntax::loader::Registry;
use mui_syntax::style::{self, Anchor, HAnchor, Rgba, ShadowSpec, VAnchor};

mod emit;
use emit::Emitter;

/// Generate a complete Rust program (`main.rs`) from a parsed MUI document.
/// The entry view (the `app { entry: }` one, else the first) is mounted by
/// `main()`, configured from the document's optional `app { }` block.
/// `fallback_title` is used when no `app`/title is given (the file stem).
pub fn generate_program(doc: &Document, fallback_title: &str) -> String {
    generate_program_with(doc, &Registry::new(), fallback_title, None)
}

/// Describes a bundle to embed in the binary (`cforge -c -b`): the files to
/// `include_bytes!` (paths relative to the generated crate root, the first
/// being `app.bundle`) and a stable temp-subdir name to extract them to.
pub struct EmbedSpec {
    pub files: Vec<String>,
    pub dir_name: String,
}

/// Like [`generate_program`] but with a component [`Registry`] (imported views)
/// so `Card(...)` call sites are inlined in the generated code, plus an optional
/// [`EmbedSpec`] that bakes the app bundle (assets + config) into the binary.
pub fn generate_program_with(
    doc: &Document,
    components: &Registry,
    fallback_title: &str,
    embed: Option<&EmbedSpec>,
) -> String {
    let mut out = String::new();
    out.push_str(&header());
    out.push_str(&foreign_modules(doc));
    if let Some(spec) = embed {
        out.push_str(&emit_embed_module(spec));
    }

    // Window size from the `app { }` block (defaults mirror generate_main), used
    // to statically resolve `Window.width` / `Window.height` dimension props.
    let win_w = doc.app.as_ref().and_then(|a| a.width).unwrap_or(900) as f32;
    let win_h = doc.app.as_ref().and_then(|a| a.height).unwrap_or(600) as f32;

    for view in &doc.views {
        out.push_str(&generate_view_fn(view, components, win_w, win_h));
        out.push('\n');
    }

    // Entry view: `app { entry: Name }` if present + found, else the first.
    let entry = doc
        .app
        .as_ref()
        .and_then(|a| a.entry.as_ref())
        .and_then(|name| doc.views.iter().find(|v| &v.name == name))
        .or_else(|| doc.views.first());

    if let Some(entry) = entry {
        out.push_str(&generate_main(doc, entry, fallback_title, embed));
    } else {
        out.push_str("fn main() {\n    eprintln!(\"no `view` to render\");\n}\n");
    }
    out
}

/// `mod`/`use` declarations for Copper (`.crs`) and Rust (`.rs`) imports, so the
/// items they define are in scope for handlers and expressions. cforge writes
/// the matching `<module>.rs` files into the generated crate (transpiling Copper
/// and copying Rust). De-duplicated by module name.
fn foreign_modules(doc: &Document) -> String {
    use mui_syntax::ast::ImportKind;
    let mut seen: Vec<String> = Vec::new();
    let mut out = String::new();
    for imp in &doc.imports {
        if imp.kind == ImportKind::Mui {
            continue;
        }
        let module = imp.module_name();
        if seen.contains(&module) {
            continue;
        }
        seen.push(module.clone());
        let lang = match imp.kind {
            ImportKind::Copper => "Copper",
            ImportKind::Rust => "Rust",
            ImportKind::Mui => unreachable!(),
        };
        out.push_str(&format!(
            "#[path = \"{module}.rs\"]\nmod {module}; // {lang} import: {}\npub use {module}::*;\n",
            imp.path
        ));
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Emit the embedded-bundle table + extractor for `cforge -c -b`. Each file is
/// baked into the binary with `include_bytes!` (path relative to the crate root,
/// i.e. `../<file>` from `src/main.rs`); at startup `__install_embedded` writes
/// them to a temp dir so the program runs with no external files.
fn emit_embed_module(spec: &EmbedSpec) -> String {
    let mut s = String::new();
    s.push_str("// Embedded app bundle (assets + config) — `cforge -c -b`.\n");
    s.push_str("const __EMBEDDED: &[(&str, &[u8])] = &[\n");
    for f in &spec.files {
        // Forward slashes for include_bytes! (portable in Rust string paths).
        let rel = f.replace('\\', "/");
        s.push_str(&format!(
            "    ({:?}, include_bytes!(concat!(\"../\", {:?}))),\n",
            rel, rel
        ));
    }
    s.push_str("];\n\n");
    s.push_str(&format!(
        "/// Write the embedded bundle to a temp dir and return it.\n\
         fn __install_embedded() -> std::path::PathBuf {{\n\
         \x20   let dir = std::env::temp_dir().join({:?});\n\
         \x20   for (rel, bytes) in __EMBEDDED {{\n\
         \x20       let p = dir.join(rel);\n\
         \x20       if let Some(parent) = p.parent() {{ let _ = std::fs::create_dir_all(parent); }}\n\
         \x20       let _ = std::fs::write(&p, bytes);\n\
         \x20   }}\n\
         \x20   dir\n\
         }}\n\n",
        spec.dir_name
    ));
    s
}

/// File header: module docs + imports + the small reactive scaffolding the
/// generated code uses (signals + subscriptions that must outlive the window).
fn header() -> String {
    "\
// Generated by cforge (mui-codegen) — MUI release codegen (M5).
// Edit the .mui/.crm source, not this file; re-run `cforge build` to regenerate.
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]

use std::cell::RefCell;
use std::rc::Rc;

use mocida::text::by_ptr;
use mocida::{
    App, BackdropMaterial, Button, Checkbox, Children, Color, Cursor, Dialog, FillMode, FontStyle,
    Glass, GlassThickness, Grid, GridView,
    HorizontalAlign, Image, ListView, ProgressBar, RadioButton, Rectangle, Scroll, Shadow, Signal,
    Slider, Sound, Spinner, Stack, StackOrientation, Subscription, Switch, Text, TextArea, TextField,
    TextHAlign, TextVAlign, VibrancyState, Video, VerticalAlign, WebView, Widget, WrapMode,
};

type MuiResult<T> = Result<T, mocida::Error>;

/// A built view: its widget tree plus the live reactive state (signals +
/// subscriptions) that must stay alive for the whole app loop.
struct BuiltView {
    children: Children,
    // Signals are kept alive (and shared with handlers) here.
    _signals: Vec<Rc<RefCell<Signal<i32>>>>,
    _subs: Vec<Subscription>,
    // `Audio` clips — kept alive so a one-shot sound finishes (a dropped
    // `Sound` stops playing).
    _sounds: Vec<Sound>,
}

"
    .to_string()
}

/// Generate `fn build_<view>(<params>) -> MuiResult<BuiltView> { ... }` — a
/// reactive builder: it creates real signals, wires text subscriptions and
/// button handlers, and returns the tree bundled with the live state.
fn generate_view_fn(view: &View, comps: &Registry, win_w: f32, win_h: f32) -> String {
    let mut e = Emitter::new();
    e.dims = mui_syntax::style::DimEnv {
        window_w: Some(win_w),
        window_h: Some(win_h),
        screen_w: Some(win_w),
        screen_h: Some(win_h),
        ids: mui_syntax::style::collect_id_dims(&view.body),
    };
    let params = view
        .params
        .iter()
        .map(|p| {
            format!(
                "{}: {}",
                sanitize_ident(&p.name),
                param_type(p.ty.as_deref())
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    e.line(&format!(
        "/// View `{}` — generated from MUI source.",
        view.name
    ));
    e.line(&format!(
        "fn {}({}) -> MuiResult<BuiltView> {{",
        view_fn_name(&view.name),
        params
    ));
    e.indent();
    e.line("let mut __children = Children::new(16)?;");
    e.line("let mut __y: f32 = 24.0;");
    e.line("let mut __subs: Vec<Subscription> = Vec::new();");
    e.line("let mut __keep: Vec<Rc<RefCell<Signal<i32>>>> = Vec::new();");
    e.line("#[allow(unused_mut)]");
    e.line("let mut __sounds: Vec<Sound> = Vec::new();");

    // Seed from params, then fold in view-level state bindings so `${count}`
    // resolves and the signal map is known for the whole tree.
    let mut env = Env::from_view(view);
    env.absorb_bindings(&view.body);
    // Declare real signals for view-level `mut x = signal(...)` bindings.
    let mut sigs = SignalScope::default();
    declare_signals(&mut e, &view.body, &env, &mut sigs);

    for node in &view.body {
        emit_node(&mut e, node, &env, &sigs, comps, "__children");
    }

    e.line(
        "Ok(BuiltView { children: __children, _signals: __keep, _subs: __subs, _sounds: __sounds })",
    );
    e.dedent();
    e.line("}");
    e.finish()
}

/// Generate the `main()` that opens the window with `entry`, configured from
/// the document's optional `app { }` block (title / size / background / name /
/// id). An `./app.bundle` manifest is still auto-loaded by `App::new`; the
/// `app {}` block, when present, overrides its name/id/title here.
fn generate_main(
    doc: &Document,
    entry: &View,
    fallback_title: &str,
    embed: Option<&EmbedSpec>,
) -> String {
    let app = doc.app.as_ref();
    // Title: app.title → app.name → "<fallback> — <view>".
    let title = app
        .and_then(|a| a.title.clone())
        .or_else(|| app.and_then(|a| a.name.clone()))
        .unwrap_or_else(|| format!("{fallback_title} — {}", entry.name));
    let width = app.and_then(|a| a.width).unwrap_or(900);
    let height = app.and_then(|a| a.height).unwrap_or(600);
    let (br, bg, bb, _ba) = app
        .and_then(|a| a.background)
        .unwrap_or((241, 245, 249, 255));

    let mut e = Emitter::new();
    e.line("fn main() -> MuiResult<()> {");
    e.indent();
    // --bundle: extract the embedded assets/config to a temp dir and load the
    // bundle from there, so the binary needs no external files.
    if embed.is_some() {
        e.line("let __bundle_dir = __install_embedded();");
        e.line(
            "let _ = mocida::bundle::load_manifest(__bundle_dir.join(\"app.bundle\").to_string_lossy().as_ref());",
        );
    }
    // Custom (client-side) title bar: must be requested BEFORE App::new so the
    // window is created borderless. `app { titlebar: custom }` (aliases:
    // `decorations: none`, `customTitlebar: true`).
    let custom_titlebar = app
        .and_then(|a| a.titlebar.clone())
        .map(|t| matches!(t.to_ascii_lowercase().as_str(), "custom" | "client" | "none"))
        .unwrap_or(false);
    if custom_titlebar {
        e.line("mocida::app::request_custom_titlebar(true);");
    }
    e.line(&format!(
        "let mut app = App::new({title:?}, {width}, {height})?;"
    ));
    e.line(&format!(
        "app.set_background_color(Color::rgb({br}, {bg}, {bb}));"
    ));
    // OS window backdrop (Mica / Acrylic / KDE blur) from `app { backdrop }`.
    if let Some(spec) = app.and_then(|a| a.backdrop.clone()) {
        e.line("{");
        e.indent();
        e.line(&format!("let __bd = BackdropMaterial::from_effect({spec:?});"));
        e.line("if __bd != BackdropMaterial::None {");
        e.indent();
        e.line("if let Some(mut __w) = mocida::window::Window::active() {");
        e.indent();
        e.line("__w.set_backdrop(__bd, Color::rgba(255, 255, 255, 0.0), 0.0);");
        e.dedent();
        e.line("}");
        e.dedent();
        e.line("}");
        e.dedent();
        e.line("}");
    }
    if let Some(name) = app.and_then(|a| a.name.clone()) {
        e.line(&format!("mocida::bundle::set_name({name:?});"));
        e.line(&format!("let _ = app.set_name({name:?});"));
    }
    if let Some(id) = app.and_then(|a| a.id.clone()) {
        e.line(&format!("let _ = app.set_app_id({id:?});"));
    }
    e.line("mocida::text::search_fonts();");
    e.line("let _ = mocida::text::get_font(\"Arial\");");
    let args = entry
        .params
        .iter()
        .map(|p| default_arg(p.default.as_ref(), p.ty.as_deref()))
        .collect::<Vec<_>>()
        .join(", ");
    e.line(&format!(
        "let view = {}({})?;",
        view_fn_name(&entry.name),
        args
    ));
    e.line("app.set_children(view.children);");
    e.line("app.show().run();");
    // Keep the signals + subscriptions alive across the whole loop.
    e.line("drop(view._subs);");
    e.line("drop(view._signals);");
    e.line("Ok(())");
    e.dedent();
    e.line("}");
    e.finish()
}

// ---------------------------------------------------------------------------
// Signal scope
// ---------------------------------------------------------------------------

/// Tracks which state names are live `Signal<i32>` and the generated local
/// variable that holds each (an `Rc<RefCell<Signal<i32>>>`).
#[derive(Default, Clone)]
struct SignalScope {
    /// name → generated var (e.g. `count` → `__sig_count`).
    vars: std::collections::HashMap<String, String>,
}

impl SignalScope {
    fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }
}

/// Emit `let __sig_<name> = Rc::new(RefCell::new(Signal::new(init)?));` for each
/// `mut name = signal(...)` binding in `nodes`, registering it in `scope` and
/// pushing a clone into `__keep` so it outlives the window.
fn declare_signals(e: &mut Emitter, nodes: &[Node], env: &Env, scope: &mut SignalScope) {
    for n in nodes {
        if let Node::Let {
            name,
            value: Some(expr),
            ..
        } = n
        {
            if scope.vars.contains_key(name) {
                continue;
            }
            if let Some(init) = signal_init(expr, env) {
                let var = format!("__sig_{}", sanitize_ident(name));
                e.line(&format!(
                    "let {var} = Rc::new(RefCell::new(Signal::<i32>::new({init})?));"
                ));
                e.line(&format!("__keep.push({var}.clone());"));
                scope.vars.insert(name.clone(), var);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Node emission
// ---------------------------------------------------------------------------

/// Emit code that builds `node` and adds it to the collection named `sink`.
fn emit_node(
    e: &mut Emitter,
    node: &Node,
    env: &Env,
    sigs: &SignalScope,
    comps: &Registry,
    sink: &str,
) {
    match node {
        Node::Element(el) => emit_element(e, el, env, sigs, comps, sink),
        Node::Let { name, .. } => {
            // The signal was created in declare_signals; just document it.
            if sigs.var(name).is_some() {
                e.line(&format!("// state `{name}` → signal (live)"));
            }
        }
        Node::Effect { raw, .. } => {
            // Run the effect's interpretable statements once, here at build time.
            // (Conditional / string / cross-signal effects await the evaluator.)
            let actions = mui_syntax::ast::parse_actions(raw);
            if actions.is_empty() {
                e.line("// effect { ... } (no interpretable statements yet)");
            }
            for action in actions {
                if let Some(v) = sigs.var(action.name()) {
                    let update = action_update_expr(&action);
                    e.line("{");
                    e.indent();
                    e.line(&format!("let __cur = {v}.borrow().get();"));
                    e.line(&format!("let __next = {update};"));
                    e.line(&format!("let _ = {v}.borrow_mut().set(__next);"));
                    e.dedent();
                    e.line("}");
                }
            }
        }
        Node::If { then, .. } => {
            e.line("// if cond { ... } — static lowering renders the `then` branch");
            for n in then {
                emit_node(e, n, env, sigs, comps, sink);
            }
        }
        Node::For { body, .. } => {
            e.line("// for ... { } — static lowering renders one iteration");
            for n in body {
                emit_node(e, n, env, sigs, comps, sink);
            }
        }
        Node::Match { .. } => e.line("// match { ... } (not yet executed)"),
        Node::Expr(_) => {}
    }
}

fn emit_element(
    e: &mut Emitter,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    comps: &Registry,
    sink: &str,
) {
    // An imported component (a view in the registry) is inlined; built-ins map
    // to mocida widgets. Core widget names can't be shadowed.
    if !is_core_widget(&el.name) && comps.contains_key(&el.name) {
        emit_component(e, el, env, comps, sink);
        return;
    }
    match el.name.as_str() {
        "Rectangle" | "Rect" | "Box" => emit_rectangle(e, el, env, sigs, comps, sink, None),
        // A `Popup` with `visible: false` renders nothing; otherwise it's a
        // floating card (a Rectangle container lifted above siblings via a high
        // default z-index).
        "Popup" if style::bool_prop(el, "visible") == Some(false) => {}
        "Popup" => emit_rectangle(e, el, env, sigs, comps, sink, Some(1000)),
        "Stack" => emit_stack(e, el, env, sigs, comps, sink),
        "Glass" => emit_glass(e, el, env, sigs, comps, sink),
        "Grid" => emit_grid(e, el, env, sigs, comps, sink),
        "Scroll" => emit_scroll(e, el, env, sigs, comps, sink),
        "ListView" => emit_listview(e, el, env, sigs, comps, sink),
        "GridView" => emit_gridview(e, el, env, sigs, comps, sink),
        "Text" => emit_text(e, el, env, sigs, sink),
        "Button" => emit_button(e, el, env, sigs, sink),
        "TextField" | "Input" | "TextInput" => emit_textfield(e, el, env, sink),
        "TextArea" => emit_textarea(e, el, env, sink),
        "Checkbox" => emit_checkbox(e, el, env, sigs, sink),
        "RadioButton" | "Radio" => emit_radio(e, el, env, sigs, sink),
        "Switch" => emit_switch(e, el, env, sigs, sink),
        "Slider" => emit_slider(e, el, sink),
        "ProgressBar" => emit_progressbar(e, el, sink),
        "Spinner" => emit_spinner(e, el, sink),
        "Image" => emit_image(e, el, env, sigs, sink),
        "Video" => emit_video(e, el, env, sigs, sink),
        "WebView" | "Webview" => emit_webview(e, el, env, sigs, sink),
        "Dialog" => emit_dialog(e, el, env, sigs, comps, sink),
        "Audio" | "Sound" => emit_audio(e, el, env, sigs),
        _ if !el.children.is_empty() => emit_stack(e, el, env, sigs, comps, sink),
        other => emit_placeholder(e, other, el, env, sigs, sink),
    }
}

/// Names that map to a built-in mocida widget (can't be shadowed by an imported
/// component). Mirrors `mui_runtime`'s `is_builtin`.
fn is_core_widget(name: &str) -> bool {
    matches!(
        name,
        "Rectangle"
            | "Rect"
            | "Box"
            | "Stack"
            | "Glass"
            | "Grid"
            | "Scroll"
            | "ListView"
            | "GridView"
            | "Text"
            | "Button"
            | "TextField"
            | "Input"
            | "TextInput"
            | "TextArea"
            | "Checkbox"
            | "RadioButton"
            | "Radio"
            | "Switch"
            | "Slider"
            | "ProgressBar"
            | "Spinner"
            | "Image"
            | "Video"
            | "WebView"
            | "Webview"
            | "Dialog"
            | "Popup"
            | "Audio"
            | "Sound"
    )
}

/// Inline an imported component at the call site: build its body inside a
/// wrapping Stack, with the component's params seeded from the call's args
/// (named props or the positional). Recursion is bounded by `depth` —
/// `comps` minus the name being expanded would be cleaner, but a depth cap is
/// simpler and avoids infinite inlining of a self-referential component.
fn emit_component(e: &mut Emitter, el: &Element, env: &Env, comps: &Registry, sink: &str) {
    let Some(view) = comps.get(&el.name) else {
        return;
    };
    // Build the component's env: param = call arg (named/positional) or default.
    let mut comp_env = Env::default();
    for (i, p) in view.params.iter().enumerate() {
        let val = component_arg(el, &p.name, i, env)
            .or_else(|| p.default.as_ref().and_then(literal_display));
        if let Some(v) = val {
            comp_env.vars.insert(p.name.clone(), v);
        }
    }
    e.line(&format!("// <{}> component", el.name));
    e.line("{");
    e.indent();
    let var = e.fresh("comp");
    e.line(&format!(
        "let mut {var} = Stack::new(StackOrientation::Vertical)?;"
    ));
    // Inline the body. To keep signatures simple we don't recurse into nested
    // imported components from generated code (a depth-1 inline covers the
    // common case); nested components fall back to placeholders.
    let inner_comps = Registry::new();
    let mut inner_sigs = SignalScope::default();
    declare_signals(e, &view.body, &comp_env, &mut inner_sigs);
    for node in &view.body {
        emit_node(e, node, &comp_env, &inner_sigs, &inner_comps, &var);
    }
    let anchor = anchor_call(style::anchor(el));
    e.line(&format!(
        "let __w = {var}.into_widget_sized(400.0, 200.0)?.position({});",
        position_args(el)
    ));
    if let Some(call) = &anchor {
        e.line(&format!("__w{call};"));
    }
    e.line(&format!("{sink}.add(__w)?;"));
    e.line("__y += 208.0;");
    e.dedent();
    e.line("}");
}

/// Resolve a component call's argument for `param`: a matching named prop, or
/// the positional for the first param. Returns its display string (resolved
/// against the caller's `env`).
fn component_arg(el: &Element, param: &str, index: usize, env: &Env) -> Option<String> {
    if let Some(p) = el.props.iter().find(|p| p.name == param) {
        if let PropValue::Expr(e) = &p.value {
            return Some(render_text_expr(e, env, &SignalScope::default()));
        }
        return None;
    }
    if index == 0 {
        if let Some(ex) = &el.positional {
            return Some(render_text_expr(ex, env, &SignalScope::default()));
        }
    }
    None
}

/// `Rectangle(...)` → `mocida::Rectangle`. The base widget and base container:
/// a filled/rounded/bordered/shadowed box that can hold children. Children are
/// emitted into an inner vertical `Stack` (so their layout matches the runtime),
/// which becomes the rectangle's single child; the rectangle owns fill, border,
/// shadow, padding and the outer size.
fn emit_rectangle(
    e: &mut Emitter,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    comps: &Registry,
    sink: &str,
    default_z: Option<i32>,
) {
    let has_children = !el.children.is_empty();
    let fill = style::color_prop(el, "fill")
        .or_else(|| style::background(el))
        .unwrap_or(Rgba {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        });
    let pad = style::f32_prop(el, "padding").unwrap_or(0.0);
    let gap = style::f32_prop(el, "gap").unwrap_or(0.0);
    // Codegen doesn't measure content, so honour explicit sizes and fall back to
    // sensible defaults (a childless rect is a small square).
    let w = dim(e, el, "width").unwrap_or(if has_children { 400.0 } else { 100.0 });
    let h = dim(e, el, "height").unwrap_or(if has_children { 160.0 } else { 100.0 });

    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut __r = Rectangle::new()?.color({});",
        color_lit(fill)
    ));
    if let Some(r) = style::f32_prop(el, "radius") {
        e.line(&format!("__r = __r.radius({});", fmt_f32(r)));
    }
    if let Some(bw) = style::f32_prop(el, "borderWidth") {
        e.line(&format!("__r = __r.border_width({});", fmt_f32(bw)));
    }
    if let Some(bc) = style::color_prop(el, "borderColor") {
        e.line(&format!("__r = __r.border_color({});", color_lit(bc)));
    }
    if let Some(sh) = style::shadow(el) {
        e.line(&format!("__r = __r.shadow({});", shadow_lit(sh)));
    }
    if pad > 0.0 {
        let p = fmt_f32(pad);
        e.line(&format!("__r = __r.padding({p}, {p}, {p}, {p});"));
    }

    if has_children {
        // Children go into an inner vertical stack (spacing = gap); the rect's
        // padding offsets it. This keeps codegen layout aligned with the
        // runtime, which lays a rect's children out vertically.
        let cw = (w - pad * 2.0).max(1.0);
        let ch = (h - pad * 2.0).max(1.0);
        e.line("{");
        e.indent();
        e.line("let mut __rc = Stack::new(StackOrientation::Vertical)?;");
        if gap > 0.0 {
            e.line(&format!("__rc = __rc.spacing({});", fmt_f32(gap)));
        }
        let mut child_env = env.clone();
        child_env.absorb_bindings(&el.children);
        let mut child_sigs = sigs.clone();
        declare_signals(e, &el.children, &child_env, &mut child_sigs);
        for node in &el.children {
            emit_node(e, node, &child_env, &child_sigs, comps, "__rc");
        }
        e.line(&format!(
            "__r.add_child(__rc.into_widget_sized({}, {})?);",
            fmt_f32(cw),
            fmt_f32(ch)
        ));
        e.dedent();
        e.line("}");
    }

    e.line(&format!(
        "let __w = __r.into_widget_sized({}, {})?.position({});",
        fmt_f32(w),
        fmt_f32(h),
        position_args(el)
    ));
    if let Some(op) = style::f32_prop(el, "opacity") {
        e.line(&format!("let __w = __w.opacity({});", fmt_f32(op)));
    }
    if let Some(rot) = style::f32_prop(el, "rotation") {
        e.line(&format!("let __w = __w.rotation({});", fmt_f32(rot)));
    }
    // `zIndex:` (or the Popup default) lifts the widget above its siblings.
    if let Some(z) = style::f32_prop(el, "zIndex")
        .map(|z| z as i32)
        .or(default_z)
    {
        e.line(&format!("let __w = __w.z_index({z});"));
    }
    if let Some(call) = anchor_call(style::anchor(el)) {
        e.line(&format!("__w{call};"));
    }
    e.line(&format!("{sink}.add(__w)?;"));
    e.line(&format!("__y += {};", fmt_f32(h + 8.0)));
    e.dedent();
    e.line("}");
}

/// Render a [`ShadowSpec`] as a `Shadow { ... }` source expression.
fn shadow_lit(s: ShadowSpec) -> String {
    format!(
        "Shadow {{ offset_x: {}, offset_y: {}, blur: {}, spread: {}, color: {} }}",
        fmt_f32(s.dx),
        fmt_f32(s.dy),
        fmt_f32(s.blur),
        fmt_f32(s.spread),
        color_lit(s.color),
    )
}

fn emit_stack(
    e: &mut Emitter,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    comps: &Registry,
    sink: &str,
) {
    let orient = match style::enum_member(el, "orientation").as_deref() {
        Some("horizontal") => "StackOrientation::Horizontal",
        _ => "StackOrientation::Vertical",
    };
    // Honour an explicit `width`/`height` (resolving `Window.width` /
    // `left_panel.width` / arithmetic); fall back to a generous default since
    // codegen doesn't measure content.
    let sw = dim(e, el, "width").unwrap_or(400.0);
    let sh = dim(e, el, "height").unwrap_or(400.0);
    let var = e.fresh("stack");
    e.line("{");
    e.indent();
    e.line(&format!("let mut {var} = Stack::new({orient})?;"));
    if let Some(gap) = style::f32_prop(el, "gap") {
        e.line(&format!("{var} = {var}.spacing({});", fmt_f32(gap)));
    }
    if let Some(pad) = style::f32_prop(el, "padding") {
        let p = fmt_f32(pad);
        e.line(&format!("{var} = {var}.padding({p}, {p}, {p}, {p});"));
    }

    // Signals declared in this block are visible to its children.
    let mut child_env = env.clone();
    child_env.absorb_bindings(&el.children);
    let mut child_sigs = sigs.clone();
    declare_signals(e, &el.children, &child_env, &mut child_sigs);
    for node in &el.children {
        emit_node(e, node, &child_env, &child_sigs, comps, &var);
    }

    if key_handler(el).is_some() {
        // Capture the Widget so an `onKeyInput` key-down handler can be wired
        // onto it before it's added to the parent.
        e.line(&format!(
            "let __w = {var}.into_widget_sized({}, {})?;",
            fmt_f32(sw),
            fmt_f32(sh)
        ));
        emit_key_handler(e, el, sigs, "__w");
        e.line(&format!("{sink}.add(__w)?;"));
    } else {
        e.line(&format!(
            "{sink}.add({var}.into_widget_sized({}, {})?)?;",
            fmt_f32(sw),
            fmt_f32(sh)
        ));
    }
    e.dedent();
    e.line("}");
}

/// `Glass(effect:, radius:, tint:, tintOpacity:, blur:, …)` → `mocida::Glass`.
/// Mirrors [`emit_stack`] but constructs a glass container with a backdrop
/// material and its tint / effect-specific knobs.
fn emit_glass(
    e: &mut Emitter,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    comps: &Registry,
    sink: &str,
) {
    let effect = style::enum_member(el, "effect").unwrap_or_else(|| "auto".to_string());
    let horizontal = matches!(style::enum_member(el, "orientation").as_deref(), Some("horizontal"));
    let sw = dim(e, el, "width").unwrap_or(400.0);
    let sh = dim(e, el, "height").unwrap_or(400.0);
    let var = e.fresh("glass");

    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut {var} = Glass::new(BackdropMaterial::from_effect({effect:?}))?;"
    ));
    e.line(&format!("{var} = {var}.horizontal({horizontal});"));
    if let Some(r) = style::f32_prop(el, "radius") {
        e.line(&format!("{var} = {var}.radius({});", fmt_f32(r)));
    }
    if let Some(c) = style::color_prop(el, "tint").or_else(|| style::background(el)) {
        e.line(&format!("{var} = {var}.tint({});", color_lit(c)));
    }
    if let Some(o) = style::f32_prop(el, "tintOpacity") {
        e.line(&format!("{var} = {var}.tint_opacity({});", fmt_f32(o)));
    }
    if let Some(th) = style::enum_member(el, "thickness").as_deref().and_then(|t| match t {
        "thin" => Some("GlassThickness::Thin"),
        "thick" => Some("GlassThickness::Thick"),
        "regular" | "medium" => Some("GlassThickness::Regular"),
        _ => None,
    }) {
        e.line(&format!("{var} = {var}.thickness({th});"));
    }
    if let Some(b) = style::f32_prop(el, "blur") {
        e.line(&format!("{var} = {var}.blur({});", fmt_f32(b)));
    }
    if let Some(r) = style::f32_prop(el, "refraction") {
        e.line(&format!("{var} = {var}.refraction({});", fmt_f32(r)));
    }
    if let Some(n) = style::f32_prop(el, "noise") {
        e.line(&format!("{var} = {var}.noise({});", fmt_f32(n)));
    }
    if let Some(s) = style::enum_member(el, "state").as_deref().and_then(|s| match s {
        "inactive" => Some("VibrancyState::Inactive"),
        "pressed" => Some("VibrancyState::Pressed"),
        "active" => Some("VibrancyState::Active"),
        _ => None,
    }) {
        e.line(&format!("{var} = {var}.vibrancy_state({s});"));
    }
    if let Some(gap) = style::f32_prop(el, "gap") {
        e.line(&format!("{var} = {var}.spacing({});", fmt_f32(gap)));
    }
    if let Some(pad) = style::f32_prop(el, "padding") {
        let p = fmt_f32(pad);
        e.line(&format!("{var} = {var}.padding({p}, {p}, {p}, {p});"));
    }
    if let Some(a) = style::enum_member(el, "align").as_deref().and_then(|a| match a {
        "center" | "middle" => Some(1),
        "end" | "right" | "bottom" => Some(2),
        "start" | "left" | "top" => Some(0),
        _ => None,
    }) {
        e.line(&format!("{var} = {var}.align({a});"));
    }
    if let Some(j) = style::enum_member(el, "justify").as_deref().and_then(|j| match j {
        "center" | "middle" => Some(1),
        "end" => Some(2),
        "spacebetween" | "between" | "space-between" => Some(3),
        "start" => Some(0),
        _ => None,
    }) {
        e.line(&format!("{var} = {var}.justify({j});"));
    }

    let mut child_env = env.clone();
    child_env.absorb_bindings(&el.children);
    let mut child_sigs = sigs.clone();
    declare_signals(e, &el.children, &child_env, &mut child_sigs);
    for node in &el.children {
        emit_node(e, node, &child_env, &child_sigs, comps, &var);
    }

    e.line(&format!(
        "{sink}.add({var}.into_widget_sized({}, {})?)?;",
        fmt_f32(sw),
        fmt_f32(sh)
    ));
    e.dedent();
    e.line("}");
}

fn emit_text(e: &mut Emitter, el: &Element, env: &Env, sigs: &SignalScope, sink: &str) {
    let size = style::f32_prop(el, "size").unwrap_or(16.0);
    let color = style::color_prop(el, "color")
        .map(color_lit)
        .unwrap_or_else(|| "Color::rgb(15, 23, 42)".to_string());
    let anchor = anchor_call(style::anchor(el));

    // Which signals does this text's label read?
    let reads: Vec<String> = el
        .positional
        .as_ref()
        .map(names_read)
        .unwrap_or_default()
        .into_iter()
        .filter(|n| sigs.var(n).is_some())
        .collect();

    let label = el
        .positional
        .as_ref()
        .map(|x| render_text_expr(x, env, sigs))
        .unwrap_or_default();
    let (w, h) = text_extent(&label, size);

    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut __t = Text::new({:?}, {})?.color({});",
        label,
        fmt_f32(size),
        color
    ));
    emit_font(e, "__t", el);
    if let Some(a) = style::enum_member(el, "align")
        .or_else(|| style::enum_member(el, "hAlign"))
        .as_deref()
        .and_then(text_h_align_lit)
    {
        e.line(&format!("__t = __t.h_align({a});"));
    }
    if let Some(a) = style::enum_member(el, "vAlign")
        .as_deref()
        .and_then(text_v_align_lit)
    {
        e.line(&format!("__t = __t.v_align({a});"));
    }
    if let Some(wm) = style::enum_member(el, "wrap").as_deref().and_then(wrap_lit) {
        e.line(&format!("__t = __t.wrap_mode({wm});"));
    }
    if let Some(c) = style::enum_member(el, "cursor") {
        e.line(&format!("__t = __t.cursor({});", cursor_lit(&c)));
    }

    let add_widget = |e: &mut Emitter| {
        e.line(&format!(
            "let __w = __t.into_widget_sized({}, {})?.position({});",
            fmt_f32(w),
            fmt_f32(h),
            position_args(el)
        ));
        if let Some(call) = &anchor {
            e.line(&format!("__w{call};"));
        }
        e.line(&format!("{sink}.add(__w)?;"));
    };

    if reads.is_empty() {
        add_widget(e);
    } else {
        // Reactive label: keep the UIText pointer, subscribe to each read
        // signal, recompute + set the text on change.
        e.line("let __tp = __t.as_ptr();");
        add_widget(e);
        // Capture each dependency signal's RAW pointer (not the Rc): the
        // subscription fires synchronously inside a button handler's
        // `borrow_mut().set(...)`, so re-borrowing the RefCell would panic
        // ("already mutably borrowed"). The value is already updated in C by
        // the time we read it, and the owning Rc (in __keep) keeps it alive.
        for r in &reads {
            let v = sigs.var(r).unwrap();
            e.line(&format!("let __u_{r} = {v}.borrow().as_ptr();"));
        }
        // The updater closure: read current values, format, set text.
        e.line("let __update = move || {");
        e.indent();
        // Build the format args from the live signals + static env fallback.
        let fmt = reactive_format(el.positional.as_ref(), env, sigs, &reads);
        e.line(&format!("let __s = {fmt};"));
        e.line("let _ = unsafe { by_ptr::set_text(__tp, &__s) };");
        e.dedent();
        e.line("};");
        // Subscribe each dependency.
        for r in &reads {
            let v = sigs.var(r).unwrap();
            e.line("{");
            e.indent();
            e.line("let __u = __update.clone();");
            e.line(&format!(
                "if let Ok(__sub) = {v}.borrow_mut().subscribe(move |_| __u()) {{ __subs.push(__sub); }}"
            ));
            e.dedent();
            e.line("}");
        }
    }
    e.line(&format!("__y += {};", fmt_f32(h + 8.0)));
    e.dedent();
    e.line("}");
}

fn emit_button(e: &mut Emitter, el: &Element, env: &Env, sigs: &SignalScope, sink: &str) {
    let label = el
        .positional
        .as_ref()
        .map(|x| render_text_expr(x, env, sigs))
        .unwrap_or_else(|| "Button".to_string());
    let size = style::f32_prop(el, "size").unwrap_or(16.0);
    let (w, h) = text_extent(&label, size);
    let bw = (w + 24.0).max(48.0);
    let bh = (h + 12.0).max(32.0);

    // Fill + text colors (background/bg/color → fill; textColor → label).
    let bg = style::background(el).unwrap_or(Rgba {
        r: 59,
        g: 130,
        b: 246,
        a: 255,
    });
    let text_col = style::color_prop(el, "textColor").unwrap_or(Rgba {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    });
    let radius = style::f32_prop(el, "radius").unwrap_or(8.0);

    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut __b = Button::new({:?}, {})?.radius({}).colors({}, {});",
        label,
        fmt_f32(size),
        fmt_f32(radius),
        color_lit(bg),
        color_lit(text_col),
    ));
    if let Some(bwd) = style::f32_prop(el, "borderWidth") {
        e.line(&format!("__b = __b.border_width({});", fmt_f32(bwd)));
    }
    if let Some(c) = style::enum_member(el, "cursor") {
        e.line(&format!("__b = __b.cursor({});", cursor_lit(&c)));
    }
    emit_font(e, "__b", el);
    if let Some(sh) = style::shadow(el) {
        e.line(&format!("__b = __b.shadow({});", shadow_lit(sh)));
    }
    if style::bool_prop(el, "enabled") == Some(false) {
        e.line("__b = __b.enabled(false);");
    }

    // Wire onClick to a signal mutation when interpretable.
    if let Some(action) = onclick_action(el) {
        if let Some(v) = sigs.var(action.name()) {
            e.line(&format!("let __h = {v}.clone();"));
            let update = action_update_expr(&action);
            e.line("__b = __b.on_click(move |_| {");
            e.indent();
            e.line("let __cur = __h.borrow().get();");
            e.line(&format!("let __next = {update};"));
            e.line("let _ = __h.borrow_mut().set(__next);");
            e.dedent();
            e.line("});");
        }
    }

    e.line(&format!(
        "let __w = __b.into_widget_sized({}, {})?.position({});",
        fmt_f32(bw),
        fmt_f32(bh),
        position_args(el)
    ));
    if let Some(call) = anchor_call(style::anchor(el)) {
        e.line(&format!("__w{call};"));
    }
    e.line(&format!("{sink}.add(__w)?;"));
    e.line(&format!("__y += {};", fmt_f32(bh + 8.0)));
    e.dedent();
    e.line("}");
}

/// The `x, y` argument string for a widget's `.position(x, y)` call. Every
/// widget accepts the common `x:` / `y:` props; each given axis overrides the
/// stacked-layout default, while a missing axis keeps the running cursor
/// (`24.0` horizontally, the auto-advanced `__y` vertically). So `x: 100` alone
/// pins the column but lets the widget flow down with its siblings.
/// A dimension prop (`width`/`height`/`x`/`y`), resolving metric refs and
/// arithmetic (`Window.width - 520`, `left_panel.width`) against the emitter's
/// [`DimEnv`] — not just literals.
fn dim(e: &Emitter, el: &Element, name: &str) -> Option<f32> {
    style::dim_prop(el, name, &e.dims)
}

fn position_args(el: &Element) -> String {
    let x = style::f32_prop(el, "x")
        .map(fmt_f32)
        .unwrap_or_else(|| "24.0".to_string());
    let y = style::f32_prop(el, "y")
        .map(fmt_f32)
        .unwrap_or_else(|| "__y".to_string());
    format!("{x}, {y}")
}

/// Emit the common tail for a sized leaf widget: lift the builder variable
/// `var` into a positioned [`Widget`], apply the anchor, add it to `sink`, and
/// advance the `__y` cursor so the next top-level sibling sits below it. (When
/// the widget is nested in a container the container re-lays it out, so the
/// `__y` position is harmless there.) `el` supplies the `x:` / `y:` overrides.
fn emit_widget_tail(
    e: &mut Emitter,
    el: &Element,
    var: &str,
    w: f32,
    h: f32,
    anchor: Option<String>,
    sink: &str,
) {
    e.line(&format!(
        "let __w = {var}.into_widget_sized({}, {})?.position({});",
        fmt_f32(w),
        fmt_f32(h),
        position_args(el)
    ));
    if let Some(call) = anchor {
        e.line(&format!("__w{call};"));
    }
    e.line(&format!("{sink}.add(__w)?;"));
    e.line(&format!("__y += {};", fmt_f32(h + 8.0)));
}

fn emit_grid(
    e: &mut Emitter,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    comps: &Registry,
    sink: &str,
) {
    let columns = style::f32_prop(el, "columns")
        .map(|c| c as i32)
        .unwrap_or(2)
        .max(1);
    let gap = style::f32_prop(el, "gap").unwrap_or(8.0);
    let var = e.fresh("grid");
    e.line("{");
    e.indent();
    let g = fmt_f32(gap);
    e.line(&format!(
        "let mut {var} = Grid::new({columns})?.gap({g}, {g});"
    ));
    let mut child_env = env.clone();
    child_env.absorb_bindings(&el.children);
    let mut child_sigs = sigs.clone();
    declare_signals(e, &el.children, &child_env, &mut child_sigs);
    for node in &el.children {
        emit_node(e, node, &child_env, &child_sigs, comps, &var);
    }
    emit_widget_tail(
        e,
        el,
        &var,
        400.0,
        300.0,
        anchor_call(style::anchor(el)),
        sink,
    );
    e.dedent();
    e.line("}");
}

/// `Scroll(direction:) { children }` — scrolling viewport whose children fill an
/// inner vertical stack set as the content. Mirrors the runtime's `build_scroll`
/// (codegen uses a generous fixed content size since it doesn't measure).
fn emit_scroll(
    e: &mut Emitter,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    comps: &Registry,
    sink: &str,
) {
    let (axis_v, axis_h) = match style::enum_member(el, "direction").as_deref() {
        Some("horizontal") => (false, true),
        Some("both") => (true, true),
        _ => (true, false),
    };
    let gap = style::f32_prop(el, "gap").unwrap_or(8.0);
    let w = dim(e, el, "width").unwrap_or(400.0);
    let height = dim(e, el, "height").unwrap_or(240.0);
    e.line("{");
    e.indent();
    e.line("let mut __scroll = Scroll::new()?;");
    e.line(&format!("__scroll = __scroll.axes({axis_v}, {axis_h});"));
    if let Some(ws) = style::f32_prop(el, "wheelSpeed") {
        e.line(&format!(
            "__scroll = __scroll.wheel_speed({});",
            fmt_f32(ws)
        ));
    }
    if style::bool_prop(el, "dragScroll") == Some(true) {
        e.line("__scroll = __scroll.drag_scroll(true);");
    }
    // `scrollbar: false` hides the bar; `scrollbarColor`/`scrollbarTrackColor`/
    // `scrollbarWidth` customize it.
    if style::bool_prop(el, "scrollbar") == Some(false) {
        e.line("__scroll = __scroll.scrollbar(false);");
    }
    let bar_thumb = style::color_prop(el, "scrollbarColor");
    let bar_track = style::color_prop(el, "scrollbarTrackColor");
    let bar_width = style::f32_prop(el, "scrollbarWidth");
    if bar_thumb.is_some() || bar_track.is_some() || bar_width.is_some() {
        let thumb = bar_thumb
            .map(color_lit)
            .unwrap_or_else(|| "Color::rgba(150, 155, 168, 0.55)".to_string());
        let track = bar_track
            .map(color_lit)
            .unwrap_or_else(|| "Color::rgba(255, 255, 255, 0.04)".to_string());
        e.line(&format!(
            "__scroll = __scroll.scrollbar_style({}, {}, {});",
            thumb,
            track,
            fmt_f32(bar_width.unwrap_or(8.0))
        ));
    }
    e.line(&format!(
        "let mut __sc = Stack::new(StackOrientation::Vertical)?.spacing({});",
        fmt_f32(gap)
    ));
    let mut child_env = env.clone();
    child_env.absorb_bindings(&el.children);
    let mut child_sigs = sigs.clone();
    declare_signals(e, &el.children, &child_env, &mut child_sigs);
    for node in &el.children {
        emit_node(e, node, &child_env, &child_sigs, comps, "__sc");
    }
    e.line("__scroll = __scroll.content(__sc.into_widget_sized(400.0, 600.0)?);");
    emit_widget_tail(
        e,
        el,
        "__scroll",
        w,
        height,
        anchor_call(style::anchor(el)),
        sink,
    );
    e.dedent();
    e.line("}");
}

/// `ListView(itemHeight:) { children }` — vertical scrolling list of rows.
fn emit_listview(
    e: &mut Emitter,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    comps: &Registry,
    sink: &str,
) {
    let item_h = style::f32_prop(el, "itemHeight").unwrap_or(40.0);
    let w = dim(e, el, "width").unwrap_or(240.0);
    let height = dim(e, el, "height").unwrap_or(320.0);
    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut __lv = ListView::new({})?;",
        fmt_f32(item_h)
    ));
    let mut child_env = env.clone();
    child_env.absorb_bindings(&el.children);
    let mut child_sigs = sigs.clone();
    declare_signals(e, &el.children, &child_env, &mut child_sigs);
    for node in &el.children {
        emit_node(e, node, &child_env, &child_sigs, comps, "__lv");
    }
    emit_widget_tail(
        e,
        el,
        "__lv",
        w,
        height,
        anchor_call(style::anchor(el)),
        sink,
    );
    e.dedent();
    e.line("}");
}

/// `GridView(columns:, cellWidth:, cellHeight:) { children }` — scrolling grid.
fn emit_gridview(
    e: &mut Emitter,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    comps: &Registry,
    sink: &str,
) {
    let columns = style::f32_prop(el, "columns")
        .map(|c| c as i32)
        .unwrap_or(2)
        .max(1);
    let cell_w = style::f32_prop(el, "cellWidth")
        .or_else(|| style::f32_prop(el, "cellSize"))
        .unwrap_or(120.0);
    let cell_h = style::f32_prop(el, "cellHeight")
        .or_else(|| style::f32_prop(el, "cellSize"))
        .unwrap_or(120.0);
    let w = dim(e, el, "width").unwrap_or(400.0);
    let height = dim(e, el, "height").unwrap_or(300.0);
    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut __gv = GridView::new({columns}, {}, {})?;",
        fmt_f32(cell_w),
        fmt_f32(cell_h)
    ));
    let mut child_env = env.clone();
    child_env.absorb_bindings(&el.children);
    let mut child_sigs = sigs.clone();
    declare_signals(e, &el.children, &child_env, &mut child_sigs);
    for node in &el.children {
        emit_node(e, node, &child_env, &child_sigs, comps, "__gv");
    }
    emit_widget_tail(
        e,
        el,
        "__gv",
        w,
        height,
        anchor_call(style::anchor(el)),
        sink,
    );
    e.dedent();
    e.line("}");
}

fn emit_textfield(e: &mut Emitter, el: &Element, _env: &Env, sink: &str) {
    let size = style::f32_prop(el, "size")
        .or_else(|| style::f32_prop(el, "fontSize"))
        .unwrap_or(16.0);
    let value = style::string_prop(el, "value")
        .or_else(|| style::string_prop(el, "text"))
        .unwrap_or_default();
    let w = dim(e, el, "width").unwrap_or(240.0);
    let h = dim(e, el, "height").unwrap_or(size + 20.0);
    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut __tf = TextField::new({:?}, {})?;",
        value,
        fmt_f32(size)
    ));
    if let Some(ph) = style::string_prop(el, "placeholder") {
        e.line(&format!("__tf = __tf.placeholder({ph:?})?;"));
    }
    e.line(&format!(
        "__tf = __tf.radius({});",
        fmt_f32(style::f32_prop(el, "radius").unwrap_or(8.0))
    ));
    if let Some(bg) = style::fill_only(el) {
        e.line(&format!("__tf = __tf.bg_color({});", color_lit(bg)));
    }
    if let Some(c) = style::color_prop(el, "textColor").or_else(|| style::color_prop(el, "color")) {
        e.line(&format!("__tf = __tf.text_color({});", color_lit(c)));
    }
    if let Some(c) = style::color_prop(el, "placeholderColor") {
        e.line(&format!("__tf = __tf.placeholder_color({});", color_lit(c)));
    }
    if let Some(c) = style::color_prop(el, "caretColor") {
        e.line(&format!("__tf = __tf.caret_color({});", color_lit(c)));
    }
    if let Some(c) = style::color_prop(el, "selectionColor") {
        e.line(&format!("__tf = __tf.selection_color({});", color_lit(c)));
    }
    if let Some(c) = style::color_prop(el, "borderColor") {
        e.line(&format!("__tf = __tf.border_color({});", color_lit(c)));
    }
    if let Some(c) = style::color_prop(el, "borderColorFocused")
        .or_else(|| style::color_prop(el, "focusBorderColor"))
    {
        e.line(&format!(
            "__tf = __tf.border_color_focused({});",
            color_lit(c)
        ));
    }
    if let Some(bw) = style::f32_prop(el, "borderWidth") {
        e.line(&format!("__tf = __tf.border_width({});", fmt_f32(bw)));
    }
    if let Some(p) = style::f32_prop(el, "padding") {
        let p = fmt_f32(p);
        e.line(&format!("__tf = __tf.padding({p}, {p});"));
    }
    emit_font(e, "__tf", el);
    if style::bool_prop(el, "password") == Some(true) {
        e.line("__tf = __tf.password(true);");
    }
    if let Some(ml) = style::f32_prop(el, "maxLength") {
        e.line(&format!("__tf = __tf.max_length({});", ml as i32));
    }
    if let Some(ms) =
        style::f32_prop(el, "caretBlink").or_else(|| style::f32_prop(el, "caretBlinkRate"))
    {
        e.line(&format!("__tf = __tf.caret_blink_rate({});", ms as i32));
    }
    if style::bool_prop(el, "placeholderAnimated") == Some(true) {
        e.line("__tf = __tf.placeholder_animated(true);");
    }
    if let Some(c) = style::enum_member(el, "cursor") {
        e.line(&format!("__tf = __tf.cursor({});", cursor_lit(&c)));
    }
    emit_widget_tail(e, el, "__tf", w, h, anchor_call(style::anchor(el)), sink);
    e.dedent();
    e.line("}");
}

/// `TextArea` — multi-line input (the dedicated `UITextArea`, not a stretched
/// field). Mirrors the runtime's `build_textarea`.
fn emit_textarea(e: &mut Emitter, el: &Element, _env: &Env, sink: &str) {
    let size = style::f32_prop(el, "size")
        .or_else(|| style::f32_prop(el, "fontSize"))
        .unwrap_or(16.0);
    let value = style::string_prop(el, "value")
        .or_else(|| style::string_prop(el, "text"))
        .unwrap_or_default();
    let w = dim(e, el, "width").unwrap_or(280.0);
    let h = dim(e, el, "height").unwrap_or(120.0);
    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut __ta = TextArea::new({:?}, {})?;",
        value,
        fmt_f32(size)
    ));
    if let Some(ph) = style::string_prop(el, "placeholder") {
        e.line(&format!("__ta = __ta.placeholder({ph:?})?;"));
    }
    e.line(&format!(
        "__ta = __ta.radius({});",
        fmt_f32(style::f32_prop(el, "radius").unwrap_or(8.0))
    ));
    if let Some(bg) = style::fill_only(el) {
        e.line(&format!("__ta = __ta.bg_color({});", color_lit(bg)));
    }
    if let Some(c) = style::color_prop(el, "textColor").or_else(|| style::color_prop(el, "color")) {
        e.line(&format!("__ta = __ta.text_color({});", color_lit(c)));
    }
    let border_c = style::color_prop(el, "borderColor");
    let border_w = style::f32_prop(el, "borderWidth");
    if border_c.is_some() || border_w.is_some() {
        let c = border_c
            .map(color_lit)
            .unwrap_or_else(|| "Color::rgb(203, 213, 225)".to_string());
        e.line(&format!(
            "__ta = __ta.border({c}, {c}, {});",
            fmt_f32(border_w.unwrap_or(1.0))
        ));
    }
    if let Some(p) = style::f32_prop(el, "padding") {
        let p = fmt_f32(p);
        e.line(&format!("__ta = __ta.padding({p}, {p});"));
    }
    if let Some(ls) = style::f32_prop(el, "lineSpacing") {
        e.line(&format!("__ta = __ta.line_spacing({});", fmt_f32(ls)));
    }
    if let Some(wm) = style::enum_member(el, "wrap").as_deref().and_then(wrap_lit) {
        e.line(&format!("__ta = __ta.wrap_mode({wm});"));
    }
    // TextArea's wrapper has no `font_style`; only the family applies.
    if let Some(fam) = style::font_family(el) {
        e.line(&format!(
            "if let Some(__p) = mocida::text::get_font({fam:?}) {{ __ta = __ta.font_family(&__p)?; }}"
        ));
    }
    if let Some(ml) = style::f32_prop(el, "maxLength") {
        e.line(&format!("__ta = __ta.max_length({});", ml as i32));
    }
    if let Some(c) = style::enum_member(el, "cursor") {
        e.line(&format!("__ta = __ta.cursor({});", cursor_lit(&c)));
    }
    emit_widget_tail(e, el, "__ta", w, h, anchor_call(style::anchor(el)), sink);
    e.dedent();
    e.line("}");
}

fn emit_checkbox(e: &mut Emitter, el: &Element, env: &Env, sigs: &SignalScope, sink: &str) {
    let on = eval_bool_prop(el, "value", env)
        .or_else(|| eval_bool_prop(el, "checked", env))
        .unwrap_or(false);
    let s = style::f32_prop(el, "size").unwrap_or(24.0);
    e.line("{");
    e.indent();
    e.line(&format!("let mut __cb = Checkbox::new({on})?;"));
    let box_c = style::fill_only(el).or_else(|| style::color_prop(el, "boxColor"));
    let check_c = style::color_prop(el, "checkColor").or_else(|| style::color_prop(el, "color"));
    match (box_c, check_c) {
        (Some(b), Some(c)) => e.line(&format!(
            "__cb = __cb.colors({}, {});",
            color_lit(b),
            color_lit(c)
        )),
        (Some(b), None) => e.line(&format!("__cb = __cb.box_color({});", color_lit(b))),
        (None, Some(c)) => e.line(&format!("__cb = __cb.check_color({});", color_lit(c))),
        (None, None) => {}
    }
    if let Some(bc) = style::color_prop(el, "borderColor") {
        e.line(&format!(
            "__cb = __cb.border({}, {});",
            color_lit(bc),
            fmt_f32(style::f32_prop(el, "borderWidth").unwrap_or(1.0))
        ));
    } else if let Some(bw) = style::f32_prop(el, "borderWidth") {
        e.line(&format!(
            "__cb = __cb.border(Color::rgb(148, 163, 184), {});",
            fmt_f32(bw)
        ));
    }
    if let Some(r) = style::f32_prop(el, "radius") {
        e.line(&format!("__cb = __cb.radius({});", fmt_f32(r)));
    }
    if let Some(ms) = style::f32_prop(el, "animMs") {
        e.line(&format!("__cb = __cb.anim_ms({});", ms as i32));
    }
    if let Some(c) = style::enum_member(el, "cursor") {
        e.line(&format!("__cb = __cb.cursor({});", cursor_lit(&c)));
    }
    emit_control_onchange(e, "__cb", el, sigs);
    emit_labeled_control(e, el, env, sigs, "__cb", s, s, 14.0, sink);
    e.dedent();
    e.line("}");
}

/// `RadioButton(label:, selected:, …)` — a radio dial with the full C style
/// surface and an optional caption beside it. Each radio gets its own (leaked,
/// opaque) group identity; mutual exclusivity is expected to come from a signal.
fn emit_radio(e: &mut Emitter, el: &Element, env: &Env, sigs: &SignalScope, sink: &str) {
    let selected = eval_bool_prop(el, "selected", env)
        .or_else(|| eval_bool_prop(el, "value", env))
        .or_else(|| eval_bool_prop(el, "checked", env))
        .unwrap_or(false);
    let dial = style::f32_prop(el, "size").unwrap_or(20.0);

    e.line("{");
    e.indent();
    // A unique, non-null, never-dereferenced group pointer.
    e.line("let __grp = Box::into_raw(Box::new(0u8)) as *mut std::ffi::c_void;");
    e.line(&format!(
        "let mut __r = unsafe {{ RadioButton::new(__grp, {selected})? }};"
    ));
    // `color`/`dotColor` set the accent DOT (not the disc — see runtime note);
    // `boxColor`/`bg` sets the disc (C default is white).
    let dot = style::color_prop(el, "dotColor").or_else(|| style::color_prop(el, "color"));
    let box_c = style::color_prop(el, "boxColor").or_else(|| style::fill_only(el));
    match (box_c, dot) {
        (Some(b), Some(d)) => e.line(&format!(
            "__r = __r.colors({}, {});",
            color_lit(b),
            color_lit(d)
        )),
        (Some(b), None) => e.line(&format!("__r = __r.box_color({});", color_lit(b))),
        (None, Some(d)) => e.line(&format!("__r = __r.dot_color({});", color_lit(d))),
        (None, None) => {}
    }
    if let Some(bc) = style::color_prop(el, "borderColor") {
        e.line(&format!(
            "__r = __r.border({}, {});",
            color_lit(bc),
            fmt_f32(style::f32_prop(el, "borderWidth").unwrap_or(1.0))
        ));
    } else if let Some(bw) = style::f32_prop(el, "borderWidth") {
        e.line(&format!(
            "__r = __r.border(Color::rgb(148, 163, 184), {});",
            fmt_f32(bw)
        ));
    }
    if let Some(ds) = style::f32_prop(el, "dotScale") {
        e.line(&format!("__r = __r.dot_scale({});", fmt_f32(ds)));
    }
    if let Some(ms) = style::f32_prop(el, "animMs") {
        e.line(&format!("__r = __r.anim_ms({});", ms as i32));
    }
    if let Some(cur) = style::enum_member(el, "cursor") {
        e.line(&format!("__r = __r.cursor({});", cursor_lit(&cur)));
    }
    if eval_bool_prop(el, "enabled", env) == Some(false) {
        e.line("__r = __r.enabled(false);");
    }
    emit_control_onchange(e, "__r", el, sigs);
    emit_labeled_control(e, el, env, sigs, "__r", dial, dial, 14.0, sink);
    e.dedent();
    e.line("}");
}

fn emit_switch(e: &mut Emitter, el: &Element, env: &Env, sigs: &SignalScope, sink: &str) {
    let on = eval_bool_prop(el, "value", env)
        .or_else(|| eval_bool_prop(el, "checked", env))
        .unwrap_or(false);
    let w = dim(e, el, "width").unwrap_or(48.0);
    let h = dim(e, el, "height").unwrap_or(28.0);
    e.line("{");
    e.indent();
    e.line(&format!("let mut __sw = Switch::new({on})?;"));
    let off_c = style::color_prop(el, "offColor");
    let on_c = style::color_prop(el, "onColor").or_else(|| style::color_prop(el, "color"));
    let knob_c = style::color_prop(el, "knobColor");
    match (off_c, on_c, knob_c) {
        (Some(o), Some(n), Some(k)) => e.line(&format!(
            "__sw = __sw.colors({}, {}, {});",
            color_lit(o),
            color_lit(n),
            color_lit(k)
        )),
        _ => {
            if let Some(o) = off_c {
                e.line(&format!("__sw = __sw.off_color({});", color_lit(o)));
            }
            if let Some(n) = on_c {
                e.line(&format!("__sw = __sw.on_color({});", color_lit(n)));
            }
            if let Some(k) = knob_c {
                e.line(&format!("__sw = __sw.knob_color({});", color_lit(k)));
            }
        }
    }
    if let Some(bc) = style::color_prop(el, "borderColor") {
        e.line(&format!(
            "__sw = __sw.border({}, {});",
            color_lit(bc),
            fmt_f32(style::f32_prop(el, "borderWidth").unwrap_or(1.0))
        ));
    } else if let Some(bw) = style::f32_prop(el, "borderWidth") {
        e.line(&format!(
            "__sw = __sw.border(Color::rgb(148, 163, 184), {});",
            fmt_f32(bw)
        ));
    }
    if let Some(ms) = style::f32_prop(el, "animMs") {
        e.line(&format!("__sw = __sw.anim_ms({});", ms as i32));
    }
    if let Some(c) = style::enum_member(el, "cursor") {
        e.line(&format!("__sw = __sw.cursor({});", cursor_lit(&c)));
    }
    emit_control_onchange(e, "__sw", el, sigs);
    emit_labeled_control(e, el, env, sigs, "__sw", w, h, 14.0, sink);
    e.dedent();
    e.line("}");
}

fn emit_slider(e: &mut Emitter, el: &Element, sink: &str) {
    let min = style::f32_prop(el, "min").unwrap_or(0.0);
    let max = style::f32_prop(el, "max").unwrap_or(100.0);
    let val = style::f32_prop(el, "value").unwrap_or(min);
    let w = dim(e, el, "width").unwrap_or(200.0);
    let h = dim(e, el, "height").unwrap_or(24.0);
    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut __sl = Slider::new({}, {}, {})?;",
        fmt_f32(min),
        fmt_f32(max),
        fmt_f32(val)
    ));
    let track = style::color_prop(el, "trackColor");
    let fill = style::color_prop(el, "fillColor").or_else(|| style::color_prop(el, "color"));
    let knob = style::color_prop(el, "knobColor");
    match (track, fill, knob) {
        (Some(t), Some(f), Some(k)) => e.line(&format!(
            "__sl = __sl.colors({}, {}, {});",
            color_lit(t),
            color_lit(f),
            color_lit(k)
        )),
        _ => {
            if let Some(t) = track {
                e.line(&format!("__sl = __sl.track_color({});", color_lit(t)));
            }
            if let Some(f) = fill {
                e.line(&format!("__sl = __sl.fill_color({});", color_lit(f)));
            }
            if let Some(k) = knob {
                e.line(&format!("__sl = __sl.knob_color({});", color_lit(k)));
            }
        }
    }
    if let Some(th) = style::f32_prop(el, "trackHeight") {
        e.line(&format!("__sl = __sl.track_height({});", fmt_f32(th)));
    }
    if let Some(kr) = style::f32_prop(el, "knobRadius") {
        e.line(&format!("__sl = __sl.knob_radius({});", fmt_f32(kr)));
    }
    if let Some(c) = style::enum_member(el, "cursor") {
        e.line(&format!("__sl = __sl.cursor({});", cursor_lit(&c)));
    }
    emit_widget_tail(e, el, "__sl", w, h, anchor_call(style::anchor(el)), sink);
    e.dedent();
    e.line("}");
}

fn emit_progressbar(e: &mut Emitter, el: &Element, sink: &str) {
    let val = style::f32_prop(el, "value").unwrap_or(0.0);
    let w = dim(e, el, "width").unwrap_or(200.0);
    let h = dim(e, el, "height").unwrap_or(8.0);
    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut __pb = ProgressBar::new({})?;",
        fmt_f32(val)
    ));
    let track = style::color_prop(el, "trackColor");
    let fill = style::color_prop(el, "fillColor").or_else(|| style::color_prop(el, "color"));
    match (track, fill) {
        (Some(t), Some(f)) => e.line(&format!(
            "__pb = __pb.colors({}, {});",
            color_lit(t),
            color_lit(f)
        )),
        _ => {
            if let Some(t) = track {
                e.line(&format!("__pb = __pb.track_color({});", color_lit(t)));
            }
            if let Some(f) = fill {
                e.line(&format!("__pb = __pb.fill_color({});", color_lit(f)));
            }
        }
    }
    if let Some(r) = style::f32_prop(el, "radius") {
        e.line(&format!("__pb = __pb.radius({});", fmt_f32(r)));
    }
    if style::bool_prop(el, "indeterminate") == Some(true)
        || style::bool_prop(el, "animated") == Some(true)
    {
        e.line("__pb = __pb.indeterminate(true);");
    }
    emit_widget_tail(e, el, "__pb", w, h, anchor_call(style::anchor(el)), sink);
    e.dedent();
    e.line("}");
}

fn emit_spinner(e: &mut Emitter, el: &Element, sink: &str) {
    let r = style::f32_prop(el, "radius").unwrap_or(16.0);
    let d = r * 2.0;
    e.line("{");
    e.indent();
    e.line(&format!("let mut __sp = Spinner::new({})?;", fmt_f32(r)));
    if let Some(c) = style::color_prop(el, "color").or_else(|| style::background(el)) {
        e.line(&format!("__sp = __sp.color({});", color_lit(c)));
    }
    if let Some(t) = style::f32_prop(el, "thickness") {
        e.line(&format!("__sp = __sp.thickness({});", fmt_f32(t)));
    }
    if let Some(s) = style::f32_prop(el, "speed") {
        e.line(&format!("__sp = __sp.speed({});", fmt_f32(s)));
    }
    emit_widget_tail(e, el, "__sp", d, d, anchor_call(style::anchor(el)), sink);
    e.dedent();
    e.line("}");
}

fn emit_image(e: &mut Emitter, el: &Element, env: &Env, sigs: &SignalScope, sink: &str) {
    let source = el
        .positional
        .as_ref()
        .map(|x| render_text_expr(x, env, sigs))
        .filter(|s| !s.is_empty())
        .or_else(|| style::string_prop(el, "source"))
        .or_else(|| style::string_prop(el, "src"))
        .unwrap_or_default();
    let fill = fill_mode_lit(style::enum_member(el, "fillMode").as_deref());
    let tint = style::color_prop(el, "tint")
        .map(color_lit)
        .unwrap_or_else(|| "Color::WHITE".to_string());
    let animated = style::bool_prop(el, "animated") == Some(true);
    let w = dim(e, el, "width").unwrap_or(120.0);
    let h = dim(e, el, "height").unwrap_or(120.0);
    e.line("{");
    e.indent();
    e.line(&format!(
        "let __img = Image::new({source:?}, {animated}, {fill}, {tint})?;"
    ));
    // `cache:` toggles the in-memory cache for http/https sources (default on).
    if style::bool_prop(el, "cache") == Some(false) {
        e.line("let __img = __img.cache(false);");
    }
    emit_widget_tail(e, el, "__img", w, h, anchor_call(style::anchor(el)), sink);
    e.dedent();
    e.line("}");
}

/// `Video("clip.mp4", width:, height:, fillMode:, radius:, autoplay:, loop:,
/// muted:, volume:)` — the same surface the runtime builds, lowered to a
/// `Video::load(...)` builder chain. `radius:` rounds the corners (cross-platform).
fn emit_video(e: &mut Emitter, el: &Element, env: &Env, sigs: &SignalScope, sink: &str) {
    let source = el
        .positional
        .as_ref()
        .map(|x| render_text_expr(x, env, sigs))
        .filter(|s| !s.is_empty())
        .or_else(|| style::string_prop(el, "source"))
        .or_else(|| style::string_prop(el, "src"))
        .unwrap_or_default();
    let w = dim(e, el, "width").unwrap_or(320.0);
    let h = dim(e, el, "height").unwrap_or(180.0);
    e.line("{");
    e.indent();
    e.line(&format!("let mut __vid = Video::load({source:?})?;"));
    if let Some(fm) = style::enum_member(el, "fillMode") {
        e.line(&format!(
            "__vid = __vid.fill_mode({});",
            fill_mode_lit(Some(fm.as_str()))
        ));
    }
    // `loop` is a Copper keyword (dropped before it becomes a prop) — accept the
    // keyword-safe `repeat:` too.
    if style::bool_prop(el, "loop") == Some(true) || style::bool_prop(el, "repeat") == Some(true) {
        e.line("__vid = __vid.loop_playback(true);");
    }
    if style::bool_prop(el, "muted") == Some(true) {
        e.line("__vid = __vid.muted(true);");
    }
    if let Some(v) = style::f32_prop(el, "volume") {
        e.line(&format!("__vid = __vid.volume({});", fmt_f32(v)));
    }
    if let Some(r) = style::f32_prop(el, "radius") {
        e.line(&format!("__vid = __vid.radius({});", fmt_f32(r)));
    }
    if style::bool_prop(el, "autoplay") == Some(true) {
        e.line("__vid.play();");
    }
    emit_widget_tail(e, el, "__vid", w, h, anchor_call(style::anchor(el)), sink);
    e.dedent();
    e.line("}");
}

/// `WebView("https://…", width:, height:, radius:, borderColor:, borderWidth:)`
/// — lowered to a `WebView::new(...)` builder chain (WebView2 / WKWebView /
/// WebKitGTK depending on platform).
fn emit_webview(e: &mut Emitter, el: &Element, env: &Env, sigs: &SignalScope, sink: &str) {
    let url = el
        .positional
        .as_ref()
        .map(|x| render_text_expr(x, env, sigs))
        .filter(|s| !s.is_empty())
        .or_else(|| style::string_prop(el, "url"))
        .or_else(|| style::string_prop(el, "src"))
        .unwrap_or_default();
    let w = dim(e, el, "width").unwrap_or(640.0);
    let h = dim(e, el, "height").unwrap_or(400.0);
    e.line("{");
    e.indent();
    if url.is_empty() {
        e.line("let mut __wv = WebView::new(None)?;");
    } else {
        e.line(&format!("let mut __wv = WebView::new(Some({url:?}))?;"));
    }
    if let Some(r) = style::f32_prop(el, "radius") {
        e.line(&format!("__wv = __wv.radius({});", fmt_f32(r)));
    }
    if let (Some(bc), Some(bw)) = (
        style::color_prop(el, "borderColor"),
        style::f32_prop(el, "borderWidth"),
    ) {
        e.line(&format!(
            "__wv = __wv.border({}, {});",
            color_lit(bc),
            fmt_f32(bw)
        ));
    }
    emit_widget_tail(e, el, "__wv", w, h, anchor_call(style::anchor(el)), sink);
    e.dedent();
    e.line("}");
}

/// `Audio("clip.wav", volume:/gain:, autoplay:)` — non-visual one-shot sound
/// (mocida's `UISound`). Loads the WAV, plays it on build (unless
/// `autoplay: false`), and parks it in the `__sounds` keep-alive so the clip
/// finishes. Emits no widget, so it never enters the layout.
fn emit_audio(e: &mut Emitter, el: &Element, env: &Env, sigs: &SignalScope) {
    let source = el
        .positional
        .as_ref()
        .map(|x| render_text_expr(x, env, sigs))
        .filter(|s| !s.is_empty())
        .or_else(|| style::string_prop(el, "source"))
        .or_else(|| style::string_prop(el, "src"))
        .unwrap_or_default();
    if source.is_empty() {
        return;
    }
    e.line("{");
    e.indent();
    e.line(&format!(
        "if let Ok(mut __snd) = Sound::load_wav({source:?}) {{"
    ));
    e.indent();
    if let Some(g) = style::f32_prop(el, "volume").or_else(|| style::f32_prop(el, "gain")) {
        e.line(&format!("__snd.set_gain({});", fmt_f32(g)));
    }
    if style::bool_prop(el, "autoplay") != Some(false) {
        e.line("__snd.play();");
    }
    e.line("__sounds.push(__snd);");
    e.dedent();
    e.line("}");
    e.dedent();
    e.line("}");
}

/// `Dialog(cardWidth:, cardHeight:, radius:, cardColor:/background:,
/// backdropColor:, dismissOnBackdrop:, visible:) { children }` — a modal-ish
/// overlay (mocida's `UIDialog`): a translucent backdrop plus a centered card
/// holding the children. Mirrors the runtime's `build_dialog`; children are
/// lowered into a vertical stack used as the card content.
fn emit_dialog(
    e: &mut Emitter,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    comps: &Registry,
    sink: &str,
) {
    let card_w = style::f32_prop(el, "cardWidth")
        .or_else(|| dim(e, el, "width"))
        .unwrap_or(360.0);
    let card_h = style::f32_prop(el, "cardHeight")
        .or_else(|| dim(e, el, "height"))
        .unwrap_or(200.0);
    e.line("{");
    e.indent();
    e.line(&format!(
        "let mut __dlg = Dialog::new({}, {})?;",
        fmt_f32(card_w),
        fmt_f32(card_h)
    ));
    if let Some(c) = style::color_prop(el, "cardColor").or_else(|| style::background(el)) {
        e.line(&format!("__dlg = __dlg.card_color({});", color_lit(c)));
    }
    if let Some(c) = style::color_prop(el, "backdropColor") {
        e.line(&format!("__dlg = __dlg.backdrop_color({});", color_lit(c)));
    }
    if let Some(r) = style::f32_prop(el, "radius") {
        e.line(&format!("__dlg = __dlg.radius({});", fmt_f32(r)));
    }
    if style::bool_prop(el, "dismissOnBackdrop") == Some(true) {
        e.line("__dlg = __dlg.dismiss_on_backdrop(true);");
    }
    if !el.children.is_empty() {
        e.line("{");
        e.indent();
        e.line("let mut __dc = Stack::new(StackOrientation::Vertical)?;");
        if let Some(gap) = style::f32_prop(el, "gap") {
            e.line(&format!("__dc = __dc.spacing({});", fmt_f32(gap)));
        }
        let mut child_env = env.clone();
        child_env.absorb_bindings(&el.children);
        let mut child_sigs = sigs.clone();
        declare_signals(e, &el.children, &child_env, &mut child_sigs);
        for node in &el.children {
            emit_node(e, node, &child_env, &child_sigs, comps, "__dc");
        }
        e.line(&format!(
            "__dlg.add_content(__dc.into_widget_sized({}, {})?)?;",
            fmt_f32(card_w),
            fmt_f32(card_h)
        ));
        e.dedent();
        e.line("}");
    }
    if style::bool_prop(el, "visible") != Some(false) {
        e.line("__dlg.show();");
    }
    // The dialog paints its own fullscreen backdrop + centered card, so it's
    // added straight to the sink rather than placed by the layout cursor.
    e.line(&format!("{sink}.add(__dlg.into_widget()?)?;"));
    e.dedent();
    e.line("}");
}

/// Render an [`Rgba`] as a `Color::rgba(...)` source expression.
fn color_lit(c: Rgba) -> String {
    format!(
        "Color::rgba({}, {}, {}, {})",
        c.r,
        c.g,
        c.b,
        fmt_f32(c.alpha_f32())
    )
}

/// Map a `cursor:` keyword to a `Cursor::*` source path.
fn cursor_lit(name: &str) -> &'static str {
    match name {
        "pointer" | "hand" => "Cursor::Pointer",
        "text" => "Cursor::Text",
        _ => "Cursor::Default",
    }
}

/// Emit the typography setters (`weight`/`fontStyle` → `font_style`, `font`/
/// `fontFamily` → `font_family`) for a `mut` builder var that exposes them
/// (Text / Button / TextField / TextArea). No-op when nothing is set. Mirrors
/// the runtime's `apply_text_font`.
fn emit_font(e: &mut Emitter, var: &str, el: &Element) {
    let bits = style::font_style_bits(el);
    if bits != 0 {
        // mocida's `FontStyle` has no `from_bits`; compose from the named
        // consts (BOLD/ITALIC/UNDERLINE/STRIKETHROUGH), which share the same
        // bit layout as `style::font_style_bits`.
        let mut flags = Vec::new();
        for (flag, name) in [
            (1 << 0, "BOLD"),
            (1 << 1, "ITALIC"),
            (1 << 2, "UNDERLINE"),
            (1 << 3, "STRIKETHROUGH"),
        ] {
            if bits & flag != 0 {
                flags.push(format!("FontStyle::{name}"));
            }
        }
        let expr = flags.join(" | ");
        e.line(&format!("{var} = {var}.font_style({expr});"));
    }
    if let Some(fam) = style::font_family(el) {
        e.line(&format!(
            "if let Some(__p) = mocida::text::get_font({fam:?}) {{ {var} = {var}.font_family(&__p)?; }}"
        ));
    }
}

/// `Color::rgba(...)` literal for a control's caption color: `textColor`, then
/// `labelColor` (never `color`, the control accent).
fn label_color_lit(el: &Element) -> Option<String> {
    style::color_prop(el, "textColor")
        .or_else(|| style::color_prop(el, "labelColor"))
        .map(color_lit)
}

/// Emit a styled caption Text into the local `__cap` (lifted to a widget),
/// sized to `box_h` and vertically centered so it lines up with the control's
/// center. Returns its width. Mirrors the runtime's `build_caption`.
fn emit_caption_widget(
    e: &mut Emitter,
    el: &Element,
    label: &str,
    default_size: f32,
    box_h: f32,
) -> f32 {
    let size = style::f32_prop(el, "labelSize")
        .or_else(|| style::f32_prop(el, "size"))
        .unwrap_or(default_size);
    let (tw, _th) = text_extent(label, size);
    e.line(&format!(
        "let mut __cap = Text::new({label:?}, {})?.v_align(TextVAlign::Center);",
        fmt_f32(size)
    ));
    if let Some(c) = label_color_lit(el) {
        e.line(&format!("__cap = __cap.color({c});"));
    }
    emit_font(e, "__cap", el);
    e.line(&format!(
        "let __cap = __cap.into_widget_sized({}, {})?;",
        fmt_f32(tw),
        fmt_f32(box_h)
    ));
    tw
}

/// Finish a labeled control: if `el` has a `label:` / positional caption, lift
/// the builder `bvar` and the caption into a horizontal row; else lift `bvar`
/// alone. Mirrors the runtime's `control_with_caption`.
#[allow(clippy::too_many_arguments)]
fn emit_labeled_control(
    e: &mut Emitter,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    bvar: &str,
    cw: f32,
    ch: f32,
    default_label_size: f32,
    sink: &str,
) {
    let label = style::string_prop(el, "label")
        .or_else(|| {
            el.positional
                .as_ref()
                .map(|x| render_text_expr(x, env, sigs))
        })
        .filter(|s| !s.is_empty());
    let anchor = anchor_call(style::anchor(el));
    match label {
        None => emit_widget_tail(e, el, bvar, cw, ch, anchor, sink),
        Some(label) => {
            e.line(&format!(
                "let __ctl = {bvar}.into_widget_sized({}, {})?;",
                fmt_f32(cw),
                fmt_f32(ch)
            ));
            let tw = emit_caption_widget(e, el, &label, default_label_size, ch);
            e.line("let mut __row = Stack::new(StackOrientation::Horizontal)?.spacing(8.0);");
            e.line("__row.add(__ctl)?;");
            e.line("__row.add(__cap)?;");
            emit_widget_tail(e, el, "__row", cw + 8.0 + tw, ch, anchor, sink);
        }
    }
}

/// `align:`/`hAlign:` → `TextHAlign::*`.
fn text_h_align_lit(name: &str) -> Option<&'static str> {
    match name {
        "left" | "start" => Some("TextHAlign::Left"),
        "center" | "middle" => Some("TextHAlign::Center"),
        "right" | "end" => Some("TextHAlign::Right"),
        _ => None,
    }
}

/// `vAlign:` → `TextVAlign::*`.
fn text_v_align_lit(name: &str) -> Option<&'static str> {
    match name {
        "top" => Some("TextVAlign::Top"),
        "center" | "middle" => Some("TextVAlign::Center"),
        "bottom" => Some("TextVAlign::Bottom"),
        _ => None,
    }
}

/// `wrap:` → `WrapMode::*`.
fn wrap_lit(name: &str) -> Option<&'static str> {
    match name {
        "none" | "false" => Some("WrapMode::None"),
        "word" | "true" => Some("WrapMode::Word"),
        "char" | "character" => Some("WrapMode::Char"),
        "fit" | "shrink" => Some("WrapMode::Fit"),
        _ => None,
    }
}

/// `fillMode:` → `FillMode::*` (full set).
fn fill_mode_lit(name: Option<&str>) -> &'static str {
    match name {
        Some("stretch") => "FillMode::Stretch",
        Some("scale") => "FillMode::Scale",
        Some("tile") => "FillMode::Tile",
        Some("center") => "FillMode::Center",
        Some("fit") => "FillMode::Fit",
        Some("fitwidth") => "FillMode::FitWidth",
        Some("fitheight") => "FillMode::FitHeight",
        Some("cover") => "FillMode::Cover",
        _ => "FillMode::None",
    }
}

/// A `.align_to_parent(V, H)` method-call suffix for an anchor, or `None` when
/// no axis is set. A missing axis defaults to Center (mocida needs both).
fn anchor_call(a: Anchor) -> Option<String> {
    if !a.is_set() {
        return None;
    }
    let v = match a.vertical {
        Some(VAnchor::Top) => "VerticalAlign::Top",
        Some(VAnchor::Bottom) => "VerticalAlign::Bottom",
        _ => "VerticalAlign::Center",
    };
    let h = match a.horizontal {
        Some(HAnchor::Left) => "HorizontalAlign::Left",
        Some(HAnchor::Right) => "HorizontalAlign::Right",
        _ => "HorizontalAlign::Center",
    };
    Some(format!(".align_to_parent({v}, {h})"))
}

fn emit_placeholder(
    e: &mut Emitter,
    name: &str,
    el: &Element,
    env: &Env,
    sigs: &SignalScope,
    sink: &str,
) {
    let inner = el
        .positional
        .as_ref()
        .map(|x| render_text_expr(x, env, sigs))
        .filter(|s| !s.is_empty())
        .map(|s| format!("{name}: {s}"))
        .unwrap_or_else(|| format!("[{name}]"));
    let (w, h) = text_extent(&inner, 14.0);
    e.line(&format!("// `{name}` not mapped yet — placeholder label"));
    e.line("{");
    e.indent();
    e.line(&format!(
        "let __t = Text::new({:?}, 14.0)?.color(Color::rgb(148, 163, 184));",
        inner
    ));
    e.line(&format!(
        "{sink}.add(__t.into_widget_sized({}, {})?.position({}))?;",
        fmt_f32(w),
        fmt_f32(h),
        position_args(el)
    ));
    e.line(&format!("__y += {};", fmt_f32(h + 8.0)));
    e.dedent();
    e.line("}");
}

/// Build the Rust `format!(...)` expression for a reactive text label: the
/// template with `{}` holes, filled by each read-signal's current value read
/// through its raw pointer (`UISignal_GetInt(__u_<name>)`) or a static value
/// for non-signal names. Reading via the raw pointer (not the `RefCell`) is
/// what keeps the subscription re-entrancy-safe — see `emit_text`.
fn reactive_format(
    positional: Option<&Expr>,
    env: &Env,
    sigs: &SignalScope,
    reads: &[String],
) -> String {
    let Some(e) = positional else {
        return "String::new()".to_string();
    };
    // Pull the template + the ordered arg names from the format-call shape.
    let (tmpl, args) = match &e.kind {
        ExprKind::Call { callee, args, .. } if is_format_call(callee) => {
            let tmpl = match args.first().map(|a| &a.kind) {
                Some(ExprKind::Literal(Literal::Str(t))) => render_template_raw(t),
                _ => String::new(),
            };
            let names: Vec<String> = args[1..]
                .iter()
                .map(|a| match &a.kind {
                    ExprKind::Ident(n) => n.clone(),
                    _ => String::new(),
                })
                .collect();
            (tmpl, names)
        }
        // A bare `${count}` (single ident) lowered without format.
        ExprKind::Ident(n) => ("{}".to_string(), vec![n.clone()]),
        _ => (render_text_expr(e, env, sigs), vec![]),
    };

    if args.is_empty() {
        return format!("{tmpl:?}.to_string()");
    }
    // Each arg: a live signal → its current value; else the static env value.
    let arg_exprs: Vec<String> = args
        .iter()
        .map(|n| {
            if reads.contains(n) {
                format!("unsafe {{ mocida::sys::UISignal_GetInt(__u_{n}) }}")
            } else if let Some(v) = env.get(n) {
                format!("{v:?}")
            } else {
                format!("{:?}", format!("{{{n}}}"))
            }
        })
        .collect();
    format!("format!({:?}, {})", tmpl, arg_exprs.join(", "))
}

/// The Rust expression computing the next value for a handler action, given
/// `__cur` holds the current value.
fn action_update_expr(action: &HandlerAction) -> String {
    match action {
        HandlerAction::AddAssign { delta, .. } => {
            if *delta >= 0 {
                format!("__cur + {delta}")
            } else {
                format!("__cur - {}", -delta)
            }
        }
        HandlerAction::SetInt { value, .. } => format!("{value}"),
    }
}

/// A named handler prop parsed into an action (int-signal subset).
fn handler_action(el: &Element, name: &str) -> Option<HandlerAction> {
    match &el.props.iter().find(|p| p.name == name)?.value {
        PropValue::Handler(h) => h.action(),
        _ => None,
    }
}

/// The `onKeyInput: { |event| ... }` handler, as `(param, raw_body)`. The body
/// is the space-joined token text (e.g. `if event . key == "Up" { score += 1 }`).
fn key_handler(el: &Element) -> Option<(String, String)> {
    match &el.props.iter().find(|p| p.name == "onKeyInput")?.value {
        PropValue::Handler(h) => {
            let param = h.params.first().cloned().unwrap_or_else(|| "event".into());
            Some((param, h.raw.clone()))
        }
        _ => None,
    }
}

/// Lower an `onKeyInput` body into Rust: the handler param's `.key` access
/// (e.g. `event.key`) becomes the bound `__key: &str`. Signal names are left
/// alone — the caller declares them as `let mut <name>` locals (loaded from the
/// signal before the body, stored back after), so `score += 1` / `score = 0` /
/// `if event.key == "Up" { ... }` all lower to valid Rust as-is.
fn lower_key_body(raw: &str, param: &str) -> String {
    let mut s = raw.to_string();
    // The tokenizer may emit `event . key` (spaced) or `event.key`.
    for pat in [format!("{param} . key"), format!("{param}.key")] {
        s = s.replace(&pat, "__key");
    }
    s
}

/// If `el` carries an `onKeyInput` handler, emit
/// `let <wvar> = <wvar>.on_key_down(move |__key, _mods| { ... });` translating
/// the body into live signal mutations. The signal clones it captures are
/// emitted into the *enclosing* scope first. Returns true when it emitted.
fn emit_key_handler(e: &mut Emitter, el: &Element, sigs: &SignalScope, wvar: &str) -> bool {
    let Some((param, raw)) = key_handler(el) else {
        return false;
    };
    // Signals the body actually touches (whole-token match), so we don't fire
    // spurious subscriptions for unrelated state.
    let toks: Vec<&str> = raw.split_whitespace().collect();
    let used: Vec<(String, String)> = sigs
        .vars
        .iter()
        .filter(|(name, _)| toks.contains(&name.as_str()))
        .map(|(n, v)| (n.clone(), v.clone()))
        .collect();
    let body = lower_key_body(&raw, &param);

    // Capture an Rc clone of each touched signal in the enclosing scope (so the
    // `move` closure — which on_key_down leaks for the program's life — owns it).
    for (name, var) in &used {
        e.line(&format!(
            "let __kc_{} = {var}.clone();",
            sanitize_ident(name)
        ));
    }
    e.line(&format!(
        "let {wvar} = {wvar}.on_key_down(move |__key: &str, _mods: i32| {{"
    ));
    e.indent();
    for (name, _) in &used {
        let id = sanitize_ident(name);
        e.line(&format!("let mut {name} = __kc_{id}.borrow().get();"));
    }
    // The lowered body is valid Rust statements; emit verbatim (one line is fine,
    // the generated crate is compiled, not formatted).
    e.line(&body);
    for (name, _) in &used {
        let id = sanitize_ident(name);
        e.line(&format!("let _ = __kc_{id}.borrow_mut().set({name});"));
    }
    e.dedent();
    e.line("});");
    true
}

/// The `onClick:` handler parsed into an action.
fn onclick_action(el: &Element) -> Option<HandlerAction> {
    handler_action(el, "onClick")
}

/// The state-changing handler of a toggle/value control: `onChange`, then
/// `onClick`, then `onToggle`.
fn control_action(el: &Element) -> Option<HandlerAction> {
    handler_action(el, "onChange")
        .or_else(|| handler_action(el, "onClick"))
        .or_else(|| handler_action(el, "onToggle"))
}

/// A boolean prop that may be a literal OR an expression resolved statically
/// against `env` (`selected: scope == "local"` → picks the right initial radio).
/// Mirrors the runtime's `eval_bool_prop`. `None` if absent / not boolean.
fn eval_bool_prop(el: &Element, name: &str, env: &Env) -> Option<bool> {
    match &el.props.iter().find(|p| p.name == name)?.value {
        PropValue::Expr(e) => eval_bool_expr(e, env),
        _ => None,
    }
}

fn eval_bool_expr(e: &Expr, env: &Env) -> Option<bool> {
    match &e.kind {
        ExprKind::Literal(Literal::Bool(b)) => Some(*b),
        ExprKind::Ident(_) => {
            let v = operand_value(e, env);
            Some(!(v.is_empty() || v == "false" || v == "0"))
        }
        ExprKind::Binary { op, lhs, rhs } => match op {
            BinOp::And => Some(eval_bool_expr(lhs, env)? && eval_bool_expr(rhs, env)?),
            BinOp::Or => Some(eval_bool_expr(lhs, env)? || eval_bool_expr(rhs, env)?),
            BinOp::Eq => Some(operand_value(lhs, env) == operand_value(rhs, env)),
            BinOp::Ne => Some(operand_value(lhs, env) != operand_value(rhs, env)),
            _ => None,
        },
        _ => None,
    }
}

/// Static value string of an expression for `==`/`!=`/truthiness: a literal
/// renders itself; an identifier resolves through `env`.
fn operand_value(e: &Expr, env: &Env) -> String {
    if let Some(s) = literal_display(e) {
        return s;
    }
    if let ExprKind::Ident(n) = &e.kind {
        return env.get(n).map(str::to_string).unwrap_or_default();
    }
    String::new()
}

/// Emit a `.on_change` wiring for a toggle control var (Checkbox/Switch/Radio)
/// when its handler is an interpretable int-signal action. No-op otherwise.
fn emit_control_onchange(e: &mut Emitter, var: &str, el: &Element, sigs: &SignalScope) {
    if let Some(action) = control_action(el) {
        if let Some(v) = sigs.var(action.name()) {
            e.line(&format!("let __h = {v}.clone();"));
            let update = action_update_expr(&action);
            e.line(&format!("{var} = {var}.on_change(move |_| {{"));
            e.indent();
            e.line("let __cur = __h.borrow().get();");
            e.line(&format!("let __next = {update};"));
            e.line("let _ = __h.borrow_mut().set(__next);");
            e.dedent();
            e.line("});");
        }
    }
}

// ---------------------------------------------------------------------------
// Static environment (param defaults + signal initial values)
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Env {
    vars: std::collections::HashMap<String, String>,
}

impl Env {
    fn from_view(view: &View) -> Self {
        let mut env = Env::default();
        for p in &view.params {
            if let Some(text) = p.default.as_ref().and_then(literal_display) {
                env.vars.insert(p.name.clone(), text);
            }
        }
        env
    }
    fn absorb_bindings(&mut self, nodes: &[Node]) {
        for n in nodes {
            if let Node::Let {
                name,
                value: Some(expr),
                ..
            } = n
            {
                // Resolve against the env built so far, so `count = signal(start)`
                // picks up `start`'s value (a param default or an earlier
                // binding), not just literal initializers.
                if let Some(text) = binding_display(expr, self) {
                    self.vars.insert(name.clone(), text);
                }
            }
        }
    }
    fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }
}

// ---------------------------------------------------------------------------
// Prop / expression helpers (mirror mui-runtime, but produce code or text)
// (Typed prop extraction lives in `mui_syntax::style`, shared with the runtime.)
// ---------------------------------------------------------------------------

/// Render a label for the **initial** draw. Signal names and param defaults
/// both resolve through `env` (signal initial values were folded in via
/// `absorb_bindings`); `sigs` is accepted for symmetry with the reactive path
/// and future use.
fn render_text_expr(e: &Expr, env: &Env, _sigs: &SignalScope) -> String {
    match &e.kind {
        ExprKind::Literal(Literal::Str(t)) => render_template(t, env),
        ExprKind::Literal(Literal::Int(i)) => i.to_string(),
        ExprKind::Literal(Literal::Float(f)) => f.to_string(),
        ExprKind::Literal(Literal::Bool(b)) => b.to_string(),
        ExprKind::Ident(n) => resolve_name(n, env),
        ExprKind::Call { callee, args, .. } if is_format_call(callee) => {
            render_format_call(args, env)
        }
        _ => String::new(),
    }
}

fn resolve_name(name: &str, env: &Env) -> String {
    env.get(name)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{{{name}}}"))
}

fn is_format_call(callee: &Expr) -> bool {
    matches!(&callee.kind, ExprKind::Ident(n) if n == "format" || n == "format!")
}

fn render_format_call(args: &[Expr], env: &Env) -> String {
    let Some((template, rest)) = args.split_first() else {
        return String::new();
    };
    let tmpl = match &template.kind {
        ExprKind::Literal(Literal::Str(t)) => render_template_raw(t),
        _ => return String::new(),
    };
    // Each interpolated arg is a literal or an ident; resolve through `env`
    // (signal initial values + param defaults were folded in upstream).
    let mut fills = rest.iter().map(|a| match &a.kind {
        ExprKind::Ident(n) => resolve_name(n, env),
        ExprKind::Literal(Literal::Int(i)) => i.to_string(),
        ExprKind::Literal(Literal::Float(f)) => f.to_string(),
        ExprKind::Literal(Literal::Bool(b)) => b.to_string(),
        ExprKind::Literal(Literal::Str(t)) => render_template_raw(t),
        _ => "{...}".to_string(),
    });
    let mut out = String::with_capacity(tmpl.len());
    let mut chars = tmpl.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'}') {
            chars.next();
            out.push_str(&fills.next().unwrap_or_else(|| "{...}".to_string()));
        } else {
            out.push(c);
        }
    }
    out
}

fn render_template_raw(t: &StrTemplate) -> String {
    t.parts
        .iter()
        .map(|p| match p {
            StrPart::Lit(s) => s.clone(),
            StrPart::Expr(_) => "{}".to_string(),
        })
        .collect()
}

fn render_template(t: &StrTemplate, env: &Env) -> String {
    let mut out = String::new();
    for part in &t.parts {
        match part {
            StrPart::Lit(s) => out.push_str(s),
            StrPart::Expr(e) => match &e.kind {
                ExprKind::Ident(n) => out.push_str(&resolve_name(n, env)),
                _ => out.push_str("{...}"),
            },
        }
    }
    out
}

fn literal_display(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Literal(Literal::Str(t)) => Some(render_template_raw(t).replace("{}", "")),
        ExprKind::Literal(Literal::Int(i)) => Some(i.to_string()),
        ExprKind::Literal(Literal::Float(f)) => Some(f.to_string()),
        ExprKind::Literal(Literal::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

fn binding_display(e: &Expr, env: &Env) -> Option<String> {
    match &e.kind {
        ExprKind::Call { callee, args, .. } if matches!(&callee.kind, ExprKind::Ident(n) if n == "signal") => {
            args.first().and_then(|a| value_display(a, env))
        }
        _ => value_display(e, env),
    }
}

/// Static display value of an expression: a literal renders itself; a bare
/// identifier resolves through `env` (param default / earlier binding).
fn value_display(e: &Expr, env: &Env) -> Option<String> {
    match &e.kind {
        ExprKind::Ident(n) => env.get(n).map(str::to_string),
        _ => literal_display(e),
    }
}

/// The initial value for a `signal(init)` binding, as a Rust `i32` literal
/// string. A literal int → itself; a param ident → its (numeric) default.
/// Returns `None` for non-integer signals (those stay out of the live set).
fn signal_init(e: &Expr, env: &Env) -> Option<String> {
    let ExprKind::Call { callee, args, .. } = &e.kind else {
        return None;
    };
    if !matches!(&callee.kind, ExprKind::Ident(n) if n == "signal") {
        return None;
    }
    match &args.first()?.kind {
        ExprKind::Literal(Literal::Int(i)) => Some(i.to_string()),
        ExprKind::Ident(n) => {
            let v = env.get(n)?;
            // Only accept it if it parses as an int (string params aren't i32).
            v.parse::<i64>().ok().map(|i| i.to_string())
        }
        _ => None,
    }
}

/// Identifier names an expression reads (for finding which signals a label
/// depends on). Walks the `format(tmpl, args...)` shape + bare idents.
fn names_read(e: &Expr) -> Vec<String> {
    let mut out = Vec::new();
    collect_idents(e, &mut out);
    out
}

fn collect_idents(e: &Expr, out: &mut Vec<String>) {
    match &e.kind {
        ExprKind::Ident(n) if !out.contains(n) => {
            out.push(n.clone());
        }
        ExprKind::Call { callee, args, .. } => {
            if !is_format_call(callee) {
                collect_idents(callee, out);
            }
            for a in args {
                collect_idents(a, out);
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            collect_idents(lhs, out);
            collect_idents(rhs, out);
        }
        _ => {}
    }
}

/// The state variable a handler action targets.
trait ActionName {
    fn name(&self) -> &str;
}

impl ActionName for HandlerAction {
    fn name(&self) -> &str {
        match self {
            HandlerAction::AddAssign { name, .. } | HandlerAction::SetInt { name, .. } => name,
        }
    }
}

// ---------------------------------------------------------------------------
// Small text/format utilities
// ---------------------------------------------------------------------------

/// Rough text bounds (matches the runtime's `text_extent`).
fn text_extent(label: &str, size: f32) -> (f32, f32) {
    let glyphs = label.chars().count().max(1) as f32;
    let w = (glyphs * size * 0.6).ceil().max(size);
    let h = (size * 1.25).ceil() + 4.0;
    (w, h)
}

/// Format an `f32` as a Rust float literal that always has a decimal point
/// (so `12` becomes `12.0`, a valid `f32` literal).
fn fmt_f32(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{v:.1}")
    } else {
        let s = format!("{v}");
        if s.contains('.') {
            s
        } else {
            format!("{s}.0")
        }
    }
}

/// Map a MUI/Copper param type to the Rust type the generated `fn` takes.
fn param_type(ty: Option<&str>) -> &'static str {
    match ty {
        Some("string") | Some("str") => "&str",
        Some("int") => "i64",
        Some("float") => "f64",
        Some("bool") => "bool",
        _ => "&str",
    }
}

/// A call-site argument for a param, using its default when present.
fn default_arg(default: Option<&Expr>, ty: Option<&str>) -> String {
    if let Some(d) = default {
        match &d.kind {
            ExprKind::Literal(Literal::Str(t)) => {
                format!("{:?}", render_template_raw(t).replace("{}", ""))
            }
            ExprKind::Literal(Literal::Int(i)) => i.to_string(),
            ExprKind::Literal(Literal::Float(f)) => fmt_f32(*f as f32),
            ExprKind::Literal(Literal::Bool(b)) => b.to_string(),
            _ => type_default(ty),
        }
    } else {
        type_default(ty)
    }
}

fn type_default(ty: Option<&str>) -> String {
    match ty {
        Some("int") => "0".to_string(),
        Some("float") => "0.0".to_string(),
        Some("bool") => "false".to_string(),
        _ => "\"\"".to_string(),
    }
}

fn view_fn_name(name: &str) -> String {
    // `view View` → `fn view_view` would be odd; use the view name lowercased
    // as a snake-ish function name, prefixed to avoid clashing with `main`.
    format!("build_{}", to_snake(name))
}

fn to_snake(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    sanitize_ident(&out)
}

/// Keep only identifier-safe characters (defensive — view/param names come
/// from the parser, but never trust unchecked text in generated code).
fn sanitize_ident(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(true)
    {
        out.insert(0, '_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen(src: &str) -> String {
        let doc = mui_syntax::parse(src);
        assert!(doc.errors.is_empty(), "parse errors: {:?}", doc.errors);
        generate_program(&doc, "MUI")
    }

    #[test]
    fn popup_floats_above_via_zindex_and_hides_when_invisible() {
        let code = gen("view P() {\n\
               Text(\"under\")\n\
               Popup(x: 40, y: 60, width: 200, height: 100) { Text(\"floating\") }\n\
               Popup(visible: false) { Text(\"hidden\") }\n\
             }\n");
        // Positioned overlay, lifted above siblings.
        assert!(
            code.contains("position(40"),
            "popup not positioned:\n{code}"
        );
        assert!(code.contains(".z_index(1000)"), "popup not lifted:\n{code}");
        // The invisible popup emits nothing.
        assert!(
            !code.contains("hidden"),
            "invisible popup rendered:\n{code}"
        );
    }

    #[test]
    fn dimension_exprs_resolve_window_id_and_arithmetic() {
        // `Window.*`, an `id:` widget's size, and arithmetic on them must lower
        // to constants — not fall back to the default size.
        let code = gen("App() {\n  width: 1280\n  height: 720\n}\n\
             view L() {\n\
               Stack(orientation: horizontal, width: Window.width, height: Window.height - 40) {\n\
                 Stack(id: side, orientation: vertical, width: 240, height: 100) {\n\
                   Rectangle(width: side.width, height: 1)\n\
                 }\n\
                 Rectangle(width: Window.width - 520, height: 200)\n\
               }\n\
             }\n");
        assert!(code.contains("1280"), "Window.width unresolved:\n{code}");
        assert!(
            code.contains("680"),
            "Window.height - 40 unresolved:\n{code}"
        );
        assert!(
            code.contains("760"),
            "Window.width - 520 unresolved:\n{code}"
        );
        // `side.width` (the id'd stack's 240) drives the inner rectangle.
        assert!(
            code.contains("into_widget_sized(240"),
            "id.width unresolved:\n{code}"
        );
    }

    #[test]
    fn emits_view_fn_and_main() {
        let code = gen("view Hello(name: string = \"world\") { Text(\"Hi, ${name}!\", size: 28) }");
        assert!(code.contains("fn build_hello(name: &str) -> MuiResult<BuiltView>"));
        assert!(code.contains("fn main() -> MuiResult<()>"));
        assert!(code.contains("App::new("));
        // The entry call uses the default.
        assert!(code.contains("build_hello(\"world\")"), "code:\n{code}");
    }

    #[test]
    fn emits_reactive_signal_button_and_subscription() {
        let code = gen(
            "view C(start: int = 0) {\n  mut count = signal(start)\n  Stack {\n    Text(\"Count: ${count}\")\n    Button(\"+\", onClick: { count = count + 1 })\n  }\n}",
        );
        // A real signal initialised from the param default.
        assert!(
            code.contains("Signal::<i32>::new(0)"),
            "signal init from param default:\n{code}"
        );
        // Reactive text: subscribes + updates via pointer.
        assert!(code.contains(".subscribe("), "text subscribes:\n{code}");
        assert!(
            code.contains("by_ptr::set_text("),
            "text updates by ptr:\n{code}"
        );
        // A real button wired to mutate the signal on click.
        assert!(code.contains("Button::new("), "real button:\n{code}");
        assert!(
            code.contains(".on_click("),
            "button has click handler:\n{code}"
        );
        assert!(code.contains("__cur + 1"), "increment expr:\n{code}");
    }

    #[test]
    fn radio_color_maps_to_dot_and_selected_expr_resolves() {
        // `color:` must paint the DOT, not the disc (else every radio looks
        // selected); `selected: scope == "x"` resolves against the signal's
        // initial value.
        let code = gen(
            "view V() {\n  mut scope = signal(\"local\")\n  RadioButton(label: \"Current\", selected: scope == \"local\", color: #d97757)\n}",
        );
        assert!(code.contains(".dot_color("), "color → dot_color:\n{code}");
        assert!(
            !code.contains(".colors(") || !code.contains("box"),
            "color alone must not set the box:\n{code}"
        );
        assert!(
            code.contains("RadioButton::new(__grp, true)"),
            "selected expr resolves to true:\n{code}"
        );
        // The caption is vertically centered.
        assert!(
            code.contains("v_align(TextVAlign::Center)"),
            "caption centered:\n{code}"
        );
    }

    #[test]
    fn substitutes_param_default_into_text() {
        let code = gen("view Hello(name: string = \"world\") { Text(\"Hi, ${name}!\") }");
        assert!(code.contains("\"Hi, world!\""), "code:\n{code}");
    }

    #[test]
    fn emits_color_and_stack() {
        let code = gen(
            "view V() { Stack(orientation: vertical, gap: 12) { Text(\"x\", color: #0f172a) } }",
        );
        assert!(code.contains("Stack::new(StackOrientation::Vertical)"));
        assert!(code.contains(".spacing(12.0)"));
        assert!(code.contains("Color::rgba(15, 23, 42,"), "code:\n{code}");
        assert!(code.contains("into_widget_sized"));
    }

    #[test]
    fn floats_always_have_decimal() {
        assert_eq!(fmt_f32(12.0), "12.0");
        assert_eq!(fmt_f32(8.0), "8.0");
        assert!(fmt_f32(12.5).starts_with("12.5"));
    }

    #[test]
    fn signal_of_param_resolves_through_env() {
        // The reported bug: `${count}` showed `{count}` because `count =
        // signal(start)` resolves through the *param* `start`, and the binding
        // lives at view-body level (a sibling of the Stack that uses it).
        let src = "view Counter(start: int = 0) {\n  mut count = signal(start)\n  Stack {\n    Text(\"Count: ${count}\")\n  }\n}";
        let code = gen(src);
        assert!(
            code.contains("\"Count: 0\""),
            "count should resolve to start's default (0), got:\n{code}"
        );
        assert!(
            !code.contains("{count}"),
            "no unresolved placeholder:\n{code}"
        );
    }
}
