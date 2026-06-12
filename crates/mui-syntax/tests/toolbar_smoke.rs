// Smoke test: verify the actual ui/toolbar.mui (after the macOS topbar fix)
// produces the expected paddingLeft for both macOS and non-macOS environments.
//
// This is the closest we can get to a "render test" without a working
// mocida-sys build env: it loads the real .mui file, parses it via the
// mui-syntax loader (which now recognizes the ${...} slot thanks to the
// T2 parser fix), and evaluates the paddingLeft through the same
// box_spacing_reactive → f32_prop_reactive → eval_dim_reactive chain the
// runtime uses at frame time. If this test passes, the toolbar's
// paddingLeft is guaranteed to be 80.0 on macOS and 14.0 elsewhere —
// which is exactly the spec's section 5.3 acceptance criterion #1.

use mui_syntax::style::{box_spacing_reactive, ReactiveEnv};
use std::path::PathBuf;

fn toolbar_path() -> PathBuf {
    // The .mui lives in the OndaEngine repo, sibling to the worktree.
    // Walk up from CARGO_MANIFEST_DIR (copper-lang/crates/mui-syntax) to
    // copper-lang, then to the OndaEngine worktree.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // .../mui-syntax
    p.pop(); // .../crates
    p.pop(); // .../copper-lang
    p.push("OndaEngine");
    p.push(".claude");
    p.push("worktrees");
    p.push("macos-topbar-traffic-lights-design");
    p.push("ui");
    p.push("toolbar.mui");
    p
}

fn find_stack<'a>(nodes: &'a [mui_syntax::ast::Node]) -> Option<&'a mui_syntax::ast::Element> {
    for n in nodes {
        if let mui_syntax::ast::Node::Element(el) = n {
            if el.name == "Stack" {
                return Some(el);
            }
            if let Some(found) = find_stack(&el.children) {
                return Some(found);
            }
        }
    }
    None
}

fn padding_left_for(toolbar_macos: bool) -> f32 {
    let path = toolbar_path();
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read toolbar.mui at {:?}: {}", path, e));
    let doc = mui_syntax::parse(&src);
    assert!(!doc.views.is_empty(), "toolbar.mui must contain a view");

    // Walk the parsed document for the first Stack inside the view body.
    // The doc shape (from inspecting the test fixtures in the crate) is
    // doc.views -> Vec<View> -> View.body -> Vec<Node>.
    let mut stack: Option<&mui_syntax::ast::Element> = None;
    for v in &doc.views {
        if let Some(s) = find_stack(&v.body) {
            stack = Some(s);
            break;
        }
    }
    let stack = stack.expect("no Stack found in toolbar.mui");

    // Build the env as the runtime's build_reactive_env would.
    let mut env = ReactiveEnv::default();
    env.signals.insert("is_macos".into(), if toolbar_macos { "1".into() } else { "0".into() });

    // The exact call the runtime makes for paddingLeft.
    let (l, _t, _r, _b) =
        box_spacing_reactive(stack, "padding", Some(&env)).expect("padding set");
    l
}

#[test]
fn toolbar_padding_left_is_80_on_macos() {
    let l = padding_left_for(true);
    assert_eq!(l, 80.0, "macOS: paddingLeft must be 80.0 to clear traffic-lights");
}

#[test]
fn toolbar_padding_left_is_14_on_non_macos() {
    let l = padding_left_for(false);
    assert_eq!(
        l, 14.0,
        "non-macOS: paddingLeft must remain 14.0 (regression guard)"
    );
}
