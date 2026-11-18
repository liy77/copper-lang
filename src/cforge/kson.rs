use serde_json::{Map, Value};
use std::path::PathBuf;
use std::process::{exit, Command};
use std::{env, fs};

use crate::cforge::COPPER_PATH;

/// Locate the lson binary, building it from lson-src/ if necessary.
#[allow(dead_code)]
fn find_lson_binary() -> PathBuf {
    let binary_name = if cfg!(windows) { "lson.exe" } else { "lson" };
    let copper_path = PathBuf::from(&*COPPER_PATH);

    let installed = copper_path.join("lson").join(binary_name);
    if installed.exists() {
        return installed;
    }

    let exe_dir = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let in_debug = exe_dir
        .to_str()
        .map(|s| {
            s.contains(&format!("target{}debug", std::path::MAIN_SEPARATOR))
                || s.contains("target/debug")
        })
        .unwrap_or(false);

    let project_root: Option<PathBuf> = if in_debug {
        exe_dir.join("../..").canonicalize().ok()
    } else {
        None
    };

    if let Some(ref root) = project_root {
        let dev_bin = root
            .join("lson-src")
            .join("target")
            .join("release")
            .join(binary_name);
        if dev_bin.exists() {
            return dev_bin;
        }
    }

    let lson_src = project_root
        .as_ref()
        .map(|r| r.join("lson-src"))
        .filter(|p| p.join("Cargo.toml").exists())
        .or_else(|| {
            let p = copper_path.join("lson-src");
            if p.join("Cargo.toml").exists() {
                Some(p)
            } else {
                None
            }
        });

    if let Some(src) = lson_src {
        println!("lson not found — building from lson-src/ (first run)…");
        let manifest = src.join("Cargo.toml");
        let target_dir = src.join("target");
        let status = Command::new("cargo")
            .args(["build", "--release"])
            .arg("--manifest-path")
            .arg(&manifest)
            .arg("--target-dir")
            .arg(&target_dir)
            .status();
        if let Ok(s) = status {
            if s.success() {
                let built = src.join("target").join("release").join(binary_name);
                if built.exists() {
                    return built;
                }
            }
        }
    }

    installed
}

/// Fast native KSON parser supporting:
/// - Top-level `key = value` pairs
/// - `$section` headers (e.g. `$dependencies`, `$dev_dependencies`, `$features`)
/// - Strings, integers, floats, booleans, arrays `["a", "b"]`, and inline
///   objects `{ version = "1.0", features = ["derive"], path = "../foo" }`
/// - `@env(VAR)` environment variable bindings
pub fn parse_native_kson(text: &str) -> Option<Value> {
    let mut root = Map::new();
    let mut current_section: Option<String> = None;
    let mut section_map = Map::new();
    let mut env_vars = Map::new();

    for raw_line in text.lines() {
        let line_no_comment = strip_comment(raw_line);
        let trimmed = line_no_comment.trim();
        if trimmed.is_empty() || trimmed == "]" || trimmed == "}" {
            continue;
        }

        // @env(VAR_NAME)
        if let Some(rest) = trimmed.strip_prefix("@env(") {
            if let Some(var_name) = rest.strip_suffix(')') {
                let vname = var_name.trim();
                let val = env::var(vname).unwrap_or_default();
                env_vars.insert(vname.to_string(), Value::String(val));
            }
            continue;
        }

        // Top-level `$section` header (supports both `$dependencies` and `$dependencies [` / `$dependencies {`)
        let is_indented = raw_line.starts_with(' ') || raw_line.starts_with('\t');
        if let Some(sec_name) = trimmed.strip_prefix('$') {
            if let Some(prev_sec) = current_section.take() {
                root.insert(prev_sec, Value::Object(std::mem::take(&mut section_map)));
            }
            let clean_sec = sec_name
                .trim_end_matches('[')
                .trim_end_matches('{')
                .trim()
                .to_string();
            current_section = Some(clean_sec);
            continue;
        }

        if !is_indented && current_section.is_some() {
            if let Some(prev_sec) = current_section.take() {
                root.insert(prev_sec, Value::Object(std::mem::take(&mut section_map)));
            }
        }

        if let Some((k, v)) = split_key_value(trimmed) {
            let val = parse_kson_value(v.trim(), &env_vars);
            if current_section.is_some() {
                section_map.insert(k.trim().to_string(), val);
            } else {
                root.insert(k.trim().to_string(), val);
            }
        }
    }

    if let Some(prev_sec) = current_section.take() {
        root.insert(prev_sec, Value::Object(section_map));
    }

    Some(Value::Object(root))
}

fn strip_comment(line: &str) -> &str {
    let mut in_quotes = false;
    let mut escape = false;
    for (i, c) in line.char_indices() {
        if in_quotes {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_quotes = false;
            }
            continue;
        }
        if c == '"' {
            in_quotes = true;
        } else if c == '#' {
            return &line[..i];
        }
    }
    line
}

