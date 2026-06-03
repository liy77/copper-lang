// The Copper type-alias table is defined ONCE in copper-syntax's lexicon so
// the transpiler and the typed expression AST never disagree about what
// `int` / `uint` / `string` / `void` / ... mean. `convert_type` simply
// re-exports the shared resolver.
pub use copper_syntax::lexicon::{convert_type, COPPER_TYPES};

pub fn convert_type_with_marking(value: &str) -> (String, Option<String>) {
    let mut kind = value.to_string();
    let mut is_optional = false;
    let mut data_type_used = None;

    if value.ends_with("?") {
        is_optional = true;
        kind = value[..value.len() - 1].to_string();
    }

    for (copper, rust) in COPPER_TYPES.iter() {
        if kind == *copper {
            kind = rust.to_string();

            // Mark which data type is being used
            match *copper {
                "json" => data_type_used = Some("json".to_string()),
                "xml" => data_type_used = Some("xml".to_string()),
                "toml" => data_type_used = Some("toml".to_string()),
                _ => {}
            }
            break;
        }
    }

    if is_optional {
        kind = format!("Option<{}>", kind);
    }

    (kind, data_type_used)
}
