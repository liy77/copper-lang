//! Running imported Rust (`.rs`) inside Alloy via WebAssembly.
//!
//! The interpreter can't execute Rust directly. Instead of delegating the whole
//! program to `cforge` (the old `NeedsCforge` path), Alloy compiles each
//! imported `.rs` to a **`wasm32-unknown-unknown`** module once (cached by
//! content hash under `~/.alloy/cache`), then runs it with an embedded
//! [`wasmi`] interpreter. The Copper code stays tree-walked (instant); only the
//! Rust leaves cross into wasm. This keeps `.loy` portable — wasm is itself a
//! platform-independent bytecode.
//!
//! **Prototype scope:** scalar `i64` in/out, and the `.rs` must export the
//! function as `#[no_mangle] pub extern "C"`. Richer types (strings/structs via
//! WASI + an alloc ABI) and auto-generated export wrappers are the next phase —
//! see `docs/superpowers/specs/2026-06-22-alloy-wasm-interop-design.md`.

use std::path::{Path, PathBuf};
use wasmi::{Engine, Instance, Linker, Module, Store, Val};

/// A compiled+instantiated `.rs` module, ready to call.
pub struct WasmRuntime {
    store: Store<()>,
    instance: Instance,
}

impl WasmRuntime {
    /// Compiles `rs_path` to wasm (cached) and instantiates it.
    pub fn from_rs(rs_path: &Path) -> Result<Self, String> {
        let bytes = compile_rs_to_wasm(rs_path)?;
        Self::from_wasm(&bytes)
    }

    /// Instantiates an in-memory wasm module.
    pub fn from_wasm(wasm: &[u8]) -> Result<Self, String> {
        let engine = Engine::default();
        let module = Module::new(&engine, wasm).map_err(|e| format!("invalid wasm: {e}"))?;
        let mut store = Store::new(&engine, ());
        let linker = <Linker<()>>::new(&engine);
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("wasm link/start failed: {e}"))?;
        Ok(Self { store, instance })
    }

    /// `true` if the module exports a callable function named `name`.
    pub fn exports(&self, name: &str) -> bool {
        self.instance.get_func(&self.store, name).is_some()
    }

    /// Calls an exported function with `i64` args, returning its `i64` result
    /// (or `0` for a `()`-returning export). Prototype: `i64` only.
    pub fn call_i64(&mut self, name: &str, args: &[i64]) -> Result<i64, String> {
        let func = self
            .instance
            .get_func(&self.store, name)
            .ok_or_else(|| format!("wasm export `{name}` not found"))?;
        let params: Vec<Val> = args.iter().map(|n| Val::I64(*n)).collect();
        let ty = func.ty(&self.store);
        let mut results: Vec<Val> = ty.results().iter().map(|_| Val::I64(0)).collect();
        func.call(&mut self.store, &params, &mut results)
            .map_err(|e| format!("wasm call `{name}` failed: {e}"))?;
        match results.first() {
            Some(Val::I64(n)) => Ok(*n),
            Some(Val::I32(n)) => Ok(*n as i64),
            _ => Ok(0),
        }
    }
}

/// Compiles a single `.rs` file to a `wasm32-unknown-unknown` cdylib, caching
/// the output by content hash so repeat `alloy run`s skip rustc.
pub fn compile_rs_to_wasm(rs_path: &Path) -> Result<Vec<u8>, String> {
    let src =
        std::fs::read(rs_path).map_err(|e| format!("could not read {}: {e}", rs_path.display()))?;
    let hash = fnv1a(&src);
    let cache = cache_dir()?;
    let cached = cache.join(format!("{hash:016x}.wasm"));
    if let Ok(bytes) = std::fs::read(&cached) {
        return Ok(bytes);
    }

    let _ = std::fs::create_dir_all(&cache);
    let tmp = cache.join(format!("{hash:016x}.wasm.tmp"));
    let status = std::process::Command::new("rustc")
        .args([
            "--edition",
            "2021",
            "--target",
            "wasm32-unknown-unknown",
            "--crate-type",
            "cdylib",
            "-C",
            "opt-level=2",
            "-o",
        ])
        .arg(&tmp)
        .arg(rs_path)
        .status()
        .map_err(|e| format!("failed to launch rustc (is it installed?): {e}"))?;
    if !status.success() {
        return Err(format!(
            "rustc failed to compile {} to wasm",
            rs_path.display()
        ));
    }
    let bytes = std::fs::read(&tmp)
        .map_err(|e| format!("rustc produced no wasm for {}: {e}", rs_path.display()))?;
    let _ = std::fs::rename(&tmp, &cached);
    Ok(bytes)
}

/// `~/.alloy/cache` (created lazily), overridable via `ALLOY_CACHE_DIR`.
fn cache_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("ALLOY_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "no HOME/USERPROFILE to locate the Alloy cache".to_string())?;
    Ok(PathBuf::from(home).join(".alloy").join("cache"))
}

/// FNV-1a 64-bit — a tiny, dependency-free content hash for cache keys.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    // Runtime half (no rustc): build a module from WAT and call its exports.
    #[test]
    fn instantiate_and_call_i64() {
        let wasm = wat::parse_str(
            r#"(module
                 (func (export "add") (param i64 i64) (result i64)
                   local.get 0 local.get 1 i64.add)
                 (func (export "noop")))"#,
        )
        .expect("wat");
        let mut rt = WasmRuntime::from_wasm(&wasm).expect("instantiate");
        assert!(rt.exports("add"));
        assert!(!rt.exports("missing"));
        assert_eq!(rt.call_i64("add", &[20, 22]).unwrap(), 42);
        // a `()`-returning export yields 0.
        assert_eq!(rt.call_i64("noop", &[]).unwrap(), 0);
        assert!(rt.call_i64("missing", &[]).is_err());
    }

    #[test]
    fn cache_key_is_content_addressed() {
        assert_eq!(fnv1a(b"abc"), fnv1a(b"abc"));
        assert_ne!(fnv1a(b"abc"), fnv1a(b"abd"));
    }
}
