//! Multi-module Copper (`crs/**/*.crs`) transpiler, post-processor, and asset/template
//! embedder built directly into `cforge`.
//!
//! Eliminates the need for per-crate `build.rs` files, `Cargo.toml` manifests,
//! or `src/*.rs` stubs in Copper projects such as `mocidad-engine`,
//! `engine/native-host`, and `mocidad-launcher`.

use crate::cforge::cache::{hash_source, write_if_changed, TranspileCache, TranspileEntry};
use crate::parser::parse;
use crate::tokenizer::tokenizer::Tokenizer;
use std::path::{Path, PathBuf};

/// Transpile an entire Copper project directory (`<project_dir>/crs/**/*.crs`,
/// plus optional `<project_dir>/mocidad_logic.crs`, `../app.bundle`, and `../templates`)
/// into `<project_dir>/dist/rust/src/`, and generate `<project_dir>/dist/rust/Cargo.toml`
/// from `<project_dir>/properties.kson`.
pub fn transpile_crs_project(project_dir: &Path) -> Result<PathBuf, String> {
    let project_dir = project_dir
        .canonicalize()
        .map_err(|e| format!("canonicalize {}: {e}", project_dir.display()))?;
    let project_dir_str = clean_win_path(&project_dir);

    let crs_dir = project_dir.join("crs");
    if !crs_dir.is_dir() {
        return Err(format!("No crs/ directory in {}", project_dir.display()));
    }

    let dist_dir = project_dir.join("dist");
    let rust_dir = dist_dir.join("rust");
    let src_dir = rust_dir.join("src");
    std::fs::create_dir_all(&src_dir).map_err(|e| e.to_string())?;
    let src_dir_str = clean_win_path(&src_dir);
    let dist_dir_str = clean_win_path(&dist_dir);

    let mut cache = TranspileCache::load(&dist_dir_str, crate::cforge::VERSION);

    // 1. Optional root `mocidad_logic.crs`
    let logic_crs = project_dir.join("mocidad_logic.crs");
    if logic_crs.is_file() {
        let src = std::fs::read_to_string(&logic_crs).map_err(|e| e.to_string())?;
        let mut rust = transpile("mocidad_logic", &src);
        if let Some(i) = rust.find("fn main()") {
            let tail = rust[i..].trim();
            let trivial = tail
                .strip_prefix("fn main()")
                .map(|r| {
                    let inner = r
                        .trim()
                        .trim_start_matches('{')
                        .trim_end_matches('}')
                        .trim();
                    inner.is_empty() || inner.chars().all(|c| c == ';' || c.is_whitespace())
                })
                .unwrap_or(false);
            if trivial {
                rust.truncate(i);
            }
        }
        let _ = write_if_changed(src_dir.join("mocidad_logic.rs"), &rust);
    }

    // 2. Optional `../templates` and `../app.bundle` (used by mocidad-launcher)
    if let Some(parent) = project_dir.parent() {
        let templates_dir = parent.join("templates");
        if templates_dir.is_dir() && logic_crs.is_file() {
            embed_templates(&templates_dir, &src_dir);
        }
        let bundle_file = parent.join("app.bundle");
        if bundle_file.is_file() && logic_crs.is_file() {
            embed_assets(parent, &bundle_file, &src_dir);
        }
    }

    // 3. Collect all `crs/**/*.crs`
    let mut entries: Vec<PathBuf> = Vec::new();
    collect_crs(&crs_dir, &mut entries);
    entries.sort();

    let mut modules: Vec<String> = Vec::new();
    let mut has_lib_crs = false;
    let mut has_main_crs = false;

    for path in &entries {
        let rel = path.strip_prefix(&crs_dir).unwrap_or(path);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.starts_with("bin/") {
            let bin_stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let rust = transpile(&bin_stem, &src);
            let bin_dir = src_dir.join("bin");
            let _ = std::fs::create_dir_all(&bin_dir);
            let _ = write_if_changed(bin_dir.join(format!("{bin_stem}.rs")), &rust);
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if name == "lib" {
            has_lib_crs = true;
            continue;
        }
        if name == "main" {
            has_main_crs = true;
            continue;
        }
        if name == "main_host" && crs_dir.join("main.crs").is_file() {
            continue;
        }

        let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let key = path.to_string_lossy().replace('\\', "/");
        let src_hash = hash_source(&src, crate::cforge::VERSION);
        let dest = src_dir.join(format!("{name}.rs"));
        if cache.get_valid(&key, &src_hash).is_none() {
            let mut r = transpile(&name, &src);
            r = rewrite_manifest_and_out_dir(&r, &project_dir_str, &src_dir_str);
            let _ = write_if_changed(&dest, &r);
            cache.insert(
                key,
                TranspileEntry {
                    hash: src_hash,
                    output_path: clean_win_path(&dest),
                    dependencies: Vec::new(),
                },
            );
        }
        modules.push(name);
    }

    let list = modules
        .iter()
        .map(|m| format!("    {m:?},\n"))
        .collect::<String>();
    let _ = write_if_changed(
        src_dir.join("crs_modules.rs"),
        &format!("pub const CRS_MODULES: &[&str] = &[\n{list}];\n"),
    );

    // 4. Generate `src/lib.rs` if `crs/lib.crs` exists
    if has_lib_crs {
        let lib_src =
            std::fs::read_to_string(crs_dir.join("lib.crs")).map_err(|e| e.to_string())?;
        let mut lib_rust = transpile("lib", &lib_src);
        lib_rust = rewrite_manifest_and_out_dir(&lib_rust, &project_dir_str, &src_dir_str);
        let mut mod_decls = String::from(
            "#![allow(dead_code, unused_imports, unused_mut, unused_variables, unused_parens, non_snake_case, clippy::all)]\n",
        );
        for m in &modules {
            mod_decls.push_str(&format!("pub mod {m};\n"));
        }
        mod_decls.push('\n');
        mod_decls.push_str(&lib_rust);

        let _ = write_if_changed(src_dir.join("lib.rs"), &mod_decls);
    }

    // 5. Generate `src/main.rs` if `crs/main.crs` exists
    if has_main_crs {
        let main_path = crs_dir.join("main.crs");
        let main_src = std::fs::read_to_string(&main_path).map_err(|e| e.to_string())?;
        let key = main_path.to_string_lossy().replace('\\', "/");
        let src_hash = hash_source(&main_src, crate::cforge::VERSION);
        let dest = src_dir.join("main.rs");
        if cache.get_valid(&key, &src_hash).is_none() {
            let mut r = transpile_main_entry(&main_src);
            r = rewrite_manifest_and_out_dir(&r, &project_dir_str, &src_dir_str);
            let header = "#![allow(dead_code, unused_imports, unused_mut, unused_variables, unused_parens, non_snake_case, clippy::all)]\n";
            let _ = write_if_changed(&dest, &format!("{header}{r}"));
            cache.insert(
                key,
                TranspileEntry {
                    hash: src_hash,
                    output_path: clean_win_path(&dest),
                    dependencies: Vec::new(),
                },
            );
        }
    }

    cache.save(&dist_dir_str);

    // 6. Generate `<project_dir>/dist/rust/Cargo.toml` from `<project_dir>/properties.kson`
    let prop_path = project_dir.join("properties.kson");
    if prop_path.is_file() {
        let (_, json) = crate::cforge::kson::read_properties(prop_path.to_str().unwrap());
        let toml = match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| {
                handle
                    .block_on(crate::cforge::properties::Properties::from_kson_in_dir(
                        &json,
                        &project_dir,
                    ))
                    .to_toml()
            }),
            Err(_) => {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(crate::cforge::properties::Properties::from_kson_in_dir(
                    &json,
                    &project_dir,
                ))
                .to_toml()
            }
        };
        let _ = write_if_changed(rust_dir.join("Cargo.toml"), &toml);
    }

    Ok(rust_dir)
}

fn clean_win_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    s.strip_prefix("//?/").unwrap_or(&s).to_string()
}

fn rewrite_manifest_and_out_dir(rust: &str, project_dir: &str, src_dir: &str) -> String {
    let mut s = rust.to_string();
    s = s.replace("env!(\"CARGO_MANIFEST_DIR\")", &format!("{project_dir:?}"));
    s = s.replace("env! (\"CARGO_MANIFEST_DIR\")", &format!("{project_dir:?}"));
    s = s.replace("env!(\"OUT_DIR\")", &format!("{src_dir:?}"));
    s = s.replace("env! (\"OUT_DIR\")", &format!("{src_dir:?}"));
    s = s.replace(
        "include_str!(\"../../",
        &format!("include_str!(concat!({project_dir:?}, \"/../"),
    );
    let prefix = format!("include_str!(concat!({project_dir:?}, \"/../");
    let mut fixed = String::with_capacity(s.len() + 128);
    let mut rest = s.as_str();
    while let Some(pos) = rest.find(&prefix) {
        fixed.push_str(&rest[..pos]);
        fixed.push_str(&prefix);
        rest = &rest[pos + prefix.len()..];
        if let Some(end_q) = rest.find("\")") {
            fixed.push_str(&rest[..end_q]);
            fixed.push_str("\"))");
            rest = &rest[end_q + 2..];
        }
    }
    fixed.push_str(rest);
    fixed
}

fn transpile_main_entry(src: &str) -> String {
    convert_crs_host_to_rust(src)
}

fn convert_crs_host_to_rust(src: &str) -> String {
    let mut out = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    let mut in_class_fields = false;
    let mut class_brace_depth = 0i32;

    for line in lines {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];

        if in_class_fields {
            for ch in trimmed.chars() {
                if ch == '{' {
                    class_brace_depth += 1;
                } else if ch == '}' {
                    class_brace_depth -= 1;
                }
            }
            if class_brace_depth <= 0 {
                in_class_fields = false;
            }
            out.push(line.to_string());
            continue;
        }

        // Convert `pub class Foo` / `class Foo` -> `pub struct Foo` / `struct Foo`
        if trimmed.starts_with("pub class ") || trimmed.starts_with("class ") {
            let replaced = if trimmed.starts_with("pub class ") {
                format!("{indent}pub struct {}", &trimmed["pub class ".len()..])
            } else {
                format!("{indent}struct {}", &trimmed["class ".len()..])
            };
            if replaced.contains('{') && !replaced.contains('}') {
                in_class_fields = true;
                class_brace_depth = 1;
            }
            out.push(replaced);
            continue;
        }

        // Convert `[pub] [unsafe] func <RetType> <name>(...) {` -> `[pub] [unsafe] fn <name>(...) [-> <RetType>] {`
        if let Some(converted) = try_convert_func_line(indent, trimmed) {
            out.push(converted);
            continue;
        }

        out.push(line.to_string());
    }
    out.join("\n") + "\n"
}

