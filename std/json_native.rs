// `json` module — read values from a JSON string via serde_json.
// Bundled into `pub mod json { ... }` alongside std/json.crs.

/// Resolve a dotted/indexed path against a parsed value. Empty path -> root.
fn lookup<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = root;
    if path.is_empty() {
        return Some(cur);
    }
    for seg in path.split('.') {
        match cur {
            serde_json::Value::Object(map) => {
                cur = map.get(seg)?;
            }
            serde_json::Value::Array(arr) => {
                let idx: usize = seg.parse().ok()?;
                cur = arr.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(cur)
}

fn parse(s: &str) -> Option<serde_json::Value> {
    serde_json::from_str(s).ok()
}

/// True when `s` is well-formed JSON.
pub fn is_valid(s: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(s).is_ok()
}

/// Scalar at `path` as a String. Strings come back unquoted; numbers/bools
/// are stringified; objects/arrays are re-serialised compactly; missing or
/// null -> "".
pub fn get(s: &str, path: &str) -> String {
    let Some(root) = parse(s) else {
        return String::new();
    };
    match lookup(&root, path) {
        Some(serde_json::Value::String(v)) => v.clone(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

/// Integer at `path`. Accepts JSON numbers and numeric strings. 0 otherwise.
pub fn get_int(s: &str, path: &str) -> i64 {
    let Some(root) = parse(s) else { return 0 };
    match lookup(&root, path) {
        Some(serde_json::Value::Number(n)) => {
            n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0)
        }
        Some(serde_json::Value::String(v)) => v.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Float at `path`. 0.0 if missing/non-numeric.
pub fn get_float(s: &str, path: &str) -> f64 {
    let Some(root) = parse(s) else { return 0.0 };
    match lookup(&root, path) {
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(serde_json::Value::String(v)) => v.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Bool at `path`. JSON `true`, the string "true", and non-zero numbers are
/// true; everything else (incl. missing) is false.
pub fn get_bool(s: &str, path: &str) -> bool {
    let Some(root) = parse(s) else { return false };
    match lookup(&root, path) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(v)) => v.eq_ignore_ascii_case("true"),
        Some(serde_json::Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        _ => false,
    }
}

/// True if `path` resolves to a present (non-null) value.
pub fn has(s: &str, path: &str) -> bool {
    parse(s)
        .as_ref()
        .and_then(|r| lookup(r, path))
        .map(|v| !v.is_null())
        .unwrap_or(false)
}

/// Length of the array/object at `path` (0 for scalars/missing).
pub fn len(s: &str, path: &str) -> i64 {
    let Some(root) = parse(s) else { return 0 };
    match lookup(&root, path) {
        Some(serde_json::Value::Array(a)) => a.len() as i64,
        Some(serde_json::Value::Object(o)) => o.len() as i64,
        _ => 0,
    }
}

/// Re-indent JSON. "" if `s` is invalid.
pub fn pretty(s: &str) -> String {
    match parse(s) {
        Some(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
        None => String::new(),
    }
}

/// Strip insignificant whitespace. "" if `s` is invalid.
pub fn minify(s: &str) -> String {
    match parse(s) {
        Some(v) => serde_json::to_string(&v).unwrap_or_default(),
        None => String::new(),
    }
}
