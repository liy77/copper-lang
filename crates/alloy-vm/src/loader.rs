//! Import resolution for Alloy.
//!
//! - `import { f } from cstd|http|fs|...` → native stdlib (resolved at runtime).
//! - `import { f } from mod` with a sibling `mod.crs` → the `.crs` is parsed and
//!   its items are **merged** into the program (recursive, with cycle guard). This
//!   is what lets multi-file projects run instantly, without rustc.
//! - `import { f } from mod` with a sibling `mod.rs` → the interpreter **does not
//!   run Rust**; signals [`LoadOutcome::NeedsCforge`] so the caller delegates to
//!   `cforge` (transpile + native compile).

use crate::bytecode;
use copper_syntax::program::{parse_program, Item, Program};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub enum LoadOutcome {
    /// Program ready to interpret (local imports merged; native stdlib).
    Program(Program),
    /// There is a `.rs` import (module `name`, file `path`): use cforge.
    NeedsCforge { module: String, rs_path: PathBuf },
}

/// Is a module part of the native stdlib (resolved at runtime)?
fn is_stdlib(module: &str) -> bool {
    crate::stdlib::handles(module) || crate::stdlib_ext::handles(module)
}

/// Loads a file for execution: `.loy` bytecode (already self-contained) or
/// `.crs` source with local imports resolved.
pub fn load_runnable(path: &Path) -> Result<LoadOutcome, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    if bytecode::is_bytecode(&bytes) {
        return Ok(LoadOutcome::Program(bytecode::load(&bytes)?));
    }
    let src = String::from_utf8(bytes).map_err(|_| "file is not UTF-8 or .loy".to_string())?;
    resolve_source(&src, path)
}

/// Parses `src` (from `path`) and recursively merges local `.crs` imports.
/// Stops at the first `.rs` import found.
pub fn resolve_source(src: &str, path: &Path) -> Result<LoadOutcome, String> {
    let prog = parse_program(src);
    if !prog.errors.is_empty() {
        let msg = prog
            .errors
            .iter()
            .map(|e| format!("sintaxe @ {}..{}: {}", e.span.start, e.span.end, e.message))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(msg);
    }
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut merged: Vec<Item> = Vec::new();
    let mut visited = HashSet::new();
    if let Some(rs) = collect(prog.items, base, &mut merged, &mut visited)? {
        return Ok(rs);
    }
    Ok(LoadOutcome::Program(Program {
        items: merged,
        errors: Vec::new(),
    }))
}

/// Appends `items` to `out`, resolving local imports. Returns
/// `Some(NeedsCforge)` if it encounters a `.rs`.
fn collect(
    items: Vec<Item>,
    base: &Path,
    out: &mut Vec<Item>,
    visited: &mut HashSet<PathBuf>,
) -> Result<Option<LoadOutcome>, String> {
    for item in items {
        if let Item::Import { path: module, .. } = &item {
            // item imports with a simple module (not path/url, not "./x.rs")
            let module = module.trim_matches('"');
            if is_stdlib(module) {
                out.push(item);
                continue;
            }
            // `import ... from "./foo.rs"` ou `from foors` com foors.rs irmão.
            let rs_candidate = sibling(base, module, "rs");
            if let Some(rs) = &rs_candidate {
                if rs.is_file() {
                    return Ok(Some(LoadOutcome::NeedsCforge {
                        module: module.to_string(),
                        rs_path: rs.clone(),
                    }));
                }
            }
            let crs_candidate = sibling(base, module, "crs");
            if let Some(crs) = crs_candidate {
                if crs.is_file() {
                    let canon = crs.canonicalize().unwrap_or(crs.clone());
                    if visited.insert(canon) {
                        let sub_src = std::fs::read_to_string(&crs)
                            .map_err(|e| format!("could not read {}: {e}", crs.display()))?;
                        let sub = parse_program(&sub_src);
                        if !sub.errors.is_empty() {
                            return Err(format!(
                                "errors in {}: {} syntax error(s)",
                                crs.display(),
                                sub.errors.len()
                            ));
                        }
                        let sub_base = crs.parent().unwrap_or(base).to_path_buf();
                        if let Some(rs) = collect(sub.items, &sub_base, out, visited)? {
                            return Ok(Some(rs));
                        }
                    }
                    // keep the import (harmless; the items were already merged)
                    out.push(item);
                    continue;
                }
            }
            // unknown module (e.g. Rust path in a signature) — preserve it.
            out.push(item);
        } else {
            out.push(item);
        }
    }
    Ok(None)
}

/// Resolves `<base>/<module>.<ext>`. Accepts `module` as a simple name or a
/// relative path (`./foo`, `foo/bar`), with or without the extension already in the name.
fn sibling(base: &Path, module: &str, ext: &str) -> Option<PathBuf> {
    let m = module.trim_start_matches("./");
    if m.is_empty() || m.contains("::") {
        return None; // Rust path (std::num::…), not a local file
    }
    let p = base.join(m);
    if p.extension().and_then(|e| e.to_str()) == Some(ext) {
        Some(p)
    } else {
        Some(p.with_extension(ext))
    }
}