fn split_key_value(s: &str) -> Option<(&str, &str)> {
    let mut in_quotes = false;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    for (i, c) in s.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if !in_quotes {
            match c {
                '{' => brace_depth += 1,
                '}' => brace_depth = brace_depth.saturating_sub(1),
                '[' => bracket_depth += 1,
                ']' => bracket_depth = bracket_depth.saturating_sub(1),
                '=' if brace_depth == 0 && bracket_depth == 0 => {
                    return Some((&s[..i], &s[i + 1..]));
                }
                _ => {}
            }
        }
    }
    None
}

fn parse_kson_value(v: &str, env_vars: &Map<String, Value>) -> Value {
    let v = v.trim();
    if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
        let inner = &v[1..v.len() - 1];
        // Support shorthand `"0.31 [bundled]"` or `"path:../foo"`
        if let Some(path_target) = inner.strip_prefix("path:") {
            let mut m = Map::new();
            m.insert(
                "path".to_string(),
                Value::String(path_target.trim().to_string()),
            );
            return Value::Object(m);
        }
        if let Some((ver_part, feat_part)) = inner.split_once('[') {
            if let Some(feat_inner) = feat_part.strip_suffix(']') {
                let feats: Vec<Value> = feat_inner
                    .split(',')
                    .map(|f| f.trim().trim_matches('"'))
                    .filter(|f| !f.is_empty())
                    .map(|f| Value::String(f.to_string()))
                    .collect();
                let mut m = Map::new();
                m.insert(
                    "version".to_string(),
                    Value::String(ver_part.trim().to_string()),
                );
                m.insert("features".to_string(), Value::Array(feats));
                return Value::Object(m);
            }
        }
        return Value::String(inner.to_string());
    }
    if v == "true" {
        return Value::Bool(true);
    }
    if v == "false" {
        return Value::Bool(false);
    }
    if let Ok(n) = v.parse::<u64>() {
        return Value::Number(n.into());
    }
    if let Ok(n) = v.parse::<i64>() {
        return Value::Number(n.into());
    }
    if let Ok(f) = v.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(f) {
            return Value::Number(num);
        }
    }
    if v.starts_with('[') && v.ends_with(']') {
        let inner = &v[1..v.len() - 1];
        let items = split_top_level_comma(inner)
            .into_iter()
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(|item| parse_kson_value(item, env_vars))
            .collect();
        return Value::Array(items);
    }
    if v.starts_with('{') && v.ends_with('}') {
        let inner = &v[1..v.len() - 1];
        let mut obj = Map::new();
        for pair in split_top_level_comma(inner) {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let kv = split_key_value(pair).or_else(|| pair.split_once(':'));
            if let Some((k, val_str)) = kv {
                let key = k.trim().trim_matches('"').to_string();
                let val = parse_kson_value(val_str.trim(), env_vars);
                obj.insert(key, val);
            }
        }
        return Value::Object(obj);
    }
    if let Some(env_val) = env_vars.get(v) {
        return env_val.clone();
    }
    Value::String(v.to_string())
}

fn split_top_level_comma(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_quotes = false;
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if !in_quotes {
            match c {
                '[' | '{' | '(' => depth += 1,
                ']' | '}' | ')' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    parts.push(&s[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
    }
    parts.push(&s[start..]);
    parts
}

pub fn parse(text: &str) -> Value {
    if let Some(val) = parse_native_kson(text) {
        return val;
    }
    let lson_bin = find_lson_binary();
    let cmd = Command::new(&lson_bin)
        .arg("raw")
        .arg("compile")
        .args(["-t", "json"])
        .args(["--text", text])
        .output();

    match cmd {
        Ok(out) => {
            let mut stdout = String::from_utf8_lossy(&out.stdout);
            if stdout
                .lines()
                .next()
                .map(|l| l.starts_with("warning "))
                .unwrap_or(false)
            {
                stdout = stdout.lines().skip(1).collect();
            }
            serde_json::from_str(&stdout).expect("Invalid KSON file")
        }
        Err(e) => {
            println!("Failed to execute lson: {}", e);
            exit(1);
        }
    }
}

/// Read properties from a KSON or TOML file.
/// If both files exist, KSON takes precedence.
///
/// Returns (is_toml, Value)
pub fn read_properties(file: &str) -> (bool, Value) {
    let mut file = file.to_string();
    let mut c = fs::read(&file);
    if c.is_err() {
        file = file.replace("properties.kson", "Cargo.toml");
        c = fs::read(&file);
        if c.is_err() {
            println!("Error: Missing properties.kson. Run `cforge init` to create one.");
            exit(1);
        }
    }

    if file.ends_with("Cargo.toml") {
        let text = String::from_utf8(c.unwrap()).unwrap();
        let parsed: toml::Value = toml::from_str(&text).expect("Invalid Cargo.toml file");
        return (true, serde_json::to_value(parsed).unwrap());
    }

    let text = String::from_utf8(c.unwrap()).unwrap();
    (false, parse(&text))
}
