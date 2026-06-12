// Tests for the reactive style evaluator (f32_prop_reactive / box_spacing_reactive).
// These are the unit-level cases from the spec section 5.1.

use mui_syntax::ast::{Element, Node};
use mui_syntax::style::{f32_prop_reactive, ReactiveEnv};

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
fn debug_expr_shape() {
    use mui_syntax::ast::PropValue;
    let src = r#"view V() { Stack(paddingLeft: ${is_macos * 66 + 14}) {} }"#;
    let doc = mui_syntax::parse(src);
    eprintln!("errors: {:?}", doc.errors);
    let body = &doc.views[0].body;
    if let Some(Node::Element(stack)) = body.first() {
        if let Some(p) = stack.props.iter().find(|p| p.name == "paddingLeft") {
            if let PropValue::Expr(e) = &p.value {
                eprintln!("paddingLeft expr: {:#?}", e);
            }
        }
    }
    let src2 = r#"view V() { Stack(paddingLeft: ${is_macos == "1" ? 80 : 14}) {} }"#;
    let doc2 = mui_syntax::parse(src2);
    eprintln!("\nternary errors: {:?}", doc2.errors);
    let body2 = &doc2.views[0].body;
    if let Some(Node::Element(stack)) = body2.first() {
        if let Some(p) = stack.props.iter().find(|p| p.name == "paddingLeft") {
            if let PropValue::Expr(e) = &p.value {
                eprintln!("ternary paddingLeft expr: {:#?}", e);
            }
        }
    }
}
