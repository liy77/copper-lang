// `http` module — blocking HTTP/HTTPS client over the `ureq` crate.
// Bundled by the compiler into `pub mod http { ... }` alongside std/http.crs.
//
// ureq is pure-Rust (rustls TLS), blocking, and tiny — a good fit for
// Copper's synchronous value-oriented style. Every helper returns an owned
// value (String / i32 / bool); errors collapse to a sensible empty value so
// scripts don't have to thread Result types through Copper surface syntax.

use std::io::Read;

/// Shared agent: a 30s timeout, redirects followed (ureq default). Built
/// once per process via OnceLock.
fn agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .build()
    })
}

/// GET `url`, return the response body. "" on any network/HTTP error.
pub fn get(url: &str) -> String {
    match agent().get(url).call() {
        Ok(resp) => resp.into_string().unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// GET `url`, return only the HTTP status code (e.g. 200, 404). 0 on a
/// transport error. ureq treats 4xx/5xx as `Err(Status)`, so unwrap both.
pub fn get_status(url: &str) -> i32 {
    match agent().get(url).call() {
        Ok(resp) => resp.status() as i32,
        Err(ureq::Error::Status(code, _)) => code as i32,
        Err(_) => 0,
    }
}

/// True when the status is 2xx.
pub fn ok(url: &str) -> bool {
    let s = get_status(url);
    (200..300).contains(&s)
}

/// POST `body` with an explicit `content_type`; return the response body.
pub fn post(url: &str, content_type: &str, body: &str) -> String {
    match agent()
        .post(url)
        .set("Content-Type", content_type)
        .send_string(body)
    {
        Ok(resp) => resp.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(_, resp)) => resp.into_string().unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// POST a JSON string body (`Content-Type: application/json`).
pub fn post_json(url: &str, json_body: &str) -> String {
    post(url, "application/json", json_body)
}

/// PUT `body` with an explicit `content_type`; return the response body.
pub fn put(url: &str, content_type: &str, body: &str) -> String {
    match agent()
        .put(url)
        .set("Content-Type", content_type)
        .send_string(body)
    {
        Ok(resp) => resp.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(_, resp)) => resp.into_string().unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// DELETE `url`; return the response body.
pub fn delete(url: &str) -> String {
    match agent().delete(url).call() {
        Ok(resp) => resp.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(_, resp)) => resp.into_string().unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// Read one response header (case-insensitive name). "" if absent/error.
pub fn get_header(url: &str, name: &str) -> String {
    match agent().get(url).call() {
        Ok(resp) => resp.header(name).unwrap_or("").to_string(),
        Err(_) => String::new(),
    }
}

/// Stream `url` to a local file at `path`. Returns true on success. Handles
/// bodies larger than memory by copying through an 8 KiB buffer.
pub fn download(url: &str, path: &str) -> bool {
    (|| -> Result<(), Box<dyn std::error::Error>> {
        let resp = agent().get(url).call()?;
        let mut reader = resp.into_reader();
        let mut file = std::fs::File::create(path)?;
        let mut buf = [0u8; 8192];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut file, &buf[..n])?;
        }
        Ok(())
    })()
    .is_ok()
}

/// Generic request. `method` is case-insensitive ("GET"/"POST"/...). A
/// non-empty `body` is sent with `content_type` (defaults to text/plain when
/// blank). Returns "status\n<body>" — the numeric status on the first line,
/// the body after the first newline — so callers get both without a struct.
pub fn request(method: &str, url: &str, content_type: &str, body: &str) -> String {
    let ct = if content_type.is_empty() {
        "text/plain"
    } else {
        content_type
    };
    let mut req = agent().request(&method.to_uppercase(), url);
    if !body.is_empty() {
        req = req.set("Content-Type", ct);
    }
    let result = if body.is_empty() {
        req.call()
    } else {
        req.send_string(body)
    };
    match result {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.into_string().unwrap_or_default();
            format!("{status}\n{text}")
        }
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            format!("{code}\n{text}")
        }
        Err(e) => format!("0\n{e}"),
    }
}
