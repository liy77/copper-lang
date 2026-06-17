// Smoke test for the macOS-aware toolbar styling (paddingTop /
// paddingLeft on Mac vs not). We use an inline mirror of the
// toolbar's root Stack so this test is self-contained (doesn't
// depend on OndaEngine being on the cargo path at test time).

use mui_syntax::style;
use mui_syntax::style::{box_spacing_reactive, ReactiveEnv};

const TOOLBAR_STACK: &str = r#"view EditorToolbar() {
  Stack(
    orientation: horizontal,
    gap: 0,
    align: center,
    justify: center,
    background: #2b2d30,
    height: 40,
    paddingTop:  ${is_macos == "1" ? 28 : 0},
    paddingLeft: ${is_macos == "1" ? 80 : 14},
    paddingRight: 14,
    width: Window.width
  ) {
    Image("mocida://onda-logo.png", width: 22, height: 22, fillMode: fit, marginRight: 10)
    if is_macos == "0" {
      Button("File", width: 46, height: 26, onClick: { file_menu_open = "1" })
      Button("Edit", width: 46, height: 26, onClick: { edit_menu_open = "1" })
    }
    Button("Save", width: 50, height: 26, onClick: { save_proj_clicked += 1 })
  }
}"#;

fn render_with_is_macos(is_macos: &str) -> (f32, f32, f32, f32) {
    let doc = mui_syntax::parse(TOOLBAR_STACK);
    let view = doc.views.first().expect("no view in toolbar mirror");
    let env = ReactiveEnv {
        signals: [("is_macos".into(), is_macos.into())]
            .into_iter()
            .collect(),
    };
    let el = find_el(&view.body, "Stack")
        .expect("no Stack element in toolbar mirror");
    box_spacing_reactive(el, "padding", Some(&env))
        .expect("padding not set on toolbar mirror")
}

fn find_el<'a>(
    nodes: &'a [mui_syntax::ast::Node],
    name: &str,
) -> Option<&'a mui_syntax::ast::Element> {
    for n in nodes {
        if let mui_syntax::ast::Node::Element(el) = n {
            if el.name == name {
                return Some(el);
            }
            if let Some(found) = find_el(&el.children, name) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn toolbar_padding_on_macos() {
    let (l, t, _r, _b) = render_with_is_macos("1");
    assert_eq!(t, 28.0, "paddingTop should be 28 on Mac (traffic-lights clear)");
    assert_eq!(l, 80.0, "paddingLeft should be 80 on Mac (traffic-lights clear)");
}

#[test]
fn toolbar_padding_on_non_macos() {
    let (l, t, _r, _b) = render_with_is_macos("0");
    assert_eq!(t, 0.0, "paddingTop should be 0 on Windows/Linux");
    assert_eq!(l, 14.0, "paddingLeft should be 14 on Windows/Linux");
}

// The Files panel renders the scene list inside a `Scroll` whose
// `height:` is bound to the `scene_h` signal. When the user drags
// the divider between the scene list and the file tree, `scene_h`
// updates — the Scroll's viewport should resize to match. This
// regression test pins the codegen's `dim_reactive` (added in
// commit 8a5540d) so the literal `height: scene_h` is no longer
// parsed as a metric ref, and the `${scene_h}` ternary IS
// evaluated against the live ReactiveEnv.
const FILES_PANEL_SCROLL: &str = r#"view FilesPanel() {
  Stack(orientation: vertical, width: 200) {
    Scroll(direction: vertical, height: ${scene_h}) {
      Stack(orientation: vertical, padding: 6) {
        Text("Scene A")
        Text("Scene B")
      }
    }
  }
}"#;

#[test]
fn files_panel_scroll_height_reactive() {
    // Build the AST and resolve the Scroll's `height:` with three
    // different `scene_h` values to confirm the codegen's reactive
    // path picks up the live signal snapshot (not the 240px default).
    let doc = mui_syntax::parse(FILES_PANEL_SCROLL);
    let view = doc.views.first().expect("no view");
    let scroll = find_el(&view.body, "Scroll").expect("no Scroll");
    for (scene_h, expected) in [("100", 100.0), ("240", 240.0), ("480", 480.0)] {
        let env = ReactiveEnv {
            signals: [
                ("is_macos".into(), "0".into()),
                ("scene_h".into(), scene_h.into()),
            ]
            .into_iter()
            .collect(),
        };
        let resolved = style::dim_prop_reactive(scroll, "height", &env)
            .expect("height should be set on Scroll");
        assert_eq!(resolved, expected,
            "Scroll height should follow scene_h (got {resolved} for scene_h={scene_h})");
    }
}
