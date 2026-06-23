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
use copper_syntax::program::{parse_program, ImportKind, Item, Program};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The `cstd` module source — the single source of truth, shared with cforge.
/// Bundled so Alloy interprets the *same* code instead of duplicating it.
const CSTD_SRC: &str = include_str!("../../../std/cstd.crs");

/// `cstd` functions whose bodies use Rust-std builder/iterator chains (or `<<`
/// shifts) the interpreter can't walk — these stay implemented natively in
/// [`crate::stdlib`] and are NOT merged from the source.
const CSTD_NATIVE: &[&str] = &["run", "list_dir", "append_file", "rand_int"];

pub enum LoadOutcome {
    /// Program ready to interpret (local imports merged; native stdlib).
    Program(Program),
    /// There is a `.rs` import (module `name`, file `path`): use cforge.
    NeedsCforge { module: String, rs_path: PathBuf },
}

/// An `import { a, b } from foo` resolved to a sibling `foo.rs` — to be run via
/// embedded wasm (see [`crate::wasm`]) instead of delegating to cforge.
pub struct RsImport {
    pub names: Vec<String>,
    pub rs_path: PathBuf,
}

/// The fully-resolved input to run: the (Copper) program, plus the Rust to run
/// via wasm — either as `.rs` files still to compile ([`RsImport`]) or as
/// already-compiled modules embedded in a `.loy` ([`bytecode::WasmModule`]).
pub struct Resolved {
    pub program: Program,
    pub rs_imports: Vec<RsImport>,
    pub wasm_modules: Vec<bytecode::WasmModule>,
}

/// Like [`resolve_source`], but instead of bailing to `NeedsCforge` at the first
/// `.rs` import, it **collects** the Rust imports so the caller can run them via
/// embedded wasm. For a `.loy`, surfaces the embedded wasm modules directly.
pub fn resolve_runnable_wasm(path: &Path) -> Result<Resolved, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    if bytecode::is_bytecode(&bytes) {
        let (program, wasm_modules) = bytecode::load(&bytes)?;
        return Ok(Resolved {
            program,
            rs_imports: Vec::new(),
            wasm_modules,
        });
    }
    let src = String::from_utf8(bytes).map_err(|_| "file is not UTF-8 or .loy".to_string())?;
    let prog = parse_program(&src);
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
    let mut rs_imports: Vec<RsImport> = Vec::new();
    collect(
        prog.items,
        base,
        &mut merged,
        &mut visited,
        Some(&mut rs_imports),
    )?;
    Ok(Resolved {
        program: Program {
            items: merged,
            errors: Vec::new(),
        },
        rs_imports,
        wasm_modules: Vec::new(),
    })
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
        let (prog, _wasm) = bytecode::load(&bytes)?;
        return Ok(LoadOutcome::Program(prog));
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
    if let Some(rs) = collect(prog.items, base, &mut merged, &mut visited, None)? {
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
    mut rs_imports: Option<&mut Vec<RsImport>>,
) -> Result<Option<LoadOutcome>, String> {
    for item in items {
        if let Item::Import {
            path: module, kind, ..
        } = &item
        {
            // item imports with a simple module (not path/url, not "./x.rs")
            let module = module.trim_matches('"');
            if is_stdlib(module) {
                // `cstd` is interpreted from its Copper source (single source of
                // truth) — merge the imported functions so the interpreter runs
                // them. The few native-only ones (CSTD_NATIVE) resolve through
                // the stdlib dispatch instead and are skipped here.
                if module == "cstd" {
                    if let Item::Import { kind, .. } = &item {
                        merge_cstd(kind, out);
                    }
                }
                out.push(item);
                continue;
            }
            // `import ... from "./foo.rs"` ou `from foors` com foors.rs irmão.
            let rs_candidate = sibling(base, module, "rs");
            if let Some(rs) = &rs_candidate {
                if rs.is_file() {
                    // In wasm mode, collect the Rust import (the caller compiles
                    // it to wasm and runs it embedded). Otherwise signal cforge.
                    match (rs_imports.as_deref_mut(), kind) {
                        (Some(acc), ImportKind::Items(names)) => {
                            acc.push(RsImport {
                                names: names.clone(),
                                rs_path: rs.clone(),
                            });
                            out.push(item);
                            continue;
                        }
                        _ => {
                            return Ok(Some(LoadOutcome::NeedsCforge {
                                module: module.to_string(),
                                rs_path: rs.clone(),
                            }));
                        }
                    }
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
                        if let Some(rs) = collect(
                            sub.items,
                            &sub_base,
                            out,
                            visited,
                            rs_imports.as_deref_mut(),
                        )? {
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

/// Merges the requested `cstd` functions (from the bundled [`CSTD_SRC`]) into
/// `out`. Honours the import list (`import { a, b } from cstd` brings only `a`,
/// `b`; `import * from cstd` brings all), skips the natively-handled functions
/// ([`CSTD_NATIVE`]), and never adds a name already present (so user definitions
/// and repeat imports don't duplicate). The source's own parse errors (the
/// native-only functions use constructs the AST parser can't lower) are ignored
/// — those items simply aren't merged.
fn merge_cstd(kind: &ImportKind, out: &mut Vec<Item>) {
    let wanted: Option<&[String]> = match kind {
        ImportKind::Items(names) => Some(names),
        ImportKind::Glob | ImportKind::Alias(_) => None, // None = take all eligible
    };
    let prog = parse_program(CSTD_SRC);
    for it in prog.items {
        let Item::Function { name, .. } = &it else {
            continue;
        };
        if CSTD_NATIVE.contains(&name.as_str()) {
            continue;
        }
        if let Some(names) = wanted {
            if !names.iter().any(|n| n == name) {
                continue;
            }
        }
        let already = out
            .iter()
            .any(|o| matches!(o, Item::Function { name: n, .. } if n == name));
        if already {
            continue;
        }
        out.push(it);
    }
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
