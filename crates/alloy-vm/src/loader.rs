//! Resolução de imports para o Alloy.
//!
//! - `import { f } from cstd|http|fs|...` → stdlib nativa (resolvida em runtime).
//! - `import { f } from mod` com um `mod.crs` irmão → o `.crs` é parseado e seus
//!   itens são **fundidos** no programa (recursivo, com guarda de ciclo). É o que
//!   permite projetos multi-arquivo rodarem instantâneo, sem rustc.
//! - `import { f } from mod` com um `mod.rs` irmão → o interpretador **não roda
//!   Rust**; sinaliza [`LoadOutcome::NeedsCforge`] para o chamador delegar ao
//!   `cforge` (transpila + compila nativo).

use crate::bytecode;
use copper_syntax::program::{parse_program, Item, Program};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub enum LoadOutcome {
    /// Programa pronto para interpretar (imports locais fundidos; stdlib nativa).
    Program(Program),
    /// Há um import de `.rs` (módulo `name`, arquivo `path`): use o cforge.
    NeedsCforge { module: String, rs_path: PathBuf },
}

/// Um módulo é stdlib nativa (resolvida em runtime)?
fn is_stdlib(module: &str) -> bool {
    crate::stdlib::handles(module) || crate::stdlib_ext::handles(module)
}

/// Carrega um arquivo para execução: bytecode `.loy` (já self-contained) ou
/// fonte `.crs` com imports locais resolvidos.
pub fn load_runnable(path: &Path) -> Result<LoadOutcome, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("não consegui ler {}: {e}", path.display()))?;
    if bytecode::is_bytecode(&bytes) {
        return Ok(LoadOutcome::Program(bytecode::load(&bytes)?));
    }
    let src = String::from_utf8(bytes).map_err(|_| "arquivo não é UTF-8 nem .loy".to_string())?;
    resolve_source(&src, path)
}

/// Parseia `src` (vindo de `path`) e funde recursivamente os imports `.crs`
/// locais. Para no primeiro import de `.rs` encontrado.
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

/// Acrescenta `items` em `out`, resolvendo imports locais. Retorna
/// `Some(NeedsCforge)` se topar com um `.rs`.
fn collect(
    items: Vec<Item>,
    base: &Path,
    out: &mut Vec<Item>,
    visited: &mut HashSet<PathBuf>,
) -> Result<Option<LoadOutcome>, String> {
    for item in items {
        if let Item::Import { path: module, .. } = &item {
            // imports de itens com módulo simples (não path/url, não "./x.rs")
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
                            .map_err(|e| format!("não consegui ler {}: {e}", crs.display()))?;
                        let sub = parse_program(&sub_src);
                        if !sub.errors.is_empty() {
                            return Err(format!(
                                "erros em {}: {} erro(s) de sintaxe",
                                crs.display(),
                                sub.errors.len()
                            ));
                        }
                        let sub_base = crs.parent().unwrap_or(base).to_path_buf();
                        if let Some(rs) = collect(sub.items, &sub_base, out, visited)? {
                            return Ok(Some(rs));
                        }
                    }
                    // mantém o import (inofensivo; os itens já foram fundidos)
                    out.push(item);
                    continue;
                }
            }
            // módulo desconhecido (ex.: caminho Rust em assinatura) — preserva.
            out.push(item);
        } else {
            out.push(item);
        }
    }
    Ok(None)
}

/// Resolve `<base>/<module>.<ext>`. Aceita `module` como nome simples ou caminho
/// relativo (`./foo`, `foo/bar`), com ou sem a extensão já no nome.
fn sibling(base: &Path, module: &str, ext: &str) -> Option<PathBuf> {
    let m = module.trim_start_matches("./");
    if m.is_empty() || m.contains("::") {
        return None; // caminho Rust (std::num::…), não um arquivo local
    }
    let p = base.join(m);
    if p.extension().and_then(|e| e.to_str()) == Some(ext) {
        Some(p)
    } else {
        Some(p.with_extension(ext))
    }
}
