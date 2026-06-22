//! Native stdlib that depends on external crates: `crypto` (sha2/hmac + pure),
//! `json` (serde_json) and `http` (ureq). Kept separate from [`crate::stdlib`]
//! to isolate pure-Rust code from dependency-pulling code.

use crate::error::RuntimeError;
use crate::value::Value;
use copper_syntax::ast::Span;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn handles(module: &str) -> bool {
    matches!(module, "crypto" | "json" | "http")
}

pub fn dispatch(
    module: &str,
    name: &str,
    args: &[Value],
    span: Span,
) -> Option<Result<Value, RuntimeError>> {
    let r = match module {
        "crypto" => crypto(name, args, span),
        "json" => json(name, args, span),
        "http" => http(name, args, span),
        _ => return None,
    };
    Some(r)
}

fn arg_str(args: &[Value], i: usize, span: Span) -> Result<String, RuntimeError> {
    args.get(i)
        .map(|v| v.to_string())
        .ok_or_else(|| RuntimeError::new(format!("argument {i} missing"), span))
}

// ===========================================================================
// crypto
// ===========================================================================

fn crypto(name: &str, args: &[Value], span: Span) -> Result<Value, RuntimeError> {
    match name {
        "base64_encode" => Ok(Value::Str(base64_encode(
            arg_str(args, 0, span)?.as_bytes(),
        ))),
        "base64_decode" => Ok(Value::Str(
            String::from_utf8_lossy(&base64_decode(&arg_str(args, 0, span)?)).into_owned(),
        )),
        "hex_encode" => Ok(Value::Str(hex_encode(arg_str(args, 0, span)?.as_bytes()))),
        "crc32" => Ok(Value::Str(format!(
            "{:08x}",
            crc32(arg_str(args, 0, span)?.as_bytes())
        ))),
        "sha256" => {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(arg_str(args, 0, span)?.as_bytes());
            Ok(Value::Str(hex_encode(&h.finalize())))
        }
        "sha512" => {
            use sha2::{Digest, Sha512};
            let mut h = Sha512::new();
            h.update(arg_str(args, 0, span)?.as_bytes());
            Ok(Value::Str(hex_encode(&h.finalize())))
        }
        "hmac_sha256" => {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            let key = arg_str(args, 0, span)?;
            let msg = arg_str(args, 1, span)?;
            let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
                .map_err(|e| RuntimeError::new(format!("hmac: {e}"), span))?;
            mac.update(msg.as_bytes());
            Ok(Value::Str(hex_encode(&mac.finalize().into_bytes())))
        }
        _ => Err(RuntimeError::new(
            format!("crypto::{name} not implemented"),
            span,
        )),
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn base64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let clean: Vec<u8> = s
        .bytes()
        .filter(|&c| c != b'=' && !c.is_ascii_whitespace())
        .collect();
    let mut out = Vec::new();
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        let mut bits = 0;
        for &c in chunk {
            if let Some(v) = val(c) {
                n = (n << 6) | v;
                bits += 6;
            }
        }
        n <<= 24 - bits;
        let nbytes = (bits) / 8;
        for i in 0..nbytes {
            out.push((n >> (16 - i * 8)) as u8);
        }
    }
    out
}

fn hex_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ===========================================================================
// json
// ===========================================================================

/// Converts a `serde_json::Value` into an Alloy [`Value`] (object→Struct,
/// array→Vec, etc.). Objects become `Struct{name:"object"}` indexable by key.
pub fn json_to_value(j: &serde_json::Value) -> Value {
    match j {
        serde_json::Value::Null => Value::Unit,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Array(a) => {
            Value::Vec(Rc::new(RefCell::new(a.iter().map(json_to_value).collect())))
        }
        serde_json::Value::Object(o) => {
            let mut m = HashMap::new();
            for (k, v) in o {
                m.insert(k.clone(), json_to_value(v));
            }
            Value::Struct {
                name: "object".into(),
                fields: Rc::new(RefCell::new(m)),
            }
        }
    }
}

/// Navigates a dotted path (`user.name`, `tags.0`) in a JSON value.
fn json_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = root;
    for seg in path.split('.') {
        cur = if let Ok(idx) = seg.parse::<usize>() {
            cur.get(idx)?
        } else {
            cur.get(seg)?
        };
    }
    Some(cur)
}

fn json(name: &str, args: &[Value], span: Span) -> Result<Value, RuntimeError> {
    let doc = arg_str(args, 0, span)?;
    let root: serde_json::Value = serde_json::from_str(&doc)
        .map_err(|e| RuntimeError::new(format!("invalid json: {e}"), span))?;
    let path = |i| arg_str(args, i, span);
    match name {
        "get" => Ok(Value::Str(match json_path(&root, &path(1)?) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        })),
        "get_int" => Ok(Value::Int(
            json_path(&root, &path(1)?)
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        )),
        "get_float" => Ok(Value::Float(
            json_path(&root, &path(1)?)
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
        )),
        "get_bool" => Ok(Value::Bool(
            json_path(&root, &path(1)?)
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        )),
        "has" => Ok(Value::Bool(json_path(&root, &path(1)?).is_some())),
        "len" => Ok(Value::Int(
            json_path(&root, &path(1)?)
                .and_then(|v| v.as_array().map(|a| a.len() as i64))
                .unwrap_or(0),
        )),
        "pretty" => Ok(Value::Str(
            serde_json::to_string_pretty(&root).unwrap_or_default(),
        )),
        "minify" => Ok(Value::Str(serde_json::to_string(&root).unwrap_or_default())),
        _ => Err(RuntimeError::new(
            format!("json::{name} not implemented"),
            span,
        )),
    }
}

// ===========================================================================
// http
// ===========================================================================

fn response_value(status: i64, body: String) -> Value {
    let mut m = HashMap::new();
    m.insert("status".into(), Value::Int(status));
    m.insert("ok".into(), Value::Bool((200..300).contains(&status)));
    m.insert("body".into(), Value::Str(body));
    Value::Struct {
        name: "Response".into(),
        fields: Rc::new(RefCell::new(m)),
    }
}

fn do_request(req: ureq::Request, body: Option<&str>) -> (i64, String) {
    let resp = match body {
        Some(b) => req.set("Content-Type", "application/json").send_string(b),
        None => req.call(),
    };
    match resp {
        Ok(r) => {
            let status = r.status() as i64;
            (status, r.into_string().unwrap_or_default())
        }
        // ureq returns Err for status >= 400; extract the status when possible.
        Err(ureq::Error::Status(code, r)) => (code as i64, r.into_string().unwrap_or_default()),
        Err(e) => (0, format!("network error: {e}")),
    }
}

fn http(name: &str, args: &[Value], span: Span) -> Result<Value, RuntimeError> {
    match name {
        "get" => {
            let url = arg_str(args, 0, span)?;
            let (s, b) = do_request(ureq::get(&url), None);
            Ok(response_value(s, b))
        }
        "post_json" | "post" => {
            let url = arg_str(args, 0, span)?;
            let body = arg_str(args, 1, span)?;
            let (s, b) = do_request(ureq::post(&url), Some(&body));
            Ok(response_value(s, b))
        }
        "download" => {
            let url = arg_str(args, 0, span)?;
            let dest = arg_str(args, 1, span)?;
            let (s, b) = do_request(ureq::get(&url), None);
            let ok = (200..300).contains(&s) && std::fs::write(&dest, b).is_ok();
            Ok(Value::Bool(ok))
        }
        _ => Err(RuntimeError::new(
            format!("http::{name} not implemented"),
            span,
        )),
    }
}