fn try_convert_func_line(indent: &str, trimmed: &str) -> Option<String> {
    let mut rest = trimmed;
    let mut prefix = String::new();
    if let Some(r) = rest.strip_prefix("pub ") {
        prefix.push_str("pub ");
        rest = r.trim_start();
    }
    if let Some(r) = rest.strip_prefix("unsafe ") {
        prefix.push_str("unsafe ");
        rest = r.trim_start();
    }
    let rest = rest.strip_prefix("func ")?.trim_start();
    let bytes = rest.as_bytes();
    let mut i = 0;
    let mut angle = 0i32;
    let mut paren = 0i32;
    let mut split_at = None;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => angle += 1,
            b'>' => angle = (angle - 1).max(0),
            b'(' if angle == 0 => {
                if paren == 0 && i > 0 {
                    let before = rest[..i].trim_end();
                    if let Some(last_space) = before.rfind(|c: char| c.is_whitespace()) {
                        let candidate_name = &before[last_space + 1..];
                        if !candidate_name.is_empty() {
                            split_at = Some((last_space, i));
                            break;
                        }
                    } else {
                        split_at = Some((0, i));
                        break;
                    }
                }
                paren += 1;
            }
            b')' if angle == 0 => {
                paren = (paren - 1).max(0);
            }
            _ => {}
        }
        i += 1;
    }
    let (ret_end, paren_pos) = split_at?;
    let (ret_type, fn_name_and_generics) = if ret_end == 0 && !rest[..paren_pos].contains(' ') {
        ("void", rest[..paren_pos].trim())
    } else {
        (rest[..ret_end].trim(), rest[ret_end..paren_pos].trim())
    };
    let after_name = &rest[paren_pos..];
    let mut pdepth = 0i32;
    let mut close_paren = None;
    for (idx, ch) in after_name.char_indices() {
        match ch {
            '(' => pdepth += 1,
            ')' => {
                pdepth -= 1;
                if pdepth == 0 {
                    close_paren = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }
    let close_idx = close_paren?;
    let params_part = &after_name[..=close_idx];
    let tail_part = after_name[close_idx + 1..].trim_start();
    let mapped_ret = match ret_type {
        "void" | "()" | "" => String::new(),
        "int" => " -> i64".to_string(),
        "float" => " -> f64".to_string(),
        "str" => " -> String".to_string(),
        other => format!(" -> {other}"),
    };
    let sep = if tail_part.is_empty() { "" } else { " " };
    Some(format!(
        "{indent}{prefix}fn {fn_name_and_generics}{params_part}{mapped_ret}{sep}{tail_part}"
    ))
}

fn collect_crs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_crs(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("crs") {
            out.push(p);
        }
    }
}

fn embed_assets(root: &Path, bundle: &Path, out_dir: &Path) {
    let Ok(text) = std::fs::read_to_string(bundle) else {
        return;
    };
    let mut src = String::from("&[\n");
    for line in text.lines() {
        let parts: Vec<&str> = line.split('"').collect();
        let (Some(key), Some(path)) = (parts.get(1), parts.get(3)) else {
            continue;
        };
        let Some(rel) = path.strip_prefix("./") else {
            continue;
        };
        if *key == "icon" || !rel.starts_with("assets/") {
            continue;
        };
        let Ok(abs) = root.join(rel).canonicalize() else {
            continue;
        };
        let abs_s = clean_win_path(&abs);
        src += &format!("    ({key:?}, include_bytes!({abs_s:?})),\n");
    }
    src += "]\n";
    let _ = write_if_changed(out_dir.join("embedded_assets.rs"), &src);
}

fn embed_templates(root: &Path, out_dir: &Path) {
    let mut files = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.file_name().is_some_and(|n| n != ".DS_Store") {
                out.push(p);
            }
        }
    }
    walk(root, &mut files);
    files.sort();
    let mut src = String::from("pub static TEMPLATE_FILES: &[(&str, &str, &[u8])] = &[\n");
    for f in &files {
        let Ok(rel) = f.strip_prefix(root) else {
            continue;
        };
        let mut parts = rel.components();
        let Some(first) = parts.next() else {
            continue;
        };
        let id = first.as_os_str().to_string_lossy().into_owned();
        let path: Vec<String> = parts
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let Ok(abs) = f.canonicalize() else {
            continue;
        };
        let abs_s = clean_win_path(&abs);
        src += &format!(
            "    ({id:?}, {:?}, include_bytes!({abs_s:?})),\n",
            path.join("/")
        );
    }
    src += "];\n";
    let _ = write_if_changed(out_dir.join("templates_gen.rs"), &src);
}

#[allow(dead_code)]
const ENGINE_UNIT_TESTS: &str = r#"
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_coerce_between_numeric_types() {
        assert_eq!(Variant::Int(7).as_float(), 7.0);
        assert_eq!(Variant::Float(3.9).as_int(), 3);
        assert_eq!(Variant::Bool(true).as_int(), 1);
    }

    #[test]
    fn variant_parse_and_display_round_trip() {
        let v = Variant::Vec2(12.5, -4.0);
        let s = v.to_string();
        assert_eq!(Variant::parse(VariantType::Vec2, &s), Some(v));
    }

    #[test]
    fn tree_paths_names_and_walk() {
        let mut t = SceneTree::with_root("Main", "Node2D");
        let p = t.add_child(t.root(), "Player", "CharacterBody2D").unwrap();
        let p_dup = t.add_child(t.root(), "Player", "Sprite2D").unwrap();
        assert_eq!(t.get(p).unwrap().name, "Player");
        assert_eq!(t.get(p_dup).unwrap().name, "Player2");
        assert_eq!(t.find_by_path("Main/Player"), Some(p));
        assert_eq!(t.find_by_path("Main/Player2"), Some(p_dup));
    }

    #[test]
    fn tree_reparent_rejects_cycles_and_remove_restores() {
        let mut t = SceneTree::with_root("Root", "Node2D");
        let a = t.add_child(t.root(), "A", "Node2D").unwrap();
        let b = t.add_child(a, "B", "Sprite2D").unwrap();
        assert_eq!(t.reparent(a, b, None), Err(TreeError::Cycle));
        let branch = t.remove_subtree(a).unwrap();
        assert!(t.get(a).is_none());
        let restored = t.insert_subtree(t.root(), None, branch).unwrap();
        assert_eq!(restored, a);
        assert!(t.get(b).is_some());
    }

    #[test]
    fn tree_properties_are_typed_and_default() {
        let mut t = SceneTree::with_root("Root", "Node2D");
        let s = t.add_child(t.root(), "Hero", "Sprite2D").unwrap();
        assert_eq!(t.get_prop(s, "centered"), Some(Variant::Bool(true)));
        assert!(t.set_prop(s, "centered", Variant::Bool(false)));
        assert_eq!(t.get_prop(s, "centered"), Some(Variant::Bool(false)));
    }
}
"#;

fn pre_fix_copper(rust: &str) -> String {
    let mut s = rust.to_string();
    // `crate const X` → `const X` (Copper uses `crate` visibility)
    s = s.replace("crate const ", "const ");
    s = s.replace("crate static ", "static ");
    s = s.replace("crate fn ", "fn ");
    s = s.replace("crate type ", "type ");
    s = s.replace("crate use ", "use ");
    // Standalone `crate` on its own line (leftover from transpiler)
    s = s.replace("\ncrate\n", "\n");
    s = s.replace("\ncrate;", "\n");
    s = s.replace("\nreturn crate;", "\n");
    // Copper includes `std` as an import item from `import { ... } from std::X`
    // e.g. `use ::collections::{BTreeMap, HashMap, std}` → remove `, std` from the braces
    s = s.replace(", std}", "}");
    s = s.replace(", std,", ",");
    s = s.replace("{std, ", "{");
    s = s.replace("{std}", "{}");
    // Similarly for `rc`: `use ::rc::{Rc, std}` → `use ::rc::{Rc}`
    s
}

pub fn transpile(module: &str, src: &str) -> String {
    if matches!(module, "graph_view" | "scene_view" | "main") {
        return convert_crs_host_to_rust(src);
    }
    let mut tokenizer = Tokenizer::new(src.to_string());
    let tokens = tokenizer.tokenize();
    let mut rust = parse(tokens);

    // Pre-fix: clean up Copper artifacts before hoisting
    rust = pre_fix_copper(&rust);
    rust = hoist_from_main(&rust);
    rust = strip_empty_main(&rust);
    rust = fix_lifetimes(&rust);
    rust = fix_dyn_spacing(&rust);
    rust = inject_aliases(module, &rust);
    rust = promote_pub(&rust);
    rust = promote_struct_fields(&rust);
    rust = post_fix_copper_bugs(&rust);
    rust = promote_all_fns(&rust);
    rust = cleanup_blank_lines(&rust);
    rust
}

/// Drop only Copper's trailing empty `fn main() {}` or `pub fn main() {}` stub,
/// keeping a real `main` (e.g. demo binary) that the `.crs` defined.
fn strip_empty_main(rust: &str) -> String {
    let mut s = rust.trim_end().to_string();
    // Try both `fn main()` and `pub fn main()`
    let markers = ["\npub fn main()", "\nfn main()"];
    let mut found = false;
    for marker in &markers {
        if let Some(at) = s.rfind(marker) {
            let tail = s[at + 1..].trim();
            let prefix = marker.trim_start_matches('\n');
            if let Some(rest) = tail.strip_prefix(prefix) {
                let inner = rest
                    .trim()
                    .trim_start_matches('{')
                    .trim_end_matches('}')
                    .trim();
                let trivial = inner.is_empty()
                    || inner.chars().all(|c| c == ';' || c.is_whitespace())
                    || inner == "crate";
                if trivial {
                    s.truncate(at);
                    found = true;
                    break;
                }
            }
        }
    }
    if !found {
        let trimmed = s.trim_start();
        for prefix in &["fn main()", "pub fn main()"] {
            if trimmed.starts_with(prefix) && s.matches(prefix).count() == 1 {
                let rest = trimmed.trim_start_matches(prefix);
                let inner = rest
                    .trim()
                    .trim_start_matches('{')
                    .trim_end_matches('}')
                    .trim();
                if inner.is_empty()
                    || inner.chars().all(|c| c == ';' || c.is_whitespace())
                    || inner == "crate"
                {
                    s.clear();
                    break;
                }
            }
        }
    }
    s.push('\n');
    s
}

