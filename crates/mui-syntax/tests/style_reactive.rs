// Tests for the reactive style evaluator (f32_prop_reactive / box_spacing_reactive).
// These are the unit-level cases from the spec section 5.1.

use mui_syntax::ast::{Element, Node};
use mui_syntax::style::{box_spacing_reactive, f32_prop_reactive, ReactiveEnv};

fn env_with(pairs: &[(&str, &str)]) -> ReactiveEnv {
    ReactiveEnv {
        signals: pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
    }
}

fn f32_with_env(src: &str, prop: &str, env: &ReactiveEnv) -> Option<f32> {
    let nodes = parse(src);
    let el = find_el(&nodes, "Stack");
    f32_prop_reactive(el, prop, Some(env))
}

fn parse(src: &str) -> Vec<Node> {
    // The crate-level `parse` returns a Document; the only view in the test
    // sources below is `V`, so take its body directly.
    let doc = mui_syntax::parse(src);
    let view = doc
        .views
        .first()
        .expect("test source should define a single view");
    view.body.clone()
}

fn find_el<'a>(nodes: &'a [Node], name: &str) -> &'a Element {
    match find_deep(nodes, name) {
        Some(el) => el,
        None => panic!("element <{name}> not found"),
    }
}

fn find_deep<'a>(nodes: &'a [Node], name: &str) -> Option<&'a Element> {
    for n in nodes {
        if let Node::Element(el) = n {
            if el.name == name {
                return Some(el);
            }
            if let Some(found) = find_deep(&el.children, name) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn literal_int_static() {
    let nodes = parse("view V() { Stack(paddingLeft: 14) {} }");
    let el = find_el(&nodes, "Stack");
    assert_eq!(f32_prop_reactive(el, "paddingLeft", None), Some(14.0));
}

#[test]
fn literal_int_with_env() {
    // A literal stays a literal regardless of env.
    let nodes = parse("view V() { Stack(paddingLeft: 14) {} }");
    let el = find_el(&nodes, "Stack");
    let env = ReactiveEnv {
        signals: [("is_macos".into(), "1".into())].into_iter().collect(),
    };
    assert_eq!(f32_prop_reactive(el, "paddingLeft", Some(&env)), Some(14.0));
}

#[test]
fn ternary_macos_true() {
    let src = r#"view V() { Stack(paddingLeft: ${is_macos == "1" ? 80 : 14}) {} }"#;
    let env = env_with(&[("is_macos", "1")]);
    assert_eq!(f32_with_env(src, "paddingLeft", &env), Some(80.0));
}

#[test]
fn ternary_macos_false() {
    let src = r#"view V() { Stack(paddingLeft: ${is_macos == "1" ? 80 : 14}) {} }"#;
    let env = env_with(&[("is_macos", "0")]);
    assert_eq!(f32_with_env(src, "paddingLeft", &env), Some(14.0));
}

#[test]
fn arithmetic_signal() {
    // ${is_macos * 66 + 14} = 80 when is_macos=1, 14 when is_macos=0
    let src = r"view V() { Stack(paddingLeft: ${is_macos * 66 + 14}) {} }";
    assert_eq!(f32_with_env(src, "paddingLeft", &env_with(&[("is_macos", "1")])), Some(80.0));
    assert_eq!(f32_with_env(src, "paddingLeft", &env_with(&[("is_macos", "0")])), Some(14.0));
}

#[test]
fn missing_signal_returns_none() {
    let src = r"view V() { Stack(paddingLeft: ${is_macos * 66 + 14}) {} }";
    let env = ReactiveEnv::default();
    assert_eq!(f32_with_env(src, "paddingLeft", &env), None);
}

#[test]
fn box_spacing_reactive_ternary_full() {
    // paddingLeft=${...}, paddingRight=14, paddingTop=4, paddingBottom=4
    let src = r#"view V() { Stack(paddingLeft: ${is_macos == "1" ? 80 : 14}, paddingRight: 14, paddingTop: 4, paddingBottom: 4) {} }"#;
    let nodes = parse(src);
    let el = find_el(&nodes, "Stack");
    let env = env_with(&[("is_macos", "1")]);
    let (l, t, r, b) = box_spacing_reactive(el, "padding", Some(&env)).expect("set");
    assert_eq!((l, t, r, b), (80.0, 4.0, 14.0, 4.0));
}

#[test]
fn box_spacing_reactive_no_env_falls_back_to_static() {
    // No env → identical to old box_spacing behavior.
    let src = r"view V() { Stack(paddingLeft: 14, paddingRight: 14) {} }";
    let nodes = parse(src);
    let el = find_el(&nodes, "Stack");
    let (l, _t, r, _b) = box_spacing_reactive(el, "padding", None).expect("set");
    assert_eq!((l, r), (14.0, 14.0));
}

#[test]
fn box_spacing_reactive_array_form_still_works() {
    // The array form `[t, r, b, l]` is a static-only shortcut (every entry
    // must be a literal — a reactive value falls out of array_f32). This
    // test pins that behavior in the reactive path: a fully-literal array
    // still resolves via array_f32 regardless of env.
    let src = r"view V() { Stack(padding: [4, 8, 12, 16]) {} }";
    let nodes = parse(src);
    let el = find_el(&nodes, "Stack");
    let env = env_with(&[("is_macos", "1")]);
    let (l, t, r, b) = box_spacing_reactive(el, "padding", Some(&env)).expect("set");
    assert_eq!((l, t, r, b), (16.0, 4.0, 8.0, 12.0));
}
