//! Shared Copper keyword + primitive-type descriptions.
//!
//! These are embedded at compile time from the **single source of truth**,
//! `editors/copper-lexicon.json`, which the MUI/CRM VS Code extension also
//! reads at runtime. Keeping both sides on one file means the keyword/type
//! hovers in `.crs` (this server) and in `.mui`/`.crm` (the MUI extension) can
//! never drift. Edit the JSON, not this file.

use once_cell::sync::Lazy;
use serde_json::Value;

/// One described word: `name`, a one-line signature `detail`, and a `doc`.
pub struct Entry {
    pub name: String,
    pub detail: String,
    pub doc: String,
}

const RAW: &str = include_str!("../../../editors/copper-lexicon.json");

static LEXICON: Lazy<Lexicon> = Lazy::new(|| {
    let v: Value = serde_json::from_str(RAW).unwrap_or(Value::Null);
    let parse = |key: &str| -> Vec<Entry> {
        v.get(key)
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| {
                        Some(Entry {
                            name: e.get("name")?.as_str()?.to_string(),
                            detail: e
                                .get("detail")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            doc: e
                                .get("doc")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    Lexicon {
        keywords: parse("keywords"),
        types: parse("types"),
    }
});

struct Lexicon {
    keywords: Vec<Entry>,
    types: Vec<Entry>,
}

/// Keyword entries, for completion (`label` + short `doc`).
pub fn keywords() -> &'static [Entry] {
    &LEXICON.keywords
}

/// Build the hover markdown for a keyword or primitive type, identical in
/// shape to what the MUI extension renders: a ```copper signature block plus
/// the description. Returns `None` for words not in the lexicon.
pub fn builtin_markdown(word: &str) -> Option<String> {
    let hit = LEXICON
        .keywords
        .iter()
        .chain(LEXICON.types.iter())
        .find(|e| e.name == word)?;
    Some(format!("```copper\n{}\n```\n\n{}", hit.detail, hit.doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_parses_and_has_core_words() {
        // If the JSON is malformed or empty, these are empty and the test
        // fails — guarding against a broken edit to the shared file.
        assert!(
            !keywords().is_empty(),
            "no keywords parsed from copper-lexicon.json"
        );
        for w in ["func", "mut", "struct", "impl", "if", "match", "import"] {
            assert!(
                keywords().iter().any(|e| e.name == w),
                "keyword `{w}` missing from copper-lexicon.json"
            );
        }
    }

    #[test]
    fn builtin_markdown_matches_expected_text() {
        // Locks the exact text so it stays in sync with what the MUI extension
        // shows for the same words.
        assert_eq!(
            builtin_markdown("func").as_deref(),
            Some("```copper\nfunc RetType name(params)\n```\n\nDeclares a function.")
        );
        assert_eq!(
            builtin_markdown("int").as_deref(),
            Some("```copper\nint\n```\n\n64-bit signed integer (alias for i64).")
        );
        assert!(builtin_markdown("not_a_keyword").is_none());
    }
}
