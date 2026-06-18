//! Native std modules selectable via `import { ... } from <name>`. Each used
//! module is now emitted as a SEPARATE local crate (`__copper__/std/<name>/`):
//! main.rs gets a `use copper_<name> as <name>;` alias (no inlined `pub mod`),
//! the module's transpiled lib is recorded in `std_lib_crates()`, and any
//! crate-backed deps (e.g. `ureq` for http) belong to that module's crate
//! (`std_module_crate_dependencies`), not the main project.

use copper_parser::parser::Parser;
use copper_syntax::tokenizer::tokenizer::Tokenizer;

struct Out {
    rust: String,
    main_deps: Vec<String>,
    lib_crates: Vec<(String, String)>,
}

fn transpile(src: &str) -> Out {
    let tokens = Tokenizer::new(src.to_string()).tokenize();
    let mut p = Parser::new(tokens);
    let rust = p.parse();
    let main_deps = p.get_required_dependencies();
    let lib_crates = p.std_lib_crates().to_vec();
    Out { rust, main_deps, lib_crates }
}

impl Out {
    /// The transpiled lib source for module `name`, if it was emitted.
    fn lib(&self, name: &str) -> Option<&str> {
        self.lib_crates
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, s)| s.as_str())
    }
}

#[test]
fn net_module_is_a_crate_no_dep() {
    let o = transpile("import { resolve, local_ip } from net\nx = resolve(\"a:80\")\n");
    assert!(o.rust.contains("use copper_net as net;"), "alias missing: {}", o.rust);
    assert!(o.rust.contains("use net::"), "use net not emitted: {}", o.rust);
    assert!(!o.rust.contains("pub mod net {"), "net was inlined: {}", o.rust);
    assert!(o.lib("net").unwrap().contains("pub fn resolve("), "resolve not in net lib");
    assert!(Parser::std_module_crate_dependencies("net").is_empty(), "net has crate deps");
    assert!(!o.main_deps.iter().any(|d| d.starts_with("ureq")), "net pulled a main dep: {:?}", o.main_deps);
}

#[test]
fn http_module_is_a_crate_with_ureq_dep() {
    let o = transpile("import { get, get_status } from http\ns = get_status(\"https://x\")\n");
    assert!(o.rust.contains("use copper_http as http;"), "alias missing: {}", o.rust);
    // (cforge adds the `copper_http = { path = ... }` dep from the recorded
    // lib crate below; the parser emits the alias + records the lib.)
    assert!(o.lib("http").is_some(), "http lib not recorded: {:?}", o.lib_crates.iter().map(|(n, _)| n).collect::<Vec<_>>());
    assert!(!o.rust.contains("pub mod http {"), "http was inlined: {}", o.rust);
    assert!(o.lib("http").unwrap().contains("pub fn get("), "get not in http lib");
    // ureq belongs to the http CRATE, not the main project.
    assert!(Parser::std_module_crate_dependencies("http").iter().any(|d| d.starts_with("ureq")), "ureq dep missing");
    assert!(!o.main_deps.iter().any(|d| d.starts_with("ureq")), "ureq leaked to main: {:?}", o.main_deps);
}

#[test]
fn unused_std_modules_not_emitted() {
    let o = transpile("x = 1\nprintln!(\"{}\", x)\n");
    assert!(o.lib_crates.is_empty(), "modules emitted unprompted: {:?}", o.lib_crates);
    assert!(o.main_deps.is_empty(), "unexpected deps: {:?}", o.main_deps);
}

#[test]
fn both_modules_coexist_as_crates() {
    let o = transpile(
        "import { resolve } from net\nimport { get } from http\nx = resolve(\"a:1\")\ny = get(\"http://x\")\n",
    );
    assert!(o.rust.contains("use copper_net as net;") && o.rust.contains("use copper_http as http;"), "aliases: {}", o.rust);
    assert!(o.lib("net").is_some() && o.lib("http").is_some(), "libs: {:?}", o.lib_crates.iter().map(|(n, _)| n).collect::<Vec<_>>());
    assert!(Parser::std_module_crate_dependencies("http").iter().any(|d| d.starts_with("ureq")), "http crate ureq missing");
}

#[test]
fn url_module_is_a_crate_no_dep() {
    let o = transpile("import { encode, decode } from url\nx = encode(\"a b\")\n");
    assert!(o.rust.contains("use copper_url as url;"), "alias: {}", o.rust);
    assert!(o.lib("url").unwrap().contains("pub fn encode("), "encode not in url lib");
    assert!(Parser::std_module_crate_dependencies("url").is_empty(), "url has crate deps");
}

#[test]
fn json_module_crate_has_serde_dep() {
    let o = transpile("import { get, get_int } from json\nx = get(\"{}\", \"a\")\n");
    assert!(o.rust.contains("use copper_json as json;"), "alias: {}", o.rust);
    assert!(o.lib("json").unwrap().contains("pub fn get("), "get not in json lib");
    assert!(Parser::std_module_crate_dependencies("json").iter().any(|d| d.starts_with("serde_json")), "serde_json crate dep missing");
}

#[test]
fn crypto_module_crate_has_sha2_hmac() {
    let o = transpile("import { sha256, base64_encode } from crypto\nx = sha256(\"a\")\n");
    assert!(o.rust.contains("use copper_crypto as crypto;"), "alias: {}", o.rust);
    assert!(o.lib("crypto").unwrap().contains("pub fn sha256("), "sha256 not in crypto lib");
    let deps = Parser::std_module_crate_dependencies("crypto");
    assert!(deps.iter().any(|d| d.starts_with("sha2")), "sha2 crate dep missing");
    assert!(deps.iter().any(|d| d.starts_with("hmac")), "hmac crate dep missing");
}

#[test]
fn time_fs_ws_are_std_only_crates() {
    for (imp, fn_name, mod_name) in [
        ("import { now_ms } from time\nx = now_ms()\n", "now_ms", "time"),
        ("import { read } from fs\nx = read(\"a\")\n", "read", "fs"),
        ("import { request } from ws\nx = request(\"ws://a/\", \"m\")\n", "request", "ws"),
    ] {
        let o = transpile(imp);
        assert!(o.rust.contains(&format!("use copper_{mod_name} as {mod_name};")), "{mod_name} alias missing: {}", o.rust);
        assert!(o.lib(mod_name).unwrap().contains(&format!("pub fn {fn_name}(")), "{fn_name} not in {mod_name} lib");
        assert!(Parser::std_module_crate_dependencies(mod_name).is_empty(), "{mod_name} has crate deps");
        assert!(o.main_deps.is_empty(), "{mod_name} pulled a main dep: {:?}", o.main_deps);
    }
}
