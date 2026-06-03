// cstd helpers that Copper cannot yet express cleanly.
// Bundled by the compiler alongside std/cstd.crs into the same `cstd`
// module. Edit this file and rebuild cforge to ship updated helpers.

pub fn read_int(prompt: &str) -> i64 {
    use std::io::Write;
    loop {
        print!("{}", prompt);
        std::io::stdout().flush().ok();
        let mut buf = String::new();
        std::io::stdin()
            .read_line(&mut buf)
            .expect("failed to read stdin");
        match buf.trim().parse::<i64>() {
            Ok(n) => return n,
            Err(_) => eprintln!("invalid integer, try again"),
        }
    }
}

pub fn read_float(prompt: &str) -> f64 {
    use std::io::Write;
    loop {
        print!("{}", prompt);
        std::io::stdout().flush().ok();
        let mut buf = String::new();
        std::io::stdin()
            .read_line(&mut buf)
            .expect("failed to read stdin");
        match buf.trim().parse::<f64>() {
            Ok(n) => return n,
            Err(_) => eprintln!("invalid number, try again"),
        }
    }
}

pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

pub fn append_file(path: &str, content: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap_or_else(|e| panic!("append_file({}): {}", path, e));
    f.write_all(content.as_bytes())
        .unwrap_or_else(|e| panic!("append_file({}): {}", path, e));
}

pub fn list_dir(path: &str) -> Vec<String> {
    std::fs::read_dir(path)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .map(|e| e.path().display().to_string())
                .collect()
        })
        .unwrap_or_default()
}

pub fn run(cmd: &str) -> String {
    let output = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd").args(["/C", cmd]).output()
    } else {
        std::process::Command::new("sh").args(["-c", cmd]).output()
    };
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

pub fn split(s: &str, sep: &str) -> Vec<String> {
    s.split(sep).map(|p| p.to_string()).collect()
}

pub fn join(parts: &[String], sep: &str) -> String {
    parts.join(sep)
}

pub fn to_int(s: &str) -> Option<i64> {
    s.trim().parse::<i64>().ok()
}

pub fn to_float(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok()
}

pub fn rand_int(min: i64, max: i64) -> i64 {
    let mut x = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xCAFE_BABE);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    let span = (max - min).max(1) as u64;
    min + (x % span) as i64
}
