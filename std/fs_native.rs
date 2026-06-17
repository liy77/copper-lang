// `fs` module — filesystem operations, std-only.
// Bundled into `pub mod fs { ... }` alongside std/fs.crs. Fallible
// operations return a bool (true = success) or a sentinel, so Copper code
// doesn't have to thread Result types.

use std::path::Path;

/// Read a whole file to a String. "" on any error (missing, not UTF-8, …).
pub fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Write `content`, truncating/creating the file. true on success.
pub fn write(path: &str, content: &str) -> bool {
    std::fs::write(path, content).is_ok()
}

/// Append `content` to a file, creating it if absent. true on success.
pub fn append(path: &str, content: &str) -> bool {
    use std::io::Write;
    (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        f.write_all(content.as_bytes())
    })()
    .is_ok()
}

pub fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

pub fn is_file(path: &str) -> bool {
    Path::new(path).is_file()
}

pub fn is_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Create a directory and all missing parents. true on success (or if it
/// already exists).
pub fn mkdir(path: &str) -> bool {
    std::fs::create_dir_all(path).is_ok()
}

/// Remove a single file. true on success.
pub fn remove_file(path: &str) -> bool {
    std::fs::remove_file(path).is_ok()
}

/// Remove a directory and its contents recursively. true on success.
pub fn remove_dir(path: &str) -> bool {
    std::fs::remove_dir_all(path).is_ok()
}

/// Copy `src` to `dst` (overwriting). true on success.
pub fn copy(src: &str, dst: &str) -> bool {
    std::fs::copy(src, dst).is_ok()
}

/// Rename/move `src` to `dst`. true on success.
pub fn rename(src: &str, dst: &str) -> bool {
    std::fs::rename(src, dst).is_ok()
}

/// File size in bytes, or -1 on error.
pub fn size(path: &str) -> i64 {
    std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(-1)
}

/// List directory entries as newline-joined paths ("" on error / empty).
pub fn list(path: &str) -> String {
    match std::fs::read_dir(path) {
        Ok(it) => {
            let mut entries: Vec<String> = it
                .filter_map(|e| e.ok())
                .map(|e| e.path().display().to_string())
                .collect();
            entries.sort();
            entries.join("\n")
        }
        Err(_) => String::new(),
    }
}

/// Alias for `read` — named for intent when the caller will split on "\n".
pub fn read_lines(path: &str) -> String {
    read(path)
}
