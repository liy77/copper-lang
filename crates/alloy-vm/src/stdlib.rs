//! Stdlib nativa do Alloy: implementações Rust das funções que os módulos
//! `std/*.crs` expõem. Quando um programa faz `import { f } from mod`, o
//! interpretador registra `f` e resolve as chamadas aqui.
//!
//! Cobertos aqui (Rust puro, sem deps externas): `cstd`, `fs`, `time`, `url`,
//! `net`. Os módulos `json`/`crypto`/`http` (que precisam de crates) ficam em
//! [`crate::stdlib_ext`].

use crate::error::RuntimeError;
use crate::value::Value;
use copper_syntax::ast::Span;
use std::io::{Read, Write};

/// Resolve `module::name(args)`. Retorna `None` se o módulo/função não é
/// conhecido por esta camada (o chamador então tenta outras vias / erro).
pub fn dispatch(
    module: &str,
    name: &str,
    args: &[Value],
    span: Span,
) -> Option<Result<Value, RuntimeError>> {
    let r = match module {
        "cstd" => cstd(name, args, span),
        "fs" => fs(name, args, span),
        "time" => time(name, args, span),
        "url" => url(name, args, span),
        "net" => net(name, args, span),
        _ => return None,
    };
    Some(r)
}

/// Conjunto de módulos tratados por esta camada (para o registro de imports).
pub fn handles(module: &str) -> bool {
    matches!(module, "cstd" | "fs" | "time" | "url" | "net")
}

// --- helpers de extração de argumentos ------------------------------------

fn arg_str(args: &[Value], i: usize, span: Span) -> Result<String, RuntimeError> {
    match args.get(i) {
        Some(v) => Ok(v.to_string()),
        None => Err(RuntimeError::new(format!("argumento {i} ausente"), span)),
    }
}

fn arg_int(args: &[Value], i: usize, span: Span) -> Result<i64, RuntimeError> {
    match args.get(i) {
        Some(Value::Int(n)) => Ok(*n),
        Some(other) => Err(RuntimeError::new(
            format!("argumento {i} deve ser int, achou {}", other.type_name()),
            span,
        )),
        None => Err(RuntimeError::new(format!("argumento {i} ausente"), span)),
    }
}

fn vec_of_strings(items: Vec<String>) -> Value {
    Value::Vec(std::rc::Rc::new(std::cell::RefCell::new(
        items.into_iter().map(Value::Str).collect(),
    )))
}

// ===========================================================================
// cstd
// ===========================================================================

fn cstd(name: &str, args: &[Value], span: Span) -> Result<Value, RuntimeError> {
    match name {
        "input" | "readln" => {
            // Imprime o prompt (se houver) e lê uma linha de stdin. Sem tty /
            // EOF → string vazia (não trava).
            if let Some(p) = args.first() {
                print!("{p} ");
                let _ = std::io::stdout().flush();
            }
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(_) => Ok(Value::Str(line.trim_end_matches(['\n', '\r']).to_string())),
                Err(_) => Ok(Value::Str(String::new())),
            }
        }
        "exit" => {
            let code = arg_int(args, 0, span).unwrap_or(0);
            std::process::exit(code as i32);
        }
        "sleep_ms" => {
            let ms = arg_int(args, 0, span)?;
            std::thread::sleep(std::time::Duration::from_millis(ms.max(0) as u64));
            Ok(Value::Unit)
        }
        "now_ms" => Ok(Value::Int(unix_millis())),
        "args" => Ok(vec_of_strings(std::env::args().skip(1).collect())),
        "env" => {
            let key = arg_str(args, 0, span)?;
            Ok(Value::Str(std::env::var(key).unwrap_or_default()))
        }
        "exists" => {
            let p = arg_str(args, 0, span)?;
            Ok(Value::Bool(std::path::Path::new(&p).exists()))
        }
        "read_file" => {
            let p = arg_str(args, 0, span)?;
            Ok(Value::Str(std::fs::read_to_string(p).unwrap_or_default()))
        }
        "write_file" => {
            let p = arg_str(args, 0, span)?;
            let c = arg_str(args, 1, span)?;
            Ok(Value::Bool(std::fs::write(p, c).is_ok()))
        }
        _ => Err(RuntimeError::new(
            format!("cstd::{name} não implementado"),
            span,
        )),
    }
}

// ===========================================================================
// fs
// ===========================================================================

fn fs(name: &str, args: &[Value], span: Span) -> Result<Value, RuntimeError> {
    let path = |i| arg_str(args, i, span);
    match name {
        "read" => Ok(Value::Str(
            std::fs::read_to_string(path(0)?).unwrap_or_default(),
        )),
        "write" => Ok(Value::Bool(std::fs::write(path(0)?, path(1)?).is_ok())),
        "append" => {
            let p = path(0)?;
            let data = path(1)?;
            let r = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .and_then(|mut f| f.write_all(data.as_bytes()));
            Ok(Value::Bool(r.is_ok()))
        }
        "exists" => Ok(Value::Bool(std::path::Path::new(&path(0)?).exists())),
        "is_file" => Ok(Value::Bool(std::path::Path::new(&path(0)?).is_file())),
        "is_dir" => Ok(Value::Bool(std::path::Path::new(&path(0)?).is_dir())),
        "mkdir" => Ok(Value::Bool(std::fs::create_dir_all(path(0)?).is_ok())),
        "remove_file" => Ok(Value::Bool(std::fs::remove_file(path(0)?).is_ok())),
        "remove_dir" => Ok(Value::Bool(std::fs::remove_dir_all(path(0)?).is_ok())),
        "size" => Ok(Value::Int(
            std::fs::metadata(path(0)?)
                .map(|m| m.len() as i64)
                .unwrap_or(-1),
        )),
        "list" => {
            let mut names = Vec::new();
            if let Ok(rd) = std::fs::read_dir(path(0)?) {
                for e in rd.flatten() {
                    names.push(e.file_name().to_string_lossy().into_owned());
                }
            }
            names.sort();
            Ok(vec_of_strings(names))
        }
        _ => Err(RuntimeError::new(
            format!("fs::{name} não implementado"),
            span,
        )),
    }
}

