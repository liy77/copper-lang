// Smoke test for the macOS-aware toolbar styling (paddingTop /
// paddingLeft on Mac vs not). We use an inline mirror of the
// toolbar's root Stack so this test is self-contained (doesn't
// depend on OndaEngine being on the cargo path at test time).

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
