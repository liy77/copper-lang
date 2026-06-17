// `crypto` module — hashing + encoding helpers.
// Bundled into `pub mod crypto { ... }` alongside std/crypto.crs.
//
// base64 / hex / crc32 are hand-rolled (std-only). SHA-256/512 and HMAC use
// the `sha2` / `hmac` crates (correctness over cleverness for real hashes).

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 (with `=` padding) of the UTF-8 bytes of `s`.
pub fn base64_encode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
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

/// Decode standard base64 to a lossy-UTF-8 String. Invalid input -> "".
pub fn base64_decode(s: &str) -> String {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let cleaned: Vec<u8> = s.bytes().filter(|&b| b != b'=' && !b.is_ascii_whitespace()).collect();
    let mut out: Vec<u8> = Vec::with_capacity(cleaned.len() / 4 * 3);
    for chunk in cleaned.chunks(4) {
        let mut n = 0u32;
        let mut bits = 0;
        for &c in chunk {
            let Some(v) = val(c) else { return String::new() };
            n = (n << 6) | v;
            bits += 6;
        }
        // Emit the high bytes that are fully present.
        let bytes_avail = bits / 8;
        n <<= 24 - bits; // left-align into 3 bytes
        for i in 0..bytes_avail {
            out.push(((n >> (16 - i * 8)) & 0xFF) as u8);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lower-case hex of the UTF-8 bytes of `s`.
pub fn hex_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.as_bytes() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Decode a hex string to a lossy-UTF-8 String. Invalid input -> "".
pub fn hex_decode(s: &str) -> String {
    let t = s.trim();
    if t.len() % 2 != 0 {
        return String::new();
    }
    let mut out = Vec::with_capacity(t.len() / 2);
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let hi = (b[i] as char).to_digit(16);
        let lo = (b[i + 1] as char).to_digit(16);
        match (hi, lo) {
            (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
            _ => return String::new(),
        }
        i += 2;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// SHA-256 of `s`, as a 64-char lower-case hex digest.
pub fn sha256(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-512 of `s`, as a 128-char lower-case hex digest.
pub fn sha512(s: &str) -> String {
    use sha2::{Digest, Sha512};
    let mut h = Sha512::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// HMAC-SHA256 of `msg` under `key`, as a hex digest. "" on a bad key length
/// (HMAC accepts any key length, so this effectively never fails).
pub fn hmac_sha256(key: &str, msg: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type H = Hmac<Sha256>;
    match H::new_from_slice(key.as_bytes()) {
        Ok(mut mac) => {
            mac.update(msg.as_bytes());
            mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
        }
        Err(_) => String::new(),
    }
}

/// CRC-32 (IEEE) of the UTF-8 bytes of `s`, returned as an i64 holding the
/// unsigned 32-bit value. Std-only table-less bit algorithm.
pub fn crc32(s: &str) -> i64 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in s.as_bytes() {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    (!crc) as i64
}
