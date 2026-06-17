// `time` module — clocks / sleep / timestamps, std-only.
// Bundled into `pub mod time { ... }` alongside std/time.crs.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn since_epoch() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO)
}

/// Unix time in milliseconds.
pub fn now_ms() -> i64 {
    since_epoch().as_millis() as i64
}

/// Unix time in whole seconds.
pub fn now_secs() -> i64 {
    since_epoch().as_secs() as i64
}

/// Unix time in nanoseconds.
pub fn now_nanos() -> i64 {
    since_epoch().as_nanos() as i64
}

/// Block the current thread for `ms` milliseconds.
pub fn sleep_ms(ms: i64) {
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(ms as u64));
    }
}

/// Block the current thread for `s` seconds.
pub fn sleep_secs(s: i64) {
    if s > 0 {
        std::thread::sleep(Duration::from_secs(s as u64));
    }
}

/// A monotonic millisecond counter from a fixed process-start instant —
/// safe for measuring elapsed durations (unaffected by wall-clock jumps).
pub fn mono_ms() -> i64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_millis() as i64
}

/// Format a Unix-seconds timestamp as UTC "YYYY-MM-DDTHH:MM:SSZ".
/// Uses the civil-from-days algorithm (exact, no leap-second handling, no
/// external crate).
pub fn iso8601(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // days since 1970-01-01 -> civil (y, m, d). Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    format!("{year:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// iso8601 of the current time.
pub fn now_iso() -> String {
    iso8601(now_secs())
}