fn post_fix_copper_bugs(rust: &str) -> String {
    let mut s = rust.to_string();
    // Copper inserts `let` after `self.` for field assignments: `self.let field = val`
    s = fix_self_let_assigns(&s);
    s = s.replace("let let ", "let ");
    s = s.replace(":let ", ": ");
    s = s.replace(": let ", ": ");
    s = s.replace(";;", ";");
    s = s.replace(";,", ";");
    s = s.replace("\n ;,", ";");
    s = s.replace("& mut ", "&mut ");
    // promote_struct_fields / depth bugs sometimes prefix statements with pub
    s = s.replace("pub let ", "let ");
    s = s.replace("pub mut ", "mut ");
    s = s.replace("pub return ", "return ");
    s = s.replace("pub if ", "if ");
    s = s.replace("pub for ", "for ");
    s = s.replace("pub while ", "while ");
    s = s.replace("pub match ", "match ");
    s = s.replace("pub app.", "app.");
    s = s.replace("pub renderer.", "renderer.");
    s = s.replace("pub static ", "static ");
    // Copper inserts `let` inside Ok/Err/Some patterns
    s = s.replace("Ok(let mut ", "Ok(mut ");
    s = s.replace("Ok(let ", "Ok(");
    s = s.replace("Err(let mut ", "Err(mut ");
    s = s.replace("Err(let ", "Err(");
    s = s.replace("Some(let mut ", "Some(mut ");
    s = s.replace("Some(let ", "Some(");
    // Broken slice syntax
    s = s.replace("&[, String]", "&[String]");
    s = s.replace("&[String)", "&[String]");
    // Copper glues `&mut Type` → `&mutType` in some positions
    s = fix_mut_refs(&s);

    s = s.replace("&[, ", "&[");
    // Copper turns expression-tail `value` into `value;` which yields `()` —
    // rewrite `;\n…}` to drop the semicolon when it looks like a tail expr.
    s = fix_tail_expr_semicolons(&s);
    // Typed assigns Copper emits without let: `names:Vec<String>=`
    s = s.replace("names:Vec<String>=", "let names: Vec<String> = ");
    // let-else missing `let`: `Some(x)=... else` / `Ok(x)=... else`
    s = fix_match_arm_lets(&s);
    s = fix_missing_let_else(&s);
    // Struct field `main: Defaults {` mangled to `let main: Defaults; {`
    s = fix_typed_field_init(&s);
    // Lone `;` lines (from Copper comment stripping) break vec!/array literals
    s = strip_lone_semicolons(&s);
    s = fix_typed_lets(&s);
    s = strip_junk_main(&s);
    s = fix_duplicate_type_imports(&s);
    s = fix_missing_stmt_semicolons(&s);
    s = s.replace("pub ;", ";");
    s = s.replace("\nJsonValue;\n", "\n");
    s = s.replace("pub fn eq(", "fn eq(");
    s = s.replace("pub fn ne(", "fn ne(");
    s = strip_duplicate_type_aliases(&s);
    s = fix_bare_assigns(&s);
    s = fix_tail_expr_semicolons(&s);
    s = fix_closure_tail_semicolons(&s);
    s = fix_tail_returns(&s);
    // Collapse accidental double returns from stacked post-fixes.
    s = s.replace("return return ", "return ");
    s = s.replace("return .", ".");
    s = s.replace("return impl ", "impl ");
    s = s.replace("return pub impl ", "pub impl ");
    s = s.replace(";;", ";");
    s = strip_struct_semicolons(&s);
    s = strip_return_before_match_arms(&s);
    s = s.replace("let t = match node_type", "t = match node_type");
    s = s.replace(
        "None => None,\n            }}",
        "None => None,\n            };",
    );
    s = s.replace(
        "None => None,\n            }}",
        "None => None,\n            };",
    );
    s = s.replace(
        "None => None,\r\n            }}",
        "None => None,\r\n            };",
    );
    // Copper emits `use ::module` instead of `use crate::module`
    s = fix_bare_use_paths(&s);
    // Copper sometimes emits `return crate;` or standalone `crate;`
    s = s.replace("\nreturn crate;}", "\n}");
    s = s.replace("\ncrate;\n", "\n");
    s = s.replace("return crate;}", "}");
    // Copper prefixes hoisted items with `crate` visibility: `crate const X = ...`
    // Rust doesn't support bare `crate` visibility — use nothing (pub already handled)
    s = s.replace("\ncrate const ", "\nconst ");
    s = s.replace("\ncrate static ", "\nstatic ");
    s = s.replace("\ncrate fn ", "\nfn ");
    s = s.replace("\ncrate type ", "\ntype ");
    s = s.replace("\ncrate use ", "\nuse ");
    // Copper corrupts raw string literals with regex patterns
    s = fix_corrupted_raw_strings(&s);
    s
}

