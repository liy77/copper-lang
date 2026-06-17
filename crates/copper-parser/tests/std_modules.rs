//! Native std modules selectable via `import { ... } from <name>`:
//! `net` (std-only TCP/UDP) and `http` (HTTP/HTTPS via the ureq crate).
//! The module body must be bundled as `pub mod <name> { ... }`, and `http`
//! must pull `ureq` into the required dependencies; `net` must not.

use copper_parser::parser::Parser;
use copper_syntax::tokenizer::tokenizer::Tokenizer;

fn transpile_with_deps(src: &str) -> (String, Vec<String>) {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    let mut p = Parser::new(tokens);
    let out = p.parse();
    let deps = p.get_required_dependencies();
    (out, deps)
}

#[test]
fn net_module_is_bundled_no_dep() {
    let (rust, deps) = transpile_with_deps(
        "import { resolve, local_ip } from net\nx = resolve(\"a:80\")\n",
    );
    assert!(rust.contains("pub mod net {"), "net module not bundled: {rust}");
    assert!(rust.contains("pub fn resolve("), "resolve not promoted: {rust}");
    assert!(rust.contains("use net::"), "use net not emitted: {rust}");
    assert!(!deps.iter().any(|d| d == "ureq"), "net pulled a dep: {deps:?}");
}

#[test]
fn http_module_is_bundled_with_ureq_dep() {
    let (rust, deps) = transpile_with_deps(
        "import { get, get_status } from http\ns = get_status(\"https://x\")\n",
    );
    assert!(rust.contains("pub mod http {"), "http module not bundled: {rust}");
    assert!(rust.contains("pub fn get("), "get not promoted: {rust}");
    assert!(deps.iter().any(|d| d == "ureq"), "ureq dep missing: {deps:?}");
}

#[test]
fn unused_std_modules_not_bundled() {
    let (rust, deps) = transpile_with_deps("x = 1\nprintln!(\"{}\", x)\n");
    assert!(!rust.contains("pub mod net"), "net bundled unprompted: {rust}");
    assert!(!rust.contains("pub mod http"), "http bundled unprompted: {rust}");
    assert!(deps.is_empty(), "unexpected deps: {deps:?}");
}

#[test]
fn both_modules_coexist() {
    let (rust, deps) = transpile_with_deps(
        "import { resolve } from net\nimport { get } from http\nx = resolve(\"a:1\")\ny = get(\"http://x\")\n",
    );
    assert!(rust.contains("pub mod net {") && rust.contains("pub mod http {"), "got: {rust}");
    assert!(deps.iter().any(|d| d == "ureq"), "deps: {deps:?}");
}


#[test]
fn url_module_is_bundled_no_dep() {
    let (rust, deps) = transpile_with_deps(
        "import { encode, decode } from url
x = encode(\"a b\")
",
    );
    assert!(rust.contains("pub mod url {"), "url not bundled: {rust}");
    assert!(rust.contains("pub fn encode("), "encode not promoted: {rust}");
    assert!(!deps.iter().any(|d| d == "ureq" || d == "serde_json"), "url pulled a dep: {deps:?}");
}

#[test]
fn json_module_is_bundled_with_serde_dep() {
    let (rust, deps) = transpile_with_deps(
        "import { get, get_int } from json
x = get(\"{}\", \"a\")
",
    );
    assert!(rust.contains("pub mod json {"), "json not bundled: {rust}");
    assert!(rust.contains("pub fn get("), "get not promoted: {rust}");
    assert!(deps.iter().any(|d| d == "serde_json"), "serde_json dep missing: {deps:?}");
}


#[test]
fn crypto_module_bundles_with_sha2_hmac() {
    let (rust, deps) = transpile_with_deps(
        "import { sha256, base64_encode } from crypto
x = sha256(\"a\")
",
    );
    assert!(rust.contains("pub mod crypto {"), "crypto not bundled: {rust}");
    assert!(rust.contains("pub fn sha256("), "sha256 not promoted: {rust}");
    assert!(deps.iter().any(|d| d == "sha2"), "sha2 dep missing: {deps:?}");
    assert!(deps.iter().any(|d| d == "hmac"), "hmac dep missing: {deps:?}");
}

#[test]
fn time_fs_ws_are_std_only() {
    for (imp, fn_name, mod_name) in [
        ("import { now_ms } from time
x = now_ms()
", "now_ms", "time"),
        ("import { read } from fs
x = read(\"a\")
", "read", "fs"),
        ("import { request } from ws
x = request(\"ws://a/\", \"m\")
", "request", "ws"),
    ] {
        let (rust, deps) = transpile_with_deps(imp);
        assert!(rust.contains(&format!("pub mod {mod_name} {{")), "{mod_name} not bundled: {rust}");
        assert!(rust.contains(&format!("pub fn {fn_name}(")), "{fn_name} not promoted: {rust}");
        assert!(deps.is_empty(), "{mod_name} pulled a dep: {deps:?}");
    }
}
