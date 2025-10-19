const COPPER_TYPES : [(&str, &str); 18] = [
    ("int", "i64"),
    ("float", "f64"),
    ("uint", "u64"),
    ("int8", "i8"),
    ("int16", "i16"),
    ("int32", "i32"),
    ("int64", "i64"),
    ("uint8", "u8"),
    ("uint16", "u16"),
    ("uint32", "u32"),
    ("uint64", "u64"),
    ("float32", "f32"),
    ("float64", "f64"),
    ("string", "String"),
    ("void", "()"),
    ("json", "JsonValue"),
    ("xml", "XmlValue"),
    ("toml", "TomlValue"),
];

pub fn convert_type(value: &str) -> String {
    let mut kind = value.to_string();
    let mut is_optional = false;

    if value.ends_with("?") {
        is_optional = true;
        kind = value[..value.len() - 1].to_string();
    }

    for (copper, rust) in COPPER_TYPES.iter() {
        if kind == *copper {
            kind = rust.to_string();
            break;
        }
    }

    if is_optional {
        kind = format!("Option<{}>", kind);
    }

    kind
}

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