fn fix_match_arm_lets(rust: &str) -> String {
    // `"arm"=>{name =` → `"arm"=>{ let name =`
    let mut out = String::with_capacity(rust.len() + 32);
    let bytes = rust.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"=>{") {
            out.push_str("=>{");
            i += 3;
            // skip whitespace
            while i < bytes.len() && bytes[i].is_ascii_whitespace() && bytes[i] != b'\n' {
                out.push(bytes[i] as char);
                i += 1;
            }
            // if ident =
            let rest = &rust[i..];
            if let Some(eq) = rest.find('=') {
                let name = rest[..eq].trim();
                if !name.is_empty()
                    && !name.starts_with("let ")
                    && name.chars().next().unwrap().is_ascii_lowercase()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !rest[eq..].starts_with("==")
                    && !rest[eq..].starts_with("=>")
                {
                    out.push_str("let ");
                }
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn fix_missing_let_else(rust: &str) -> String {
    let mut out = String::with_capacity(rust.len() + 32);
    for line in rust.lines() {
        let t = line.trim_start();
        let needs = (t.starts_with("Some(")
            || t.starts_with("Ok(")
            || t.starts_with("Err(")
            || t.starts_with("("))
            && (t.contains(") =") || t.contains(")="))
            && t.contains(" else ")
            && !t.starts_with("let ");
        if needs {
            let indent_len = line.len() - t.len();
            out.push_str(&line[..indent_len]);
            out.push_str("let ");
            out.push_str(t);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !rust.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

fn fix_typed_field_init(rust: &str) -> String {
    let mut s = rust.to_string();
    s = s.replace("let main: Defaults; {", "main: Defaults {");
    s = s.replace(
        "let docked: Option<Defaults>; {",
        "docked: Option<Defaults> {",
    );
    s
}

fn strip_lone_semicolons(rust: &str) -> String {
    rust.lines()
        .filter(|l| l.trim() != ";")
        .collect::<Vec<_>>()
        .join("\n")
        + if rust.ends_with('\n') { "\n" } else { "" }
}

/// Drop a semicolon that sits alone as the last statement before `}` so the
/// preceding expression becomes the block value / is fixed by adding return.
/// Safer approach: turn `EXPR;` immediately before `}` into `return EXPR;`.
fn fix_typed_lets(rust: &str) -> String {
    // `name: Type = expr` → `let name: Type = expr` (Copper drops `let`)
    let mut out = String::with_capacity(rust.len() + 64);
    for line in rust.lines() {
        let t = line.trim_start();
        let indent = &line[..line.len() - t.len()];
        let fixed = if !t.starts_with("let ")
            && !t.starts_with("pub ")
            && !t.starts_with("//")
            && !t.starts_with("struct ")
            && !t.starts_with("enum ")
        {
            // name: Type =  or mut name: Type =
            let t2 = t.strip_prefix("mut ").unwrap_or(t);
            let is_mut = t2.len() != t.len();
            if let Some(colon) = t2.find(':') {
                let name = t2[..colon].trim();
                let rest = t2[colon + 1..].trim_start();
                let is_name = !name.is_empty()
                    && name.chars().next().unwrap().is_ascii_lowercase()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                let eq = rest.find('=');
                let has_type = eq
                    .map(|i| {
                        let ty = rest[..i].trim();
                        let looks_like_type = !ty.is_empty()
                            && !ty.starts_with(':')
                            && !ty.contains("&&")
                            && !ty.contains("||")
                            && !ty.contains("==")
                            && !ty.contains(' ')
                            && (ty.contains('<')
                                || ty.contains("::")
                                || ty.starts_with("Map")
                                || ty.starts_with("Vec")
                                || ty.starts_with("Option")
                                || ty.starts_with("Result")
                                || ty.starts_with("Value")
                                || ty.starts_with("String")
                                || ty.starts_with("BTreeMap")
                                || ty.starts_with("HashMap")
                                || ty.starts_with("NodeId")
                                || ty.starts_with("Aabb")
                                || (ty
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_uppercase())
                                    .unwrap_or(false)
                                    && ty.chars().all(|c| {
                                        c.is_ascii_alphanumeric()
                                            || c == '_'
                                            || c == '<'
                                            || c == '>'
                                            || c == ','
                                            || c == ':'
                                            || c == '('
                                            || c == ')'
                                    })));
                        // Allow `&Type` / `&[T]` typed lets explicitly
                        let ref_type = ty.starts_with('&') && ty.len() < 80;
                        looks_like_type || ref_type
                    })
                    .unwrap_or(false);
                if is_name && has_type && !rest.starts_with(':') {
                    let body = if is_mut {
                        format!("mut {t2}")
                    } else {
                        t2.to_string()
                    };
                    Some(format!("{indent}let {body}"))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        out.push_str(fixed.as_deref().unwrap_or(line));
        out.push('\n');
    }
    if !rust.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

fn fix_bare_assigns(rust: &str) -> String {
    // Only insert `let` the first time a simple name is assigned in a scope.
    // Reassignments to `mut` bindings must stay bare (`cur = ...`), not `let cur =`.
    let mut out = String::with_capacity(rust.len() + 64);
    let mut declared: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut depth: i32 = 0;
    let mut depth_stack: Vec<std::collections::HashSet<String>> = Vec::new();
    for line in rust.lines() {
        let t = line.trim_start();
        let indent = &line[..line.len() - t.len()];
        // Track brace depth roughly so nested blocks get their own declared set.
        let opens = line.chars().filter(|c| *c == '{').count() as i32;
        let closes = line.chars().filter(|c| *c == '}').count() as i32;
        // Record names introduced by explicit let/mut on this line.
        if let Some(rest) = t
            .strip_prefix("let mut ")
            .or_else(|| t.strip_prefix("let "))
        {
            if let Some(name) = rest.split('=').next() {
                let name = name
                    .trim()
                    .trim_end_matches(':')
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    declared.insert(name.to_string());
                }
            }
        }
        let add_let = {
            if t.starts_with("let ")
                || t.starts_with("mut ")
                || t.starts_with("pub ")
                || t.starts_with("if ")
                || t.starts_with("while ")
                || t.starts_with("for ")
                || t.starts_with("return ")
                || t.starts_with("type ")
                || t.starts_with("const ")
                || t.starts_with("static ")
                || t.starts_with("//")
                || t.starts_with('*')
            {
                false
            } else if let Some(eq) = t.find('=') {
                let lhs = t[..eq].trim();
                let after = &t[eq..];
                let is_cmp = after.starts_with("==")
                    || after.starts_with("!=")
                    || after.starts_with("=>")
                    || (eq > 0 && matches!(t.as_bytes()[eq - 1], b'!' | b'<' | b'>'));
                let tuple = lhs.starts_with('(') && lhs.ends_with(')') && !lhs.contains('=');
                let simple = !lhs.is_empty()
                    && !lhs.contains('.')
                    && !lhs.contains('[')
                    && (!lhs.contains('(') || tuple)
                    && (tuple || !lhs.contains(' '))
                    && (tuple
                        || (lhs.chars().next().unwrap().is_ascii_lowercase()
                            && lhs.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')));
                if is_cmp || !(simple || tuple) {
                    false
                } else if tuple {
                    // Destructure: always needs let the first time; treat as fresh.
                    true
                } else if declared.contains(lhs) {
                    false
                } else {
                    declared.insert(lhs.to_string());
                    true
                }
            } else {
                false
            }
        };
        if add_let {
            out.push_str(indent);
            out.push_str("let ");
            out.push_str(t);
        } else {
            out.push_str(line);
        }
        out.push('\n');
        // Push/pop declared sets around braces (best-effort).
        for _ in 0..opens {
            depth += 1;
            depth_stack.push(declared.clone());
        }
        for _ in 0..closes {
            if depth > 0 {
                depth -= 1;
                if let Some(prev) = depth_stack.pop() {
                    declared = prev;
                }
            } else {
                declared.clear();
            }
        }
    }
    if !rust.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

fn strip_duplicate_type_aliases(rust: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut out = String::new();
    for line in rust.lines() {
        let t = line.trim();
        if let Some(rest) = t
            .strip_prefix("pub type ")
            .or_else(|| t.strip_prefix("type "))
        {
            let name = rest.split('=').next().unwrap_or("").trim();
            if !name.is_empty() && !seen.insert(name.to_string()) {
                continue; // skip duplicate
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !rust.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

fn fix_missing_stmt_semicolons(rust: &str) -> String {
    let lines: Vec<&str> = rust.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_end();
        let next = lines.get(i + 1).map(|l| l.trim_start()).unwrap_or("");
        let needs = trimmed.ends_with('}')
            && !trimmed.ends_with("};")
            && !trimmed.ends_with("},")
            && !trimmed.ends_with("})")
            && !trimmed.ends_with("} else")
            && (next.starts_with("let ")
                || next.starts_with("for ")
                || next.starts_with("return ")
                || next.starts_with("mut ")
                || next.starts_with("if ")
                || next.starts_with("while ")
                || next.starts_with("match ")
                || next.starts_with("loop")
                || (!next.starts_with("|")
                    && !next.is_empty()
                    && next.chars().next().unwrap().is_ascii_lowercase()
                    && next.contains('=')
                    && !next.starts_with("else")
                    && !next.starts_with("pub ")
                    && !next.starts_with("fn ")
                    && !next.starts_with("impl ")
                    && !next.starts_with("struct ")
                    && !next.starts_with("enum ")
                    && !next.starts_with("use ")
                    && !next.starts_with("type ")
                    && !next.starts_with("const ")
                    && !next.starts_with("#")));
        // Also: `}\n else {` when it's if/else wrongly split - don't touch
        if needs && !next.starts_with("else") {
            out.push(format!("{trimmed};"));
        } else {
            out.push(line.to_string());
        }
        i += 1;
    }
    let mut s = out.join("\n");
    if rust.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn fix_duplicate_type_imports(rust: &str) -> String {
    // `type NodeId = ...` plus `use ...::{NodeId, ...}` → drop NodeId from use
    let mut s = rust.to_string();
    if s.contains("type NodeId =") {
        s = s.replace(
            "use crate::tree::{Branch, NodeId, SceneTree, TreeError}",
            "use crate::tree::{Branch, SceneTree, TreeError}",
        );
        s = s.replace(
            "use crate::tree::{NodeId, SceneTree, Transform2D}",
            "use crate::tree::{SceneTree, Transform2D}",
        );
        s = s.replace(
            "use crate::graph::{BlockDef, BlockId, Graph, PinType}",
            "use crate::graph::{BlockDef, Graph, PinType}",
        );
    }
    if s.contains("type BlockId =") {
        s = s.replace(
            "use crate::graph::{BlockDef, BlockId, Graph, PinType}",
            "use crate::graph::{BlockDef, Graph, PinType}",
        );
    }
    s
}

fn strip_junk_main(rust: &str) -> String {
    // Drop `fn main() { Ident; }` or `pub fn main() { crate }` leftovers from failed hoists
    let markers = ["\npub fn main()", "\nfn main()"];
    for marker in &markers {
        let re_start = rust.rfind(marker);
        let Some(at) = re_start else { continue };
        let tail = &rust[at + 1..];
        let prefix = marker.trim_start_matches('\n');
        if let Some(rest) = tail.strip_prefix(prefix) {
            let inner = rest
                .trim()
                .trim_start_matches('{')
                .trim_end_matches('}')
                .trim();
            let junk = inner.is_empty()
                || inner.chars().all(|c| c == ';' || c.is_whitespace())
                || inner == "crate"
                || inner
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .all(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
            if junk {
                let mut s = rust[..at].to_string();
                s.push('\n');
                return s;
            }
        }
    }
    rust.to_string()
}

fn fix_closure_tail_semicolons(rust: &str) -> String {
    // Last stmt in closure/block before `}` with trailing `;` that should be a value.
    let lines: Vec<&str> = rust.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or("");
        if (next == "})" || next == "};" || next == "}" || next == "}," || next.starts_with("})"))
            && t.ends_with(';')
        {
            let body = t.trim_end_matches(';').trim();
            let valued = body.starts_with("!(")
                || body.starts_with("Some(")
                || body.starts_with("None")
                || body.starts_with("Ok(")
                || body.starts_with("Err(")
                || body.starts_with("true")
                || body.starts_with("false")
                || body.starts_with('(')
                || body.starts_with("vec![")
                || body.ends_with(".collect()")
                || body.ends_with(".clone()")
                || body.ends_with(".to_string()")
                || body.ends_with(".unwrap_or_default()")
                || body.contains("TouchButton")
                || body.contains("Block {")
                || body.contains("NamePair")
                || body.contains("Transform2D")
                || body.contains("PlatformInfo")
                || body.contains("InputMap")
                || body.contains("Target {")
                || (body.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && body
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_lowercase())
                        .unwrap_or(false));
            let banned = body.starts_with("return ")
                || body.starts_with("let ")
                || body.starts_with("if ")
                || body.starts_with("for ")
                || body.starts_with("while ")
                || body.starts_with("match ")
                || body.contains("insert(")
                || body.contains("push(")
                || body.contains("remove(")
                || body.contains("clear(")
                || body.contains("retain(")
                || body.contains(" = ")
                || body.contains('=') && !body.starts_with("Some(") && !body.starts_with("Ok(")
                || body == "stack.pop()";
            if valued && !banned && !body.starts_with('.') {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                // Nested block exprs (struct field inits) need a value, not `return`.
                // Prefer dropping the semicolon so the expr is the block's value.
                let simple_ident = body.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && body
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_lowercase())
                        .unwrap_or(false);
                if simple_ident
                    || body.ends_with(".collect()")
                    || body.ends_with(".clone()")
                    || body.ends_with(".to_string()")
                    || body.ends_with(".unwrap_or_default()")
                    || body.starts_with("vec![")
                {
                    out.push(format!("{indent}{body}"));
                } else {
                    out.push(format!("{indent}return {body};"));
                }
                continue;
            }
        }
        out.push((*line).to_string());
    }
    let mut s = out.join("\n");
    if rust.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn fix_tail_returns(rust: &str) -> String {
    let lines: Vec<&str> = rust.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        let t = line.trim();
        let next = lines[idx + 1..]
            .iter()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with("//"));
        let at_end = next == Some("}");
        let mut rewritten = None;
        if at_end
            && t.ends_with(';')
            && !t.ends_with("];")
            && !t.ends_with(");")
            && !t.ends_with("};")
        {
            let body = t[..t.len() - 1].trim();
            let banned = body.starts_with("return ")
                || body.starts_with("let ")
                || body.starts_with("if ")
                || body.starts_with("for ")
                || body.starts_with("while ")
                || body.starts_with("match ")
                || body.starts_with("break")
                || body.starts_with("continue")
                || body.starts_with('.')
                || body.starts_with("return .")
                || body.contains("retain(")
                || body.contains("insert(")
                || body.contains("push(")
                || body.contains("remove(")
                || body.contains("clear(")
                || body.contains(" = ");
            let valued = body.starts_with("self.")
                || body.starts_with('!')
                || body.starts_with("Some(")
                || body.starts_with("None")
                || body.starts_with("Ok(")
                || body.starts_with("Err(")
                || body.starts_with("true")
                || body.starts_with("false")
                || body.starts_with("format!(")
                || body.starts_with("json!(")
                || body.contains(".clone()")
                || body.contains(".to_string()")
                || body.contains(".as_str()")
                || body.contains(".is_empty()")
                || body.contains(".map(")
                || body.contains(".unwrap_or")
                || body
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_uppercase())
                    .unwrap_or(false);
            if !banned && valued {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                rewritten = Some(format!("{indent}return {body};"));
            }
        }
        out.push(rewritten.unwrap_or_else(|| (*line).to_string()));
    }
    let mut s = out.join("\n");
    if rust.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn fix_tail_expr_semicolons(rust: &str) -> String {
    // 1) `};` immediately before `}` → drop the semicolon (struct/block expr return).
    // 2) Other `expr;` before `}` → `return expr;` when it looks like a value.
    let mut out = String::with_capacity(rust.len() + 64);
    let bytes = rust.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'}' {
            let before = std::str::from_utf8(&bytes[..i]).unwrap_or("");
            let trimmed_end = before.trim_end();
            if trimmed_end.ends_with("};") {
                let line_start = trimmed_end.rfind('\n').map(|x| x + 1).unwrap_or(0);
                let line = trimmed_end[line_start..].trim();
                // Only strip for multi-line struct/enum literal closes (`};` alone, or
                // `TypeName { ... };`), not for `x = ...;` / `if ... { ... };`.
                let looks_struct = line == "};"
                    || (line.ends_with("};")
                        && !line.contains(" = ")
                        && !line.starts_with("if ")
                        && !line.starts_with("while ")
                        && !line.starts_with("for ")
                        && !line.starts_with("match ")
                        && (line
                            .chars()
                            .next()
                            .map(|c| c.is_ascii_uppercase())
                            .unwrap_or(false)
                            || line.starts_with("return ")));
                if looks_struct {
                    while out.ends_with([' ', '\t', '\n']) {
                        out.pop();
                    }
                    if out.ends_with(';') {
                        out.pop();
                    }
                }
            } else if trimmed_end.ends_with(';') {
                let stmt_start = trimmed_end.rfind('\n').map(|x| x + 1).unwrap_or(0);
                let stmt = trimmed_end[stmt_start..].trim();
                let stmt_body = stmt.trim_end_matches(';').trim();
                let skip = stmt_body.starts_with("return ")
                    || stmt_body.starts_with("let ")
                    || stmt_body.starts_with("pub ")
                    || stmt_body.starts_with("if ")
                    || stmt_body.starts_with("for ")
                    || stmt_body.starts_with("while ")
                    || stmt_body.starts_with("match ")
                    || stmt_body.starts_with("unsafe ")
                    || stmt_body.starts_with("loop")
                    || stmt_body.starts_with("impl ")
                    || stmt_body.starts_with("struct ")
                    || stmt_body.starts_with("enum ")
                    || stmt_body.starts_with("fn ")
                    || stmt_body.starts_with('.')
                    || stmt_body == "continue"
                    || stmt_body == "break"
                    || stmt_body == "stack.pop()"
                    || stmt_body.is_empty()
                    || stmt_body == "}"
                    || stmt_body.contains("=>") // match arms
                    || stmt_body.contains(".pop(")
                    || stmt_body.contains("insert(")
                    || stmt_body.contains("push(")
                    || stmt_body.contains("remove(")
                    || stmt_body.contains("clear(")
                    || stmt_body.contains("retain(")
                    || stmt_body.contains(" = ");
                let valued = stmt_body.starts_with("Some(")
                    || stmt_body.starts_with("None")
                    || stmt_body.starts_with("Ok(")
                    || stmt_body.starts_with("Err(")
                    || stmt_body.starts_with("true")
                    || stmt_body.starts_with("false")
                    || stmt_body.starts_with('(')
                    || stmt_body.starts_with("vec![")
                    || stmt_body.starts_with("format!(")
                    || stmt_body.starts_with("json!(")
                    || stmt_body.ends_with(".collect()")
                    || stmt_body.ends_with(".clone()")
                    || stmt_body.ends_with(".to_string()")
                    || stmt_body.ends_with(".unwrap_or_default()")
                    || stmt_body.contains("Transform2D")
                    || stmt_body.contains("PlatformInfo")
                    || stmt_body.contains("InputMap")
                    || stmt_body.contains("Target {")
                    || stmt_body.contains("NamePair")
                    || (stmt_body
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                        && stmt_body
                            .chars()
                            .next()
                            .map(|c| c.is_ascii_lowercase())
                            .unwrap_or(false));
                if !skip && valued {
                    while out.ends_with([' ', '\t', '\n']) {
                        out.pop();
                    }
                    if out.ends_with(';') {
                        out.pop();
                    }
                    let line_at = out.rfind('\n').map(|x| x + 1).unwrap_or(0);
                    let indent: String = out[line_at..]
                        .chars()
                        .take_while(|c| c.is_whitespace())
                        .collect();
                    let rest = out[line_at + indent.len()..].to_string();
                    out.truncate(line_at);
                    out.push_str(&indent);
                    if !rest.starts_with("return ") {
                        out.push_str("return ");
                    }
                    out.push_str(&rest);
                    out.push(';');
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn strip_return_before_match_arms(rust: &str) -> String {
    // Only strip `return` glued onto match *patterns*: `return Some(x)=>{`, `return None=>`.
    // Keep this ASCII/byte-safe — Copper headers can contain multi-byte junk.
    let mut s = rust.to_string();
    for (pat, repl) in [
        ("return None=>{", "None=>{"),
        ("return None =>{", "None=>{"),
        ("return None=>", "None=>"),
        ("return None =>", "None=>"),
    ] {
        s = s.replace(pat, repl);
    }
    // `return Some(ident)=>{` / `return Ok(ident)=>{` / `return Err(ident)=>{`
    let re = regex_lite_arm();
    for (pat, repl) in &re {
        s = s.replace(pat, repl);
    }
    // Generic: return Some(x)=>{ where x is a simple ident (byte scan)
    s = strip_return_variant_arms(&s, "return Some(", "Some(");
    s = strip_return_variant_arms(&s, "return Ok(", "Ok(");
    s = strip_return_variant_arms(&s, "return Err(", "Err(");
    s
}

fn regex_lite_arm() -> Vec<(String, String)> {
    // Placeholder list; real work is in strip_return_variant_arms.
    Vec::new()
}

fn strip_return_variant_arms(rust: &str, pat: &str, repl: &str) -> String {
    let b = rust.as_bytes();
    let pb = pat.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(pb) {
            let after = i + pb.len();
            // Parse a simple ident or `ident` then `)`
            let mut j = after;
            // allow nested simple: ident only for match bindings
            if j < b.len() && (b[j].is_ascii_alphabetic() || b[j] == b'_') {
                j += 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if j < b.len() && b[j] == b')' {
                    j += 1;
                    let mut k = j;
                    while k < b.len() && b[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if b[k..].starts_with(b"=>") {
                        out.extend_from_slice(repl.as_bytes());
                        out.extend_from_slice(&b[after..]);
                        // Actually we need to copy from after through end with recursive...
                        // Simpler: write repl, set i=after, continue
                        i = after;
                        continue;
                    }
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn strip_struct_semicolons(rust: &str) -> String {
    // Only `pub struct Foo { ... };\n` / `pub enum Foo { ... };\n` → drop trailing `;`
    // when the next non-empty line starts a new item. Avoid touching type aliases.
    let lines: Vec<&str> = rust.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_end();
        let mut rewritten = (*line).to_string();
        if t.ends_with("};") {
            let next = lines[i + 1..]
                .iter()
                .map(|l| l.trim())
                .find(|l| !l.is_empty() && !l.starts_with("//"));
            let next_ok = match next {
                Some(n) => {
                    n.starts_with("impl ")
                        || n.starts_with("pub ")
                        || n.starts_with("fn ")
                        || n.starts_with("const ")
                        || n.starts_with("type ")
                        || n.starts_with("use ")
                        || n.starts_with("#[")
                        || n.starts_with("}")
                }
                None => true,
            };
            // Require struct/enum on this line or recently above.
            let mut has_type = t.contains("struct ") || t.contains("enum ");
            if !has_type {
                for prev in lines[..i].iter().rev().take(3) {
                    let p = prev.trim();
                    if p.contains("struct ") || p.contains("enum ") || p.starts_with("#[") {
                        has_type = true;
                        break;
                    }
                    if !p.is_empty() {
                        break;
                    }
                }
            }
            if next_ok && has_type {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                let body = t.trim_end_matches(';');
                rewritten = format!("{indent}{body}");
            }
        }
        out.push(rewritten);
    }
    let mut s = out.join("\n");
    if rust.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn fix_mut_refs(rust: &str) -> String {
    let mut out = String::with_capacity(rust.len() + 8);
    let bytes = rust.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"&mut") {
            let after = i + 4;
            if after < bytes.len() && bytes[after].is_ascii_alphabetic() {
                out.push_str("&mut ");
                i = after;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn fix_dyn_spacing(rust: &str) -> String {
    let mut out = String::with_capacity(rust.len() + 8);
    let bytes = rust.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"dyn") {
            let after = i + 3;
            if after < bytes.len() && bytes[after].is_ascii_alphabetic() {
                out.push_str("dyn ");
                i = after;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn hoist_from_main(rust: &str) -> String {
    let Some(main_at) = rust.find("fn main()") else {
        return rust.to_string();
    };
    let brace_at = match rust[main_at..].find('{') {
        Some(i) => main_at + i,
        None => return rust.to_string(),
    };
    let body_start = brace_at + 1;
    let Some(body_end) = find_closing_brace(rust, brace_at) else {
        return rust.to_string();
    };
    let before = &rust[..main_at];
    let body = &rust[body_start..body_end];
    let after = &rust[body_end + 1..];

    let mut hoisted = String::new();
    let mut kept = String::new();
    for item in split_top_level_items(body) {
        let t = item.trim();
        if t.is_empty() || t == ";" {
            continue;
        }
        if is_hoistable(t) {
            let mut item = t.to_string();
            item = item.replace(":let ", ": ").replace(": let ", ": ");
            hoisted.push_str(&item);
            if !item.ends_with('}') && !item.ends_with(';') {
                hoisted.push(';');
            }
            hoisted.push('\n');
        } else {
            kept.push_str(t);
            kept.push('\n');
        }
    }

    let mut out = String::new();
    out.push_str(before);
    out.push_str(&hoisted);
    if !kept.trim().is_empty() {
        out.push_str("fn main() {\n");
        out.push_str(&kept);
        out.push_str("}\n");
    } else {
        out.push_str("fn main() {}\n");
    }
    out.push_str(after);
    out
}

fn is_hoistable(t: &str) -> bool {
    let t = t.trim_start();
    t.starts_with("static ")
        || t.starts_with("const ")
        || t.starts_with("type ")
        || t.starts_with("use ")
        || t.starts_with("pub static ")
        || t.starts_with("pub const ")
        || t.starts_with("pub type ")
        || t.starts_with("pub use ")
}

fn split_top_level_items(body: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            cur.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                cur.push(c);
            }
            '{' => {
                depth += 1;
                cur.push(c);
            }
            '}' => {
                depth -= 1;
                cur.push(c);
                if depth == 0 {
                    items.push(std::mem::take(&mut cur));
                }
            }
            ';' if depth == 0 => {
                cur.push(c);
                items.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        items.push(cur);
    }
    items
}

fn find_closing_brace(s: &str, open_at: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.get(open_at) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_str = false;
    let mut i = open_at;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn fix_lifetimes(rust: &str) -> String {
    let mut s = rust.replace("&'staticstr", "&'static str");
    s = s.replace("& 'static str", "&'static str");
    s = s.replace("&'static,", "&'static ");
    // Copper glues `&'aValue` / `<'aValue` — split lifetime from type name.
    s = fix_lifetime_type_glue(&s);
    s
}

fn fix_lifetime_type_glue(rust: &str) -> String {
    let mut out = String::with_capacity(rust.len() + 16);
    let bytes = rust.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            out.push('\'');
            i += 1;
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let lt = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
            // Prefer short lifetime `'a` / `'v` then Type; longer names stay as-is
            // unless followed by uppercase type start glued on: `'aValue`
            if lt.len() >= 2 {
                let chars: Vec<char> = lt.chars().collect();
                // `'aValue` or `'vstr`
                if chars[0].is_ascii_lowercase() {
                    let rest: String = chars[1..].iter().collect();
                    let split = chars
                        .get(1)
                        .map(|c| c.is_ascii_uppercase())
                        .unwrap_or(false)
                        || rest == "str"
                        || rest == "static"
                        || rest.starts_with("str");
                    if split {
                        out.push(chars[0]);
                        out.push(' ');
                        out.push_str(&rest);
                        continue;
                    }
                }
            }
            out.push_str(lt);
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn inject_aliases(module: &str, rust: &str) -> String {
    let mut prelude = String::from("// Generated from Copper — do not edit.\n");
    prelude.push_str("type StaticStr = &'static str;\n");
    match module {
        // mocidad-engine modules
        "types" => {
            prelude.push_str("type StaticNodeType = &'static NodeType;\n");
            prelude.push_str("type StaticPropDef = &'static PropDef;\n");
        }
        "tree" => {
            prelude.push_str("type NodeId = u32;\n");
            prelude.push_str("type StaticNodeType = &'static crate::types::NodeType;\n");
        }
        "document" => {
            prelude.push_str("type NodeId = crate::tree::NodeId;\n");
        }
        "graph" => {
            prelude.push_str("type BlockId = u32;\n");
            prelude.push_str("type StaticBlockDef = &'static BlockDef;\n");
            prelude.push_str("type BlockPin = (BlockId, String);\n");
        }
        "world" => {
            prelude.push_str(
                "pub type Loader = dyn FnMut(&str) -> Result<String, String>;\n\
                 type NodeId = crate::tree::NodeId;\n\
                 type BlockId = crate::graph::BlockId;\n\
                 type Size2 = (f64, f64);\n",
            );
        }
        "platform" => {
            prelude.push_str("type U32Slice = &'static [u32];\n");
            prelude.push_str("type ResSlice = &'static [(u32, u32)];\n");
            prelude.push_str("type ResList = Vec<(u32, u32)>;\n");
        }
        "scene_file" => {
            prelude.push_str("pub type Loader = dyn FnMut(&str) -> Result<String, String>;\n");
        }
        // engine/native-host modules
        "frame" => {
            prelude.push_str("type RectSrc = (f32, f32, f32, f32);\n");
        }
        "texture" => {
            prelude.push_str(
                "type ImageMap = std::collections::HashMap<String, mocida::image::Image>;\n\
                 type MissingMap = std::collections::HashMap<String, bool>;\n",
            );
        }
        "hotreload_host" => {
            prelude.push_str(
                "type PathBufVec = Vec<std::path::PathBuf>;\n\
                 type MtimeMap = std::collections::HashMap<std::path::PathBuf, u64>;\n",
            );
        }
        "runtime" => {
            prelude.push_str(
                "pub type SharedFrame = std::sync::Arc<std::sync::Mutex<crate::frame::FramePacket>>;\n\
                 type OptRenderDriver = Option<mocida::app::RenderDriver>;\n\
                 type PathBufVec = Vec<std::path::PathBuf>;\n",
            );
        }
        "audio_host" => {
            prelude.push_str("type SoundMap = std::collections::HashMap<String, mocida::Sound>;\n");
        }
        "save_host" => {
            prelude.push_str("type StringMap = std::collections::HashMap<String, String>;\n");
        }
        // mocidad-launcher modules
        "templates" => {
            prelude.push_str("include!(concat!(env!(\"OUT_DIR\"), \"/templates_gen.rs\"));\n");
            prelude.push_str("type Bytes = &'static [u8];\n");
            prelude.push_str("pub const SEP: char = '\\u{1}';\n");
        }
        "menubar" => {
            prelude.push_str("use std::cell::RefCell;\n");
            prelude.push_str("use std::os::raw::c_void;\n");
            prelude.push_str("use mocida::menu_bar::Menu;\n");
            prelude.push_str(
                "use mocida::menu_bar::role::{ABOUT as ROLE_ABOUT, FRONT as ROLE_FRONT, FULLSCREEN as ROLE_FULLSCREEN, HIDE as ROLE_HIDE, HIDE_OTHERS as ROLE_HIDE_OTHERS, MINIMIZE as ROLE_MINIMIZE, QUIT as ROLE_QUIT, SHOW_ALL as ROLE_SHOW_ALL, ZOOM as ROLE_ZOOM};\n",
            );
        }
        "device_frame" => {
            prelude.push_str("use std::sync::atomic::{AtomicU32, Ordering};\n");
            prelude.push_str("use mocidad_engine::platform::Target;\n");
            prelude.push_str("use mui_runtime::device_frames::{self, FrameDevice};\n");
            prelude.push_str("use mui_runtime::device_simulator::DeviceOrientation;\n");
            prelude.push_str("type UiAppPtr = *mut mocida::sys::UIApp;\n");
            prelude.push_str("type StaticFrame = &'static FrameDevice;\n");
            prelude.push_str("type PathBuf = std::path::PathBuf;\n");
            prelude.push_str("static ACTION: AtomicU32 = AtomicU32::new(0);\n");
            prelude.push_str(
                "unsafe extern \"C\" fn on_toolbar(action: std::os::raw::c_int, _: *mut std::ffi::c_void) {\n    ACTION.store(action as u32, Ordering::Release);\n}\n",
            );
        }
        "guide" => {
            prelude.push_str("type TopicRef = Topic;\n");
            prelude.push_str("pub const SEP: char = '\\u{1}';\n");
        }
        "debug_console" => {
            prelude.push_str("use std::io::{BufRead, Write};\n");
            prelude.push_str("use std::process::{ChildStderr, ChildStdin};\n");
            prelude.push_str("use std::sync::{Arc, Mutex};\n");
            prelude.push_str("pub type Shared = Arc<Mutex<Session>>;\n");
            prelude.push_str("pub const SEP: char = '\\u{1}';\n");
        }
        "source_control" => {
            prelude.push_str("use std::path::Path;\n");
            prelude.push_str("use crate::run_git;\n");
            prelude.push_str("pub const SEP: char = '\\u{1}';\n");
        }
        "upgrade" => {
            prelude.push_str("use std::path::Path;\n");
            prelude.push_str("use mocidad_engine::scene_file;\n");
            prelude.push_str("use crate::templates;\n");
        }
        "agent_ui" => {
            prelude.push_str("pub const SEP: char = '\\u{1}';\n");
            prelude.push_str("use mocida::FontStyle;\n");
        }
        "agent" => {
            prelude.push_str("use std::io::{BufRead, BufReader};\n");
            prelude.push_str("use std::sync::atomic::{AtomicBool, Ordering};\n");
            prelude.push_str("use std::sync::mpsc::Sender;\n");
            prelude.push_str("use std::sync::Arc;\n");
        }
        "project_settings" => {
            prelude.push_str("pub const SEP: char = '\\u{1}';\n");
            prelude.push_str("use std::collections::HashMap;\n");
            prelude.push_str("use std::path::{Path, PathBuf};\n");
            prelude.push_str("use mocidad_engine::platform::{self, mode_name, Device, Target};\n");
            prelude.push_str("use mocidad_engine::touch::{TouchControls, MAX_BUTTONS};\n");
            prelude.push_str("use serde_json::Value;\n");
        }
        "api_docs" => {
            prelude.push_str("use std::fmt::Write;\n");
            prelude.push_str("use mocidad_engine::platform::{self, GROUPS};\n");
            prelude.push_str(
                "use mocidad_engine::{block_defs, node_types, BlockDef, Category, PinType};\n",
            );
            prelude.push_str("use serde_json::Value;\n");
        }
        "docs_window" => {
            prelude.push_str("use std::cell::RefCell;\n");
            prelude.push_str("use std::collections::HashMap;\n");
            prelude.push_str("use std::io::{BufRead, Write};\n");
            prelude.push_str("use std::path::{Path, PathBuf};\n");
            prelude.push_str("use std::process::{Child, Stdio};\n");
            prelude.push_str("use std::rc::Rc;\n");
            prelude.push_str("use std::sync::mpsc;\n");
            prelude.push_str("use std::time::SystemTime;\n");
            prelude.push_str("use mocida::{App, Children, Color};\n");
            prelude.push_str("use mui_runtime::Reactive;\n");
        }
        "scene" => {
            prelude.push_str("pub const SEP: char = '\\u{1}';\n");
            prelude.push_str("pub const DEFAULT_SCENE: &str = \"scenes/main.scene\";\n");
            prelude.push_str("use std::collections::HashSet;\n");
            prelude.push_str("use std::path::{Path, PathBuf};\n");
            prelude.push_str("use mocidad_engine::{node_type, node_types, scene_file, NodeId, SceneDoc, SceneTree, Variant};\n");
            prelude.push_str("use serde_json::Value;\n");
        }
        "window" => {
            prelude.push_str("use std::cell::RefCell;\n");
            prelude.push_str("use std::collections::HashMap;\n");
            prelude.push_str("use std::io::BufRead;\n");
            prelude.push_str("use std::path::{Path, PathBuf};\n");
            prelude.push_str("use std::rc::Rc;\n");
            prelude.push_str("use std::time::SystemTime;\n");
            prelude.push_str("use mocida::{App, Children, Color};\n");
            prelude.push_str("use mui_runtime::Reactive;\n");
            prelude.push_str("use crate::scene::EditorScene;\n");
            prelude.push_str("use crate::{graph_view, scene_view};\n");
            prelude.push_str("type UiAppPtr = *mut mocida::sys::UIApp;\n");
        }
        "player" => {
            prelude.push_str("use std::cell::RefCell;\n");
            prelude.push_str("use std::collections::{HashMap, HashSet};\n");
            prelude.push_str("use std::io::{BufRead, Write};\n");
            prelude.push_str("use std::path::{Path, PathBuf};\n");
            prelude.push_str("use std::rc::Rc;\n");
            prelude.push_str("use std::time::Instant;\n");
            prelude.push_str("use mocida::sound::Sound;\n");
            prelude.push_str("use mocida::{App, Color};\n");
            prelude.push_str("use mocidad_engine::input::{Binding, PadAxis, PadButton};\n");
            prelude.push_str("use mocidad_engine::{scene_file, Event, World};\n");
            prelude.push_str("use crate::{input_view, scene, scene_view};\n");
        }
        "lsp" => {
            prelude.push_str("use serde_json::{json, Value};\n");
            prelude.push_str("use std::collections::HashMap;\n");
            prelude.push_str("use std::io::{BufRead, BufReader, Write};\n");
            prelude.push_str("use std::path::Path;\n");
            prelude
                .push_str("use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};\n");
            prelude.push_str("use std::sync::mpsc::{channel, Sender};\n");
            prelude.push_str("use std::sync::{Arc, Mutex};\n");
            prelude.push_str("use std::thread;\n");
            prelude.push_str("type DiagMap = Arc<Mutex<HashMap<String, Vec<Diag>>>>;\n");
            prelude.push_str("type CompSlot = Arc<Mutex<(i64, Vec<String>)>>;\n");
            prelude.push_str("type MsgReader = BufReader<ChildStdout>;\n");
        }
        "assets" => {
            prelude.push_str("use std::collections::hash_map::DefaultHasher;\n");
            prelude.push_str("use std::hash::{Hash, Hasher};\n");
            prelude.push_str("use std::io::Read;\n");
            prelude.push_str("use std::path::{Path, PathBuf};\n");
            prelude.push_str("use std::process::{Child, Command, Stdio};\n");
            prelude.push_str(
                "pub const TEXTURE_EXTS: &[&str] = &[\"png\", \"jpg\", \"jpeg\", \"gif\", \"bmp\", \"webp\", \"svg\", \"tga\"];\n",
            );
            prelude.push_str(
                "pub const AUDIO_EXTS: &[&str] = &[\"wav\", \"ogg\", \"mp3\", \"flac\", \"m4a\", \"aif\", \"aiff\"];\n",
            );
            prelude.push_str("const THUMB_PX: u32 = 96;\n");
            prelude.push_str("const MAX_ASSETS: usize = 4000;\n");
        }
        _ => {}
    }
    let mut body = String::new();
    body.push_str(&prelude);
    body.push_str(rust);
    if module == "graph" {
        body.push_str(
            r#"
impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}
fn catalog() -> &'static Vec<BlockDef> {
    static CAT: std::sync::OnceLock<Vec<BlockDef>> = std::sync::OnceLock::new();
    CAT.get_or_init(make_catalog)
}
pub fn block_def(id: &str) -> Option<&'static BlockDef> {
    catalog().iter().find(|b| b.id == id)
}
pub fn block_defs() -> Vec<&'static BlockDef> {
    catalog().iter().collect()
}
"#,
        );
    }
    if module == "scene_file" {
        body.push_str(
            r#"
impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}
"#,
        );
    }
    if module == "variant" {
        body.push_str(
            r#"
impl std::fmt::Display for Variant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}
"#,
        );
    }
    if module == "types" {
        body.push_str(
            r#"
fn registry() -> &'static Vec<NodeType> {
    static REG: std::sync::OnceLock<Vec<NodeType>> = std::sync::OnceLock::new();
    REG.get_or_init(make_registry)
}
pub fn node_type(name: &str) -> Option<&'static NodeType> {
    registry().iter().find(|t| t.name == name)
}
pub fn node_types() -> Vec<&'static NodeType> {
    registry().iter().collect()
}
"#,
        );
    }
    if module == "platform" {
        body = insert_after_struct(
            &body,
            "Model",
            "type ModelSlice = &'static [Model];\ntype StaticModel = &'static Model;\n",
        );
        body = insert_after_struct(
            &body,
            "Device",
            "type DeviceSlice = &'static [Device];\ntype StaticDevice = &'static Device;\n",
        );
        body.push_str(
            r#"
pub fn devices() -> &'static [Device] {
    static DEVICES: std::sync::OnceLock<Vec<Device>> = std::sync::OnceLock::new();
    DEVICES.get_or_init(make_devices).as_slice()
}
fn object(v: &mut Value) -> &mut Map<String, Value> {
    if !v.is_object() {
        *v = Value::Object(Map::new());
    }
    v.as_object_mut().expect("an object")
}
fn targets_mut(project: &mut Value) -> &mut Map<String, Value> {
    let p = object(project).entry("platforms").or_insert_with(|| Value::Object(Map::new()));
    object(object(p).entry("targets").or_insert_with(|| Value::Object(Map::new())))
}
"#,
        );
    }
    if module == "menubar" {
        body.push_str(
            r#"
thread_local! {
    static QUEUE: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}
unsafe extern "C" fn on_item(id: u32, _user: *mut c_void) {
    QUEUE.with(|q| q.borrow_mut().push(id));
}
"#,
        );
    }
    if module == "guide" {
        body.push_str(
            r#"
fn tour_table() -> &'static Vec<TourStep> {
    static T: std::sync::OnceLock<Vec<TourStep>> = std::sync::OnceLock::new();
    T.get_or_init(make_tour)
}
pub fn tour_len() -> usize {
    tour_table().len()
}
pub fn tour_at(i: usize) -> &'static TourStep {
    &tour_table()[i]
}
"#,
        );
    }
    if module == "agent_ui" {
        body.push_str(
            r#"
pub fn model_rows_slice(models: &[String], current: &str, provider: &str) -> Vec<String> {
    let v: Vec<String> = models.to_vec();
    model_rows(&v, current, provider)
}
pub fn chat_rows_pairs(history: &[(String, String)], provider: &str) -> Vec<String> {
    let roles: Vec<String> = history.iter().map(|(r, _)| r.clone()).collect();
    let texts: Vec<String> = history.iter().map(|(_, t)| t.clone()).collect();
    chat_rows(&turns_from(&roles, &texts), provider)
}
"#,
        );
    }
    if module == "api_docs" {
        body.push_str(
            r#"
pub fn search_str_pairs(query: &str, pages: &[(&str, String)]) -> String {
    let owned: Vec<(String, String)> = pages.iter().map(|(t, x)| ((*t).to_string(), x.clone())).collect();
    search(query, &owned)
}
"#,
        );
    }
    if module == "lsp" {
        body = insert_after_struct(
            &body,
            "Client",
            r#"
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
"#,
        );
    }
    if module == "assets" {
        body = insert_after_struct(
            &body,
            "Player",
            r#"
impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}
"#,
        );
    }
    body
}

fn insert_after_struct(rust: &str, name: &str, insert: &str) -> String {
    let marker = format!("struct {name}");
    let Some(at) = rust.find(&marker) else {
        return rust.to_string();
    };
    let Some(brace) = rust[at..].find('{') else {
        return rust.to_string();
    };
    let open = at + brace;
    let Some(close) = find_closing_brace(rust, open) else {
        return rust.to_string();
    };
    let mut out = String::with_capacity(rust.len() + insert.len());
    out.push_str(&rust[..=close]);
    out.push('\n');
    out.push_str(insert);
    out.push_str(&rust[close + 1..]);
    out
}

fn promote_pub(rust: &str) -> String {
    let mut out = String::with_capacity(rust.len() + 64);
    let mut depth = 0i32;
    let mut in_str = false;
    let mut line_start = true;
    let mut chars = rust.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_str = false;
            }
            if c == '\n' {
                line_start = true;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
                line_start = false;
            }
            '{' => {
                depth += 1;
                out.push(c);
                line_start = false;
            }
            '}' => {
                depth -= 1;
                out.push(c);
                line_start = false;
            }
            '\n' => {
                out.push(c);
                line_start = true;
            }
            _ if line_start && depth == 0 && !c.is_whitespace() => {
                let rest_start = {
                    let mut look = String::new();
                    look.push(c);
                    for p in chars.clone().take(24) {
                        look.push(p);
                    }
                    look
                };
                let trimmed = rest_start.trim_start();
                let needs = trimmed.starts_with("fn ")
                    || trimmed.starts_with("struct ")
                    || trimmed.starts_with("enum ")
                    || trimmed.starts_with("type ")
                    || trimmed.starts_with("trait ")
                    || trimmed.starts_with("static ")
                    || trimmed.starts_with("const ")
                    || trimmed.starts_with("use ");
                let already = trimmed.starts_with("pub ")
                    || trimmed.starts_with("//")
                    || trimmed.starts_with("#[")
                    || trimmed.starts_with("impl ")
                    || trimmed.starts_with("mod ");
                if needs && !already {
                    out.push_str("pub ");
                }
                out.push(c);
                line_start = false;
            }
            _ => {
                if !c.is_whitespace() {
                    line_start = false;
                }
                out.push(c);
            }
        }
    }
    out
}

/// Copper drops `pub` on struct fields; restore it for the public host API.
fn promote_struct_fields(rust: &str) -> String {
    let mut out = String::with_capacity(rust.len() + 64);
    let mut rest = rust;
    while let Some(at) = find_struct_kw(rest) {
        out.push_str(&rest[..at]);
        // Emit from `struct` through opening `{`
        let after = &rest[at..];
        let Some(brace_rel) = after.find('{') else {
            out.push_str(after);
            rest = "";
            break;
        };
        out.push_str(&after[..=brace_rel]);
        let body_start = at + brace_rel + 1;
        let Some(close) = find_closing_brace(rest, at + brace_rel) else {
            out.push_str(&rest[body_start..]);
            rest = "";
            break;
        };
        let body = &rest[body_start..close];
        out.push_str(&promote_fields_in_body(body));
        out.push('}');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

fn find_struct_kw(s: &str) -> Option<usize> {
    let mut i = 0;
    while let Some(rel) = s[i..].find("struct ") {
        let at = i + rel;
        let ok_before = at == 0 || {
            let b = s.as_bytes()[at - 1];
            !(b.is_ascii_alphanumeric() || b == b'_')
        };
        if ok_before {
            return Some(at);
        }
        i = at + 7;
    }
    None
}

fn promote_fields_in_body(body: &str) -> String {
    // Split top-level fields by comma, respecting nested generics/angles and braces.
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut depth_angle = 0i32;
    let mut depth_brace = 0i32;
    let mut depth_paren = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for c in body.chars() {
        if in_str {
            cur.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                cur.push(c);
            }
            '<' => {
                depth_angle += 1;
                cur.push(c);
            }
            '>' => {
                depth_angle -= 1;
                cur.push(c);
            }
            '{' => {
                depth_brace += 1;
                cur.push(c);
            }
            '}' => {
                depth_brace -= 1;
                cur.push(c);
            }
            '(' => {
                depth_paren += 1;
                cur.push(c);
            }
            ')' => {
                depth_paren -= 1;
                cur.push(c);
            }
            ',' if depth_angle == 0 && depth_brace == 0 && depth_paren == 0 => {
                parts.push(std::mem::take(&mut cur));
                parts.push(",".to_string());
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    let mut out = String::new();
    for part in parts {
        if part == "," {
            out.push(',');
            continue;
        }
        let trimmed = part.trim_start();
        let indent_len = part.len() - trimmed.len();
        if is_struct_field_line(trimmed.trim_end().trim_end_matches(','))
            || looks_like_field_fragment(trimmed)
        {
            out.push_str(&part[..indent_len]);
            if !trimmed.starts_with("pub ") {
                out.push_str("pub ");
            }
            out.push_str(trimmed);
        } else {
            out.push_str(&part);
        }
    }
    out
}

fn looks_like_field_fragment(trimmed: &str) -> bool {
    let t = trimmed.trim().trim_end_matches(',');
    if t.is_empty() || t.starts_with("pub ") || t.starts_with("//") || t.starts_with("#[") {
        return false;
    }
    let Some((name, rest)) = t.split_once(':') else {
        return false;
    };
    let name = name.trim();
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !rest.trim().is_empty()
}

fn is_struct_field_line(trimmed: &str) -> bool {
    looks_like_field_fragment(trimmed)
}

fn promote_all_fns(rust: &str) -> String {
    let mut s = rust.to_string();
    // Compact Copper output: promote items glued mid-line.
    s = s.replace(" fn ", " pub fn ");
    s = s.replace("{fn ", "{ pub fn ");
    s = s.replace("\nfn ", "\npub fn ");
    s = s.replace(" struct ", " pub struct ");
    s = s.replace(";struct ", "; pub struct ");
    s = s.replace("\nstruct ", "\npub struct ");
    s = s.replace(" enum ", " pub enum ");
    s = s.replace(";enum ", "; pub enum ");
    s = s.replace("\nenum ", "\npub enum ");
    s = s.replace(" type ", " pub type ");
    s = s.replace(";type ", "; pub type ");
    s = s.replace("\ntype ", "\npub type ");
    while s.contains("pub pub ") {
        s = s.replace("pub pub ", "pub ");
    }
    // Trait impl methods and extern C callbacks must not have `pub` after `extern "C"`.
    s = s.replace("unsafe extern \"C\" pub fn ", "unsafe extern \"C\" fn ");
    s = strip_pub_in_trait_impls(&s);
    s
}

fn strip_pub_in_trait_impls(rust: &str) -> String {
    let markers = [
        "impl Default for",
        "impl std::fmt::Display for",
        "impl std::fmt::Debug for",
        "impl From<",
        "impl Into<",
    ];
    let mut out = String::with_capacity(rust.len());
    let mut rest = rust;
    while !rest.is_empty() {
        let mut hit_at: Option<(usize, &str)> = None;
        for m in markers {
            if let Some(at) = rest.find(m) {
                match hit_at {
                    Some((prev, _)) if prev <= at => {}
                    _ => hit_at = Some((at, m)),
                }
            }
        }
        let Some((at, m)) = hit_at else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..at]);
        out.push_str(m);
        rest = &rest[at + m.len()..];
        let Some(brace_rel) = rest.find('{') else {
            continue;
        };
        out.push_str(&rest[..brace_rel + 1]);
        let open_in_full = out.len() - 1;
        // Re-find brace index in original combined string for find_closing_brace:
        let combined = format!("{out}{}", &rest[brace_rel + 1..]);
        // Simpler: scan braces in rest from brace_rel
        let open = brace_rel;
        let Some(close_rel) = find_closing_brace(rest, open) else {
            out.push_str(&rest[brace_rel + 1..]);
            break;
        };
        let body = &rest[open + 1..close_rel];
        let fixed = body
            .replace(" pub fn ", " fn ")
            .replace("\npub fn ", "\nfn ");
        out.push_str(&fixed);
        out.push('}');
        rest = &rest[close_rel + 1..];
        let _ = open_in_full;
        let _ = combined;
    }
    out
}

fn cleanup_blank_lines(rust: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in rust.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank <= 2 {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Fix `self.let field = value` → `self.field = value`.
/// Copper's bare-assign insertion sometimes inserts `let` after `self.`.
fn fix_self_let_assigns(rust: &str) -> String {
    // Copper inserts `let` after any `identifier.` for field assignments:
    //   self.let field = val  →  self.field = val
    //   a.let name = new     →  a.name = new
    //   t.let show = v       →  t.show = v
    // We replace all occurrences of `.let ` preceded by an identifier char.
    let needle = b".let ";
    let mut result = Vec::with_capacity(rust.len());
    let bytes = rust.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + needle.len() <= bytes.len() && &bytes[i..i + needle.len()] == needle {
            // Check if preceded by an identifier char
            if i > 0 {
                let prev = bytes[i - 1];
                if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b')' || prev == b']' {
                    // Keep the dot, skip "let "
                    result.push(b'.');
                    i += needle.len(); // skip ".let "
                    continue;
                }
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(result).unwrap_or_else(|_| rust.to_string())
}

/// Fix `use ::module::` → `use crate::module::` or `use std::module::`.
/// Copper emits bare `use ::tree::` instead of `use crate::tree::`.
/// Standard library modules (collections, rc, cell, etc.) go to `std::`.
/// Handles both line-start and mid-line occurrences (e.g. after `;`).
fn fix_bare_use_paths(rust: &str) -> String {
    let mut s = rust.to_string();
    // Known std modules that Copper emits as `use ::X`
    let std_mods = [
        "collections",
        "rc",
        "cell",
        "sync",
        "io",
        "fs",
        "path",
        "env",
        "fmt",
        "mem",
        "ops",
        "cmp",
        "hash",
        "iter",
        "net",
        "thread",
        "time",
        "process",
        "any",
        "marker",
        "convert",
        "borrow",
        "str",
        "string",
        "vec",
        "boxed",
        "slice",
        "num",
        "ffi",
        "os",
    ];
    // First, fix std modules: `use ::collections::` → `use std::collections::`
    for m in &std_mods {
        let bare = format!("use ::{m}::");
        let fixed = format!("use std::{m}::");
        s = s.replace(&bare, &fixed);
        let bare_semi = format!(";use ::{m}::");
        let fixed_semi = format!(";use std::{m}::");
        s = s.replace(&bare_semi, &fixed_semi);
        let bare_brace = format!("}}use ::{m}::");
        let fixed_brace = format!("}}use std::{m}::");
        s = s.replace(&bare_brace, &fixed_brace);
        // pub variants
        let bare_pub = format!("pub use ::{m}::");
        let fixed_pub = format!("pub use std::{m}::");
        s = s.replace(&bare_pub, &fixed_pub);
    }
    // Then convert remaining `use ::X` → `use crate::X` (crate modules)
    // Mid-line: `;use ::` → `;use crate::`
    s = s.replace(";use ::", ";use crate::");
    // After `}use ::`
    s = s.replace("}use ::", "}use crate::");
    // Line-start patterns
    let mut out = String::with_capacity(s.len());
    for line in s.lines() {
        let t = line.trim();
        if t.starts_with("use ::") {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            let rest = &t["use ::".len()..];
            out.push_str(&indent);
            out.push_str("use crate::");
            out.push_str(rest);
        } else if t.starts_with("pub use ::") {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            let rest = &t["pub use ::".len()..];
            out.push_str(&indent);
            out.push_str("pub use crate::");
            out.push_str(rest);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !rust.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Fix corrupted raw string literals where Copper splits `w * 2.0` across
/// a raw string boundary, producing `w__regex__::Regex::new(r" 2.0)`.
fn fix_corrupted_raw_strings(rust: &str) -> String {
    // This is a known Copper bug: when a `.crs` file contains multiplication
    // like `w * 2.0` near a raw-string pattern, the transpiler garbles it.
    // We fix the specific known cases here.
    let mut s = rust.to_string();
    // Fix the camera_limits corruption in world.rs
    if s.contains("w__regex__") {
        // The original Copper source has: axis(x, l, r, w * 2.0), axis(y, t, b, h * 2.0)
        // but Copper garbles it. Find and replace the specific pattern.
        s = s.replace(
            "w__regex__::Regex::new(r\" 2.0), axis(y, t, b, h \").unwrap()2.0)",
            "w * 2.0), axis(y, t, b, h * 2.0)",
        );
    }
    s
}
