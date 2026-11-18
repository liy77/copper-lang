//! Call-site auto-borrow: a value passed to `&str` / `&T` gets `&` inserted,
//! including associated functions on `class` / `impl` (`Type::name(arg)`)
//! and the explicit arguments of an instance method (`obj.name(arg)`).

use copper_syntax::tokenizer::tokenizer::Tokenizer;

fn transpile(src: &str) -> String {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    copper_parser::parser::parse(tokens)
}

#[test]
fn free_function_borrows_string_arg() {
    let rust = transpile(
        "func void greet(name: &str) {\n\
             println!(\"{}\", name)\n\
         }\n\
         mut who = \"ada\".to_string()\n\
         greet(who)\n",
    );
    assert!(
        rust.contains("greet(&who)") || rust.contains("greet(& who)"),
        "missing borrow at greet call:\n{rust}"
    );
}

#[test]
fn class_static_method_borrows_owned_string() {
    let rust = transpile(
        "class Greeter {\n\
             static void hello(name: &str) {\n\
                 println!(\"{}\", name)\n\
             }\n\
         }\n\
         mut who = \"ada\".to_string()\n\
         Greeter::hello(who)\n",
    );
    assert!(
        rust.contains("Greeter::hello(&who)") || rust.contains("Greeter::hello(& who)"),
        "missing borrow at Type::method call:\n{rust}"
    );
}

#[test]
fn class_instance_method_borrows_arg_not_receiver() {
    let rust = transpile(
        "class Greeter {\n\
             n: i32\n\
             Greeter(n: i32) {\n\
                 self.n = n\n\
             }\n\
             void hello(name: &str) {\n\
                 println!(\"{}\", name)\n\
             }\n\
         }\n\
         mut g = Greeter { n: 1 }\n\
         mut who = \"ada\".to_string()\n\
         g.hello(who)\n",
    );
    assert!(
        rust.contains("g.hello(&who)") || rust.contains("g.hello(& who)"),
        "missing borrow on instance-method arg:\n{rust}"
    );
    assert!(
        !rust.contains("(&g).hello") && !rust.contains("&g.hello"),
        "receiver was borrowed:\n{rust}"
    );
}

#[test]
fn cstd_run_keeps_cfg_windows_branch() {
    let src = include_str!("../../../std/cstd.crs");
    let rust = transpile(src);
    assert!(
        rust.contains("cfg!(target_os = \"windows\")") || rust.contains("cfg ! ( target_os"),
        "cstd::run lost cfg!:\n{rust}"
    );
    assert!(
        !rust.contains("let target_os"),
        "cfg! inside cstd::run became a let:\n{rust}"
    );
    assert!(
        rust.contains("cmd") && rust.contains("\"/C\""),
        "windows cmd branch missing:\n{rust}"
    );
}

#[test]
fn cfg_macro_is_not_a_let_binding() {
    let rust = transpile(
        "func String run(cmd: &str) {\n\
             if cfg!(target_os = \"windows\") {\n\
                 return cmd.to_string()\n\
             }\n\
             return cmd.to_string()\n\
         }\n",
    );
    assert!(
        rust.contains("cfg!(target_os = \"windows\")") || rust.contains("cfg !"),
        "cfg! was rewritten:\n{rust}"
    );
    assert!(
        !rust.contains("let target_os"),
        "cfg! assignment became a let:\n{rust}"
    );
}
