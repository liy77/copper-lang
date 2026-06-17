// The Copper type-alias table is defined ONCE in copper-syntax's lexicon so
// the transpiler and the typed expression AST never disagree about what
// `int` / `uint` / `string` / `void` / ... mean. `convert_type` simply
// re-exports the shared resolver.
pub use copper_syntax::lexicon::{convert_type, COPPER_TYPES};

pub fn convert_type_with_marking(value: &str) -> (String, Option<String>) {
    let mut data_type_used = None;

    // Track json/xml/toml usage from the base spelling (strip an optional `?`).
    let base = value.strip_suffix('?').unwrap_or(value);
    for (copper, _rust) in COPPER_TYPES.iter() {
        if base == *copper {
            match *copper {
                "json" => data_type_used = Some("json".to_string()),
                "xml" => data_type_used = Some("xml".to_string()),
                "toml" => data_type_used = Some("toml".to_string()),
                _ => {}
            }
            break;
        }
    }

    // Delegate the actual lowering to convert_type so aliases inside generic
    // args recurse (`Vec<int>` -> `Vec<i64>`, `Rc<RefCell<int>>` -> ...).
    (convert_type(value), data_type_used)
}
