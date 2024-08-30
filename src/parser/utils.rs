const COPPER_TYPES : [(&str, &str); 15] = [
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