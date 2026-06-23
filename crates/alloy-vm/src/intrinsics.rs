//! Native `std::...` intrinsics for the interpreter.
//!
//! These let the tree-walking interpreter execute the handful of Rust-std leaf
//! operations that appear in **merged Copper source** — above all
//! `std/cstd.crs`, which is the single source of truth for the `cstd` module
//! (shared with the cforge transpiler). Without these, the interpreter could not
//! reach stdin, the filesystem, the clock, or the process, and `cstd` would have
//! to be reimplemented natively (the duplication this module removes).
//!
//! Opaque Rust handles (`Stdin`, `Path`, `SystemTime`, `Duration`) are modelled
//! as tagged [`Value::Struct`]s with a `__`-prefixed name so they never collide
//! with user types.

use crate::error::RuntimeError;
use crate::value::Value;
use copper_syntax::ast::Span;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

fn handle(name: &str, fields: Vec<(&str, Value)>) -> Value {
    let mut m = HashMap::new();
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    Value::Struct {
        name: name.to_string(),
        fields: Rc::new(RefCell::new(m)),
    }
}

fn vec_of(items: Vec<Value>) -> Value {
    Value::Vec(Rc::new(RefCell::new(items)))
}

fn str_of(v: Option<&Value>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

fn int_of(v: &Option<Value>) -> i64 {
    match v {
        Some(Value::Int(n)) => *n,
        _ => 0,
    }
}

fn unix_nanos() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Resolves a `std::...` / `String::new` / `Vec::new` **call**. Returns `None`
/// when the path isn't a known intrinsic (the caller then tries user-defined
/// associated functions / enum variants).
pub fn call_path(
    segments: &[String],
    args: &[Value],
    _span: Span,
) -> Option<Result<Value, RuntimeError>> {
    let segs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    let r: Value = match segs.as_slice() {
        ["String", "new"] => Value::Str(String::new()),
        ["String", "from"] => Value::Str(str_of(args.first())),
        ["Vec", "new"] => vec_of(Vec::new()),

        ["std", "io", "stdin"] => handle("__Stdin", vec![]),
        ["std", "io", "stdout"] | ["std", "io", "stderr"] => handle("__Stdout", vec![]),
        // `std::io::Write::flush(&mut std::io::stdout())` → flush real stdout, Ok(()).
        ["std", "io", "Write", "flush"] => {
            let _ = std::io::stdout().flush();
            Value::ok(Value::Unit)
        }

        ["std", "process", "exit"] => {
            let _ = std::io::stdout().flush();
            let code = match args.first() {
                Some(Value::Int(n)) => *n as i32,
                _ => 0,
            };
            std::process::exit(code);
        }

        ["std", "thread", "sleep"] => {
            if let Some(Value::Struct { name, fields }) = args.first() {
                if name == "__Duration" {
                    let ns = int_of(&fields.borrow().get("nanos").cloned());
                    std::thread::sleep(std::time::Duration::from_nanos(ns.max(0) as u64));
                }
            }
            Value::Unit
        }
        ["std", "time", "Duration", "from_millis"] => {
            let ms = int_of(&args.first().cloned());
            handle(
                "__Duration",
                vec![("nanos", Value::Int(ms.saturating_mul(1_000_000)))],
            )
        }
        ["std", "time", "SystemTime", "now"] => {
            handle("__Instant", vec![("nanos", Value::Int(unix_nanos()))])
        }

        ["std", "env", "var"] => match std::env::var(str_of(args.first())) {
            Ok(v) => Value::ok(Value::Str(v)),
            Err(_) => Value::err(Value::Str("NotPresent".into())),
        },
        ["std", "env", "args"] => vec_of(std::env::args().map(Value::Str).collect()),

        ["std", "path", "Path", "new"] => {
            handle("__Path", vec![("path", Value::Str(str_of(args.first())))])
        }

        ["std", "fs", "read_to_string"] => match std::fs::read_to_string(str_of(args.first())) {
            Ok(s) => Value::ok(Value::Str(s)),
            Err(e) => Value::err(Value::Str(e.to_string())),
        },
        ["std", "fs", "write"] => match std::fs::write(str_of(args.first()), str_of(args.get(1))) {
            Ok(_) => Value::ok(Value::Unit),
            Err(e) => Value::err(Value::Str(e.to_string())),
        },
        _ => return None,
    };
    Some(Ok(r))
}

/// Resolves a `std::...` path used as a **bare value** (not called), e.g.
/// `std::time::UNIX_EPOCH`.
pub fn path_value(segments: &[String]) -> Option<Value> {
    let segs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    match segs.as_slice() {
        ["std", "time", "UNIX_EPOCH"] => Some(handle("__Instant", vec![("nanos", Value::Int(0))])),
        _ => None,
    }
}

/// Resolves a method call on one of the opaque std handles. Returns `None` when
/// the receiver/method isn't an intrinsic (the caller falls through to the
/// regular builtin / user method resolution).
pub fn call_method(
    recv: &Value,
    name: &str,
    args: &[Value],
    _span: Span,
) -> Option<Result<Value, RuntimeError>> {
    let Value::Struct { name: ty, fields } = recv else {
        return None;
    };
    let get = |k: &str| fields.borrow().get(k).cloned();
    let r: Value = match (ty.as_str(), name) {
        ("__Path", "exists") => {
            Value::Bool(std::path::Path::new(&str_of(get("path").as_ref())).exists())
        }
        ("__Path", "is_file") => {
            Value::Bool(std::path::Path::new(&str_of(get("path").as_ref())).is_file())
        }
        ("__Path", "is_dir") => {
            Value::Bool(std::path::Path::new(&str_of(get("path").as_ref())).is_dir())
        }

        ("__Instant", "duration_since") => {
            let me = int_of(&get("nanos"));
            let other = match args.first() {
                Some(Value::Struct { fields, .. }) => {
                    int_of(&fields.borrow().get("nanos").cloned())
                }
                _ => 0,
            };
            Value::ok(handle(
                "__Duration",
                vec![("nanos", Value::Int((me - other).max(0)))],
            ))
        }
        ("__Instant", "elapsed") => {
            let me = int_of(&get("nanos"));
            Value::ok(handle(
                "__Duration",
                vec![("nanos", Value::Int((unix_nanos() - me).max(0)))],
            ))
        }
        ("__Duration", "as_nanos") => Value::Int(int_of(&get("nanos"))),
        ("__Duration", "as_micros") => Value::Int(int_of(&get("nanos")) / 1_000),
        ("__Duration", "as_millis") => Value::Int(int_of(&get("nanos")) / 1_000_000),
        ("__Duration", "as_secs") => Value::Int(int_of(&get("nanos")) / 1_000_000_000),
        _ => return None,
    };
    Some(Ok(r))
}
