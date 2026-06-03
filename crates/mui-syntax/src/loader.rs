//! Multi-file loading + the component registry.
//!
//! `import { Card } from "./card.mui"` lets one MUI file use views defined in
//! another as elements. The [`load`] function parses an entry file, follows its
//! imports (relative to each importing file), and returns a [`Loaded`] bundle:
//! the entry document plus a [`Registry`] mapping every importable view name to
//! its [`View`]. Both the runtime and the codegen consume the registry to
//! instantiate `Card(...)` by inlining `Card`'s body with the call's args bound
//! to its params.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{Document, View};

/// A view available for instantiation as an element, keyed by name.
pub type Registry = HashMap<String, View>;

/// The result of loading an entry file and its import graph.
pub struct Loaded {
    /// The parsed entry document (its own views, app block, imports).
    pub entry: Document,
    /// Every imported component view, by name. The entry's own views are not
    /// included here (they're in `entry.views`); only names brought in via
    /// `import` from other files. Use [`Loaded::component`] to look one up.
    pub components: Registry,
    /// Non-fatal load problems (missing file, unknown name) — surfaced so the
    /// caller can report them without aborting the render.
    pub warnings: Vec<String>,
    /// Every file that contributed to this load: the entry plus every import
    /// that was successfully read, transitively (normalized, de-duplicated).
    /// Hot-reload watches all of these so editing an imported component (e.g.
    /// `card.mui`) reloads the window, not just edits to the entry file.
    pub sources: Vec<PathBuf>,
}

impl Loaded {
    /// Look up a component (imported view) by element name.
    pub fn component(&self, name: &str) -> Option<&View> {
        self.components.get(name)
    }

    /// The full set of views instantiable from the entry document: the imported
    /// components PLUS the entry file's *own* views — so sibling `view`s in one
    /// file can use each other as elements (e.g. an `Installer` view rendering a
    /// `Pill` view defined just below it). Imported names win on a clash. Both
    /// the runtime and the codegen build from this (not the bare `components`
    /// map, which is imports-only).
    pub fn registry(&self) -> Registry {
        let mut reg = self.components.clone();
        for v in &self.entry.views {
            reg.entry(v.name.clone()).or_insert_with(|| v.clone());
        }
        reg
    }
}

/// Parse `entry_path` and resolve its import graph. Reads files from disk; for
/// a string-only flow (tests / in-memory), use [`load_from`].
pub fn load(entry_path: &Path) -> std::io::Result<Loaded> {
    let src = std::fs::read_to_string(entry_path)?;
    Ok(load_with(&src, entry_path, &mut |p| {
        std::fs::read_to_string(p)
    }))
}

/// Like [`load`] but takes the already-read entry source (so callers that
/// already hold it — e.g. the hot-reload loop — don't read twice).
pub fn load_from(entry_src: &str, entry_path: &Path) -> Loaded {
    load_with(entry_src, entry_path, &mut |p| std::fs::read_to_string(p))
}

/// Core loader, parameterised over a file reader so it's testable without
/// touching disk. Resolves imports breadth-first, relative to each importing
/// file's directory, de-duplicating already-visited paths.
pub fn load_with(
    entry_src: &str,
    entry_path: &Path,
    read: &mut dyn FnMut(&Path) -> std::io::Result<String>,
) -> Loaded {
    let entry = crate::parse(entry_src);
    let mut components: Registry = HashMap::new();
    let mut warnings = Vec::new();
    let mut visited: Vec<PathBuf> = vec![normalize(entry_path)];

    // Work-list of (importing-file-dir, import) to resolve.
    let mut queue: Vec<(PathBuf, crate::ast::Import)> = entry
        .imports
        .iter()
        .map(|imp| (dir_of(entry_path), imp.clone()))
        .collect();

    while let Some((base_dir, imp)) = queue.pop() {
        let resolved = resolve_path(&base_dir, &imp.path);
        let norm = normalize(&resolved);
        let already = visited.contains(&norm);
        let src = match read(&resolved) {
            Ok(s) => s,
            Err(e) => {
                warnings.push(format!("import {:?}: {}", imp.path, e));
                continue;
            }
        };
        if !already {
            visited.push(norm);
        }
        // Copper/Rust imports contribute logic to the generated crate, not MUI
        // views. We still read them (so hot-reload watches them via `sources`),
        // but we don't parse them as MUI or look for view names in them.
        if !imp.kind.is_mui() {
            continue;
        }
        let doc = crate::parse(&src);
        // Register the requested names that this file actually defines.
        for name in &imp.names {
            match doc.views.iter().find(|v| &v.name == name) {
                Some(v) => {
                    components.insert(name.clone(), v.clone());
                }
                None => warnings.push(format!("`{}` not found in {:?}", name, imp.path)),
            }
        }
        // Follow this file's own imports (transitive), relative to its dir.
        if !already {
            let this_dir = dir_of(&resolved);
            for nested in &doc.imports {
                queue.push((this_dir.clone(), nested.clone()));
            }
        }
    }

    Loaded {
        entry,
        components,
        warnings,
        // `visited` is the entry + every successfully-read import (normalized,
        // de-duplicated) — exactly the set hot-reload should watch.
        sources: visited,
    }
}

