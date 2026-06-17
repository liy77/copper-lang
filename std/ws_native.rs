// `ws` module — minimal RFC 6455 WebSocket client over a std TcpStream.
// Bundled into `pub mod ws { ... }` alongside std/ws.crs.
//
// Plaintext `ws://` only (no TLS, hence no external crate). Implements just
// enough of the protocol for one-shot request/reply scripting: the HTTP
// upgrade handshake (with an inline SHA-1 for the accept key), client-masked
// text frames, and reading a single server text frame.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Parse "ws://host:port/path" -> (host:port, path). Defaults: port 80,
/// path "/". Returns None for non-ws schemes (e.g. wss://).
fn parse_ws_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("ws://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let hostport = if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    Some((hostport, path.to_string()))
}

/// base64 of raw bytes (used for the 16-byte nonce key + the accept hash).
fn b64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Minimal SHA-1 (handshake accept key only — not a security primitive).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let ml = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&ml.to_be_bytes());

    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// A weakly-random 16-byte nonce for Sec-WebSocket-Key (uniqueness, not
/// secrecy — std-only, no rng crate). xorshift seeded from the nanosecond
/// clock, perturbed by a per-call counter so two nonces in the same tick
/// still differ.
fn nonce16() -> [u8; 16] {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let bump = COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
        ^ bump;
    if seed == 0 {
        seed = 0x9E37_79B9_7F4A_7C15;
    }
    let mut out = [0u8; 16];
    for b in out.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        *b = (seed & 0xFF) as u8;
    }
    out
}

/// Encode a client text frame: FIN+text opcode, masked payload (RFC 6455
/// requires client frames to be masked).
fn encode_text_frame(payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x81); // FIN=1, opcode=0x1 (text)
    let len = payload.len();
    if len < 126 {
        frame.push(0x80 | len as u8);
    } else if len <= 0xFFFF {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    for (i, &byte) in payload.iter().enumerate() {
        frame.push(byte ^ mask[i % 4]);
    }
    frame
}

/// Open a ws:// connection and perform the upgrade handshake. Returns the
/// connected stream, or an error string on failure.
fn connect(url: &str) -> Result<TcpStream, String> {
    let (hostport, path) = parse_ws_url(url).ok_or_else(|| "ws: only ws:// URLs are supported".to_string())?;
    let host = hostport.split(':').next().unwrap_or("").to_string();
    let mut stream = TcpStream::connect(&hostport).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

    let key = b64(&nonce16());
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    stream.flush().ok();

    // Read until the end of the HTTP response headers.
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => buf.push(byte[0]),
            Err(e) => return Err(e.to_string()),
        }
        if buf.len() > 8192 {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let accept_expected = b64(&sha1(format!("{key}{GUID}").as_bytes()));
    if !head.contains("101") || !head.to_lowercase().contains(&accept_expected.to_lowercase()) {
        return Err("ws: handshake rejected".to_string());
    }
    Ok(stream)
}

/// Read one server data frame's text payload (handles 7/16/64-bit lengths;
/// server frames are unmasked). "" on close/error.
fn read_text_frame(stream: &mut TcpStream) -> String {
    let mut hdr = [0u8; 2];
    if stream.read_exact(&mut hdr).is_err() {
        return String::new();
    }
    let masked = hdr[1] & 0x80 != 0;
    let mut len = (hdr[1] & 0x7F) as usize;
    if len == 126 {
        let mut ext = [0u8; 2];
        if stream.read_exact(&mut ext).is_err() {
            return String::new();
        }
        len = u16::from_be_bytes(ext) as usize;
    } else if len == 127 {
        let mut ext = [0u8; 8];
        if stream.read_exact(&mut ext).is_err() {
            return String::new();
        }
        len = u64::from_be_bytes(ext) as usize;
    }
    let mut mask = [0u8; 4];
    if masked && stream.read_exact(&mut mask).is_err() {
        return String::new();
    }
    let mut payload = vec![0u8; len];
    if stream.read_exact(&mut payload).is_err() {
        return String::new();
    }
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    String::from_utf8_lossy(&payload).into_owned()
}

/// Connect, send one text `message`, read one text reply, close. "" on error.
pub fn request(url: &str, message: &str) -> String {
    match connect(url) {
        Ok(mut stream) => {
            let frame = encode_text_frame(message.as_bytes(), nonce16()[..4].try_into().unwrap());
            if stream.write_all(&frame).is_err() {
                return String::new();
            }
            stream.flush().ok();
            read_text_frame(&mut stream)
        }
        Err(_) => String::new(),
    }
}

/// Connect, send one text `message`, close. true on success.
pub fn send(url: &str, message: &str) -> bool {
    match connect(url) {
        Ok(mut stream) => {
            let frame = encode_text_frame(message.as_bytes(), nonce16()[..4].try_into().unwrap());
            stream.write_all(&frame).is_ok() && stream.flush().is_ok()
        }
        Err(_) => false,
    }
}