// ===========================================================================
// time
// ===========================================================================

fn unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn time(name: &str, args: &[Value], span: Span) -> Result<Value, RuntimeError> {
    match name {
        "now_ms" | "mono_ms" => Ok(Value::Int(unix_millis())),
        "now_secs" => Ok(Value::Int(unix_millis() / 1000)),
        "now_nanos" => Ok(Value::Int(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0),
        )),
        "sleep_ms" => {
            let ms = arg_int(args, 0, span)?;
            std::thread::sleep(std::time::Duration::from_millis(ms.max(0) as u64));
            Ok(Value::Unit)
        }
        "sleep_secs" => {
            let s = arg_int(args, 0, span)?;
            std::thread::sleep(std::time::Duration::from_secs(s.max(0) as u64));
            Ok(Value::Unit)
        }
        "iso8601" | "now_iso" => {
            // ISO8601 simplificado em UTC a partir dos segundos epoch.
            let secs = if name == "iso8601" && !args.is_empty() {
                arg_int(args, 0, span)?
            } else {
                unix_millis() / 1000
            };
            Ok(Value::Str(iso_from_epoch(secs)))
        }
        _ => Err(RuntimeError::new(
            format!("time::{name} não implementado"),
            span,
        )),
    }
}

/// Formata segundos-epoch UTC como `YYYY-MM-DDTHH:MM:SSZ` (algoritmo de Howard
/// Hinnant para a data civil).
fn iso_from_epoch(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

// ===========================================================================
// url
// ===========================================================================

fn url(name: &str, args: &[Value], span: Span) -> Result<Value, RuntimeError> {
    match name {
        "encode" | "encode_component" => Ok(Value::Str(percent_encode(&arg_str(args, 0, span)?))),
        "decode" => Ok(Value::Str(percent_decode(&arg_str(args, 0, span)?))),
        "join" => {
            let base = arg_str(args, 0, span)?;
            let path = arg_str(args, 1, span)?;
            let joined = if base.ends_with('/') {
                format!("{base}{}", path.trim_start_matches('/'))
            } else {
                format!("{base}/{}", path.trim_start_matches('/'))
            };
            Ok(Value::Str(joined))
        }
        "query2" => {
            // query2(k1, v1) → "k1=v1" (forma mínima encodada).
            let k = arg_str(args, 0, span)?;
            let v = arg_str(args, 1, span)?;
            Ok(Value::Str(format!(
                "{}={}",
                percent_encode(&k),
                percent_encode(&v)
            )))
        }
        _ => Err(RuntimeError::new(
            format!("url::{name} não implementado"),
            span,
        )),
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ===========================================================================
// net
// ===========================================================================

fn net(name: &str, args: &[Value], span: Span) -> Result<Value, RuntimeError> {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;
    match name {
        "resolve" => {
            let host = arg_str(args, 0, span)?;
            let addrs = format!("{host}:0")
                .to_socket_addrs()
                .map(|it| it.map(|a| a.ip().to_string()).collect::<Vec<_>>())
                .unwrap_or_default();
            Ok(vec_of_strings(addrs))
        }
        "local_ip" => {
            // Truque UDP: conectar a um host externo revela o IP local da rota.
            let ip = std::net::UdpSocket::bind("0.0.0.0:0")
                .and_then(|s| {
                    s.connect("8.8.8.8:80")?;
                    s.local_addr()
                })
                .map(|a| a.ip().to_string())
                .unwrap_or_else(|_| "127.0.0.1".into());
            Ok(Value::Str(ip))
        }
        "is_port_open" => {
            let host = arg_str(args, 0, span)?;
            let port = arg_int(args, 1, span)?;
            let ok = format!("{host}:{port}")
                .to_socket_addrs()
                .ok()
                .and_then(|mut it| it.next())
                .map(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(800)).is_ok())
                .unwrap_or(false);
            Ok(Value::Bool(ok))
        }
        "tcp_request" => {
            let addr = arg_str(args, 0, span)?;
            let payload = arg_str(args, 1, span)?;
            let resp = (|| -> std::io::Result<String> {
                let mut s = TcpStream::connect(&addr)?;
                s.set_read_timeout(Some(Duration::from_secs(3)))?;
                s.write_all(payload.as_bytes())?;
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                Ok(buf)
            })();
            match resp {
                Ok(s) => Ok(Value::Str(s)),
                Err(e) => Err(RuntimeError::new(format!("tcp_request falhou: {e}"), span)),
            }
        }
        _ => Err(RuntimeError::new(
            format!("net::{name} não implementado"),
            span,
        )),
    }
}