fn dir_of(p: &Path) -> PathBuf {
    p.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Resolve an import path string against the importing file's directory. A
/// relative path joins onto `base_dir`; an absolute path is used as-is.
fn resolve_path(base_dir: &Path, raw: &str) -> PathBuf {
    let p = Path::new(raw);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base_dir.join(p)
    }
}

/// Best-effort path normalization for cycle detection (canonicalize when the
/// file exists, else the lexical path). Avoids re-reading a file twice.
fn normalize(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_imported_component() {
        // Two in-memory files: app.mui imports Card from card.mui.
        let app_src = "import { Card } from \"./card.mui\"\nview Main() { Card(title: \"hi\") }";
        let card_src = "view Card(title: string = \"\") { Text(\"${title}\") }";

        let mut read = |p: &Path| -> std::io::Result<String> {
            if p.to_string_lossy().replace('\\', "/").ends_with("card.mui") {
                Ok(card_src.to_string())
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no"))
            }
        };
        let loaded = load_with(app_src, Path::new("/proj/app.mui"), &mut read);
        assert!(
            loaded.warnings.is_empty(),
            "warnings: {:?}",
            loaded.warnings
        );
        let card = loaded.component("Card").expect("Card registered");
        assert_eq!(card.name, "Card");
        assert_eq!(card.params.len(), 1);
        // `sources` is the hot-reload watch set: the entry + the imported file.
        assert_eq!(loaded.sources.len(), 2, "sources: {:?}", loaded.sources);
        assert!(loaded
            .sources
            .iter()
            .any(|p| p.to_string_lossy().replace('\\', "/").ends_with("card.mui")));
    }

    #[test]
    fn missing_name_warns_not_panics() {
        let app_src = "import { Nope } from \"./card.mui\"\nview Main() { Text(\"x\") }";
        let card_src = "view Card() { Text(\"hi\") }";
        let mut read = |_: &Path| -> std::io::Result<String> { Ok(card_src.to_string()) };
        let loaded = load_with(app_src, Path::new("/proj/app.mui"), &mut read);
        assert!(loaded.component("Nope").is_none());
        assert_eq!(loaded.warnings.len(), 1);
    }

    #[test]
    fn registry_includes_same_file_views() {
        // A file whose entry view renders a sibling view defined in the same
        // file (the `Pill`/`LogLine` case): `components` is imports-only, but
        // `registry()` must expose the file's own views so they're instantiable.
        let src = "view Main() { Pill(text: \"x\") }\nview Pill(text: string = \"\") { Text(\"${text}\") }";
        let mut read = |_: &Path| -> std::io::Result<String> {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        };
        let loaded = load_with(src, Path::new("/proj/app.mui"), &mut read);
        assert!(
            loaded.component("Pill").is_none(),
            "components is imports-only"
        );
        let reg = loaded.registry();
        assert!(reg.contains_key("Pill"), "registry exposes same-file Pill");
        assert!(
            reg.contains_key("Main"),
            "registry exposes the entry view too"
        );
    }
}
