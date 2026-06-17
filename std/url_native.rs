// `url` module — percent-encoding helpers, std-only (no dependencies).
// Bundled into `pub mod url { ... }` alongside std/url.crs.

/// Percent-encode a query *value*: keep the RFC 3986 unreserved set
/// (A–Z a–z 0–9 - _ . ~), encode everything else as %XX. Spaces become
/// %20 (not `+`) so the output is valid in both path and query positions.
pub fn encode(s: &str) -> String {
    encode_with(s, false)
}

/// Stricter form for a single component — same as `encode` (the unreserved
/// set already excludes `/ ? & = #`, so they are encoded). Kept as a
/// distinct name for intent at call sites.
pub fn encode_component(s: &str) -> String {
    encode_with(s, false)
}

fn encode_with(s: &str, space_as_plus: bool) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' if space_as_plus => out.push('+'),
            _ => {
                out.push('%');
                out.push(hex_digit(b >> 4));
                out.push(hex_digit(b & 0x0F));
            }
        }
    }
    out
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + (n - 10)) as char,
    }
}

/// Reverse percent-encoding: `%XX` -> byte, `+` -> space. Invalid `%`
/// sequences are passed through literally. Output is lossy-UTF-8.
pub fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h << 4) | l);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Build a two-pair query string `k1=v1&k2=v2`, encoding every key/value.
/// A blank key is skipped, so passing one pair (and two empty args) yields
/// a single-pair query.
pub fn query2(k1: &str, v1: &str, k2: &str, v2: &str) -> String {
    let mut parts = Vec::new();
    if !k1.is_empty() {
        parts.push(format!("{}={}", encode(k1), encode(v1)));
    }
    if !k2.is_empty() {
        parts.push(format!("{}={}", encode(k2), encode(v2)));
    }
    parts.join("&")
}

/// Join a base URL and a path with exactly one `/` between them.
pub fn join(base: &str, path: &str) -> String {
    let b = base.trim_end_matches('/');
    let p = path.trim_start_matches('/');
    format!("{b}/{p}")
}
