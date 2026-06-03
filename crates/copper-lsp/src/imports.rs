//! Import resolution for diagnostics + hover.
//!
//! Copper's `import { Name } from module` can point at the standard library
//! (`std`/`cstd`/…), a sibling source file (a local `.crs`/`.rs`/`.mui` module),
//! or an external crate declared in `properties.kson`. This module parses the
//! import lines (with byte spans, so the LSP can place diagnostics), and
//! resolves each target so the server can warn about unresolved imports and
//! describe resolved ones on hover.

use std::path::{Path, PathBuf};

use copper_syntax::ast::Span;

/// One parsed `import` line: the module and the names it brings in, each with a
/// byte span into the source.
pub struct ImportRef {
    pub module: String,
    pub module_span: Span,
    pub names: Vec<(String, Span)>,
}

/// What an import's module resolves to.
pub enum Resolution {
    /// Rust/Copper standard library (`std`, `core`, `alloc`, `crate`, …).
    Stdlib,
    /// Copper's bundled standard library.
    Cstd,
    /// A sibling source file (the given file name).
    Local(String),
    /// An external crate declared in properties.kson, with its version.
    Crate(String),
    /// Not found anywhere.
    Unknown,
}

impl Resolution {
    pub fn is_found(&self) -> bool {
        !matches!(self, Resolution::Unknown)
    }

    /// A markdown description for hover.
    pub fn describe(&self, module: &str) -> String {
        match self {
            Resolution::Stdlib => format!("**`{module}`** — standard library module."),
            Resolution::Cstd => {
                format!("**`{module}`** — Copper's bundled standard library (`cstd`).")
            }
            Resolution::Local(file) => {
                format!("**`{module}`** — local module (`{file}`).")
            }
            Resolution::Crate(ver) => {
                format!("**`{module}`** — external crate, version `{ver}` (from properties.kson).")
            }
            Resolution::Unknown => format!(
                "**`{module}`** — ⚠️ not found. Not std/cstd, no sibling file, and not in \
                 properties.kson. Run `cforge install {module}` to add it."
            ),
        }
    }
}

/// Parse every `import … from <module>` line in `text`. Single-line imports
/// (Copper's only form) with byte spans for the module and each imported name.
pub fn parse_imports(text: &str) -> Vec<ImportRef> {
    let mut out = Vec::new();
    let mut offset = 0usize; // byte offset of the current line's start
    for line in text.split_inclusive('\n') {
        let ltrim = line.trim_start();
        if ltrim.starts_with("import") && line.contains("from") {
            if let Some(imp) = parse_line(line, offset) {
                out.push(imp);
            }
        }
        offset += line.len();
    }
    out
}

fn parse_line(line: &str, line_off: usize) -> Option<ImportRef> {
    let fpos = line.rfind("from")?;
    let after = fpos + "from".len();
    let rest = &line[after..];
    let lead = rest.len() - rest.trim_start().len();
    let mod_start = after + lead;
    let module: String = line[mod_start..]
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if module.is_empty() {
        return None;
    }
    let mstart = line_off + mod_start;
    let module_span = Span::new(mstart as u32, (mstart + module.len()) as u32);

    let mut names = Vec::new();
    if let (Some(ob), Some(cb)) = (line.find('{'), line.find('}')) {
        if ob < cb {
            let mut cursor = ob + 1;
            for raw in line[ob + 1..cb].split(',') {
                let name = raw.trim();
                if name.is_empty() {
                    cursor += raw.len() + 1;
                    continue;
                }
                if let Some(rel) = line[cursor..cb].find(name) {
                    let ns = line_off + cursor + rel;
                    names.push((
                        name.to_string(),
                        Span::new(ns as u32, (ns + name.len()) as u32),
                    ));
                    cursor = cursor + rel + name.len();
                }
            }
        }
    } else {
        // `import name from module` (single binding, no braces).
        let after_kw = ltrim_after_keyword(line, "import");
        let nm: String = after_kw
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !nm.is_empty() && nm != "from" {
            if let Some(rel) = line.find(&nm) {
                let ns = line_off + rel;
                names.push((nm.clone(), Span::new(ns as u32, (ns + nm.len()) as u32)));
            }
        }
    }

    Some(ImportRef {
        module,
        module_span,
        names,
    })
}

fn ltrim_after_keyword<'a>(line: &'a str, kw: &str) -> &'a str {
    let i = line.find(kw).map(|p| p + kw.len()).unwrap_or(0);
    line[i..].trim_start()
}

/// Resolve an import module against the standard library, sibling files in
/// `doc_dir`, and the nearest `properties.kson` dependency list.
pub fn resolve(module: &str, doc_dir: Option<&Path>) -> Resolution {
    match module {
        "std" | "core" | "alloc" | "crate" | "self" | "super" => return Resolution::Stdlib,
        "cstd" => return Resolution::Cstd,
        _ => {}
    }
    if let Some(dir) = doc_dir {
        for ext in ["crs", "rs", "mui", "crm"] {
            if dir.join(format!("{module}.{ext}")).exists() {
                return Resolution::Local(format!("{module}.{ext}"));
            }
        }
        if let Some(ver) = find_dep_version(dir, module) {
            return Resolution::Crate(ver);
        }
    }
    Resolution::Unknown
}

/// Walk up from `dir` looking for a `properties.kson`; if found, return the
/// version pinned for `module` under `$dependencies` (string values only).
fn find_dep_version(dir: &Path, module: &str) -> Option<String> {
    let mut cur: Option<&Path> = Some(dir);
    let mut hops = 0;
    while let Some(d) = cur {
        let manifest = d.join("properties.kson");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            if let Some(v) = dep_version_in(&text, module) {
                return Some(v);
            }
        }
        cur = d.parent();
        hops += 1;
        if hops > 8 {
            break;
        }
    }
    None
}

fn dep_version_in(text: &str, module: &str) -> Option<String> {
    let mut in_deps = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            in_deps = trimmed == "$dependencies";
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            if k.trim() == module {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Directory of a `file://` document URI.
pub fn dir_of_uri(uri: &tower_lsp::lsp_types::Url) -> Option<PathBuf> {
    uri.to_file_path()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}
