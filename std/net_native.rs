// `net` module — TCP / UDP helpers built on the Rust standard library.
// Bundled by the compiler into `pub mod net { ... }` alongside std/net.crs.
//
// Everything here is std-only (no external crates): std::net::{TcpStream,
// UdpSocket, ToSocketAddrs}. The helpers favour one-shot request/response
// shapes that return owned values (String / bool), which map cleanly onto
// Copper's value semantics — no socket handles cross the module boundary.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

/// Resolve a host (or "host:port") to its first socket address as a string
/// ("93.184.216.34:80"). Returns "" when resolution fails. A bare host with
/// no port resolves against port 0 (so the IP is still useful).
pub fn resolve(host: &str) -> String {
    let target = if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:0")
    };
    match target.to_socket_addrs() {
        Ok(mut it) => it.next().map(|a| a.to_string()).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// True if a TCP connection to `host:port` succeeds within 2 seconds.
pub fn is_port_open(host: &str, port: u16) -> bool {
    let addr = format!("{host}:{port}");
    let Ok(mut addrs) = addr.to_socket_addrs() else {
        return false;
    };
    let Some(sa) = addrs.next() else {
        return false;
    };
    TcpStream::connect_timeout(&sa, Duration::from_secs(2)).is_ok()
}

/// Connect to `addr` ("host:port"), send `payload`, read the full response
/// until the peer closes (or a 10s read timeout), and return it as a
/// lossy-UTF-8 String. Returns "" on any connection/IO error. Good for
/// simple line/redis/http-style request-response protocols.
pub fn tcp_request(addr: &str, payload: &str) -> String {
    match tcp_request_impl(addr, payload.as_bytes()) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
    }
}

fn tcp_request_impl(addr: &str, payload: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(payload)?;
    stream.flush()?;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            // A read timeout still returns whatever we already gathered.
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break
            }
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// Fire-and-forget: connect, write `payload`, close. Returns true on success.
pub fn tcp_send(addr: &str, payload: &str) -> bool {
    (|| -> std::io::Result<()> {
        let mut stream = TcpStream::connect(addr)?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        stream.write_all(payload.as_bytes())?;
        stream.flush()
    })()
    .is_ok()
}

/// Send a single UDP datagram to `addr`. Returns true on success.
pub fn udp_send(addr: &str, payload: &str) -> bool {
    (|| -> std::io::Result<()> {
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.send_to(payload.as_bytes(), addr)?;
        Ok(())
    })()
    .is_ok()
}

/// Send a UDP datagram to `addr` and wait (up to 5s) for one reply datagram,
/// returned as a lossy-UTF-8 String. "" on timeout/error.
pub fn udp_request(addr: &str, payload: &str) -> String {
    (|| -> std::io::Result<String> {
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.set_read_timeout(Some(Duration::from_secs(5)))?;
        sock.send_to(payload.as_bytes(), addr)?;
        let mut buf = [0u8; 65535];
        let (n, _) = sock.recv_from(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
    })()
    .unwrap_or_default()
}

/// Best-effort local outbound IP: opens a UDP socket "connected" to a public
/// address (no packets are sent) and reads back the chosen local interface.
/// Returns "" if it can't be determined.
pub fn local_ip() -> String {
    (|| -> std::io::Result<String> {
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.connect("8.8.8.8:80")?;
        Ok(sock.local_addr()?.ip().to_string())
    })()
    .unwrap_or_default()
}
