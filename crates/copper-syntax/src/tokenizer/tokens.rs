use super::kind::TokenKind;

#[derive(Debug, PartialEq, Clone)]
pub enum Data {
    None,
    String(String),
    /// String literal that contained `$ident` or `${expr}` placeholders.
    /// `placeholder` is the format string with `{}` markers; `args` are the
    /// expressions, in order, that fill them. The token's `value` carries the
    /// fully wrapped `format!(...)` form for default emission, while these
    /// fields let the parser unwrap into raw macro-arg form when appropriate.
    Interpolation {
        placeholder: String,
        args: Vec<String>,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub struct LocationData {
    pub first_line: isize,
    pub first_column: usize,
    pub last_line: isize,
    pub last_column: usize,
    pub range: (usize, usize),
}

#[derive(Debug, PartialEq, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub value: String,
    pub length: usize,
    pub data: Data,
    pub generated: bool,
    /// For a `BraceStart`/`BraceEnd` token: `true` when this `{`/`}` opens or
    /// closes a **struct/enum literal** (`Point { x: 1 }`) rather than a code
    /// block. The tokenizer decides this (it has the surrounding context); the
    /// parser reads it to coerce string literals in struct fields (Bug B).
    pub struct_brace: bool,
    pub origin: Option<Box<Token>>,
    pub location_data: Option<LocationData>,
}

impl Token {
    pub fn new(kind: TokenKind, value: String, length: usize, data: Data, generated: bool) -> Self {
        Self {
            kind,
            value,
            length,
            data,
            generated,
            struct_brace: false,
            origin: None,
            location_data: None,
        }
    }

    pub fn set_origin(&mut self, origin: Token) -> &mut Self {
        self.origin = Some(Box::new(origin));
        self
    }

    pub fn set_location_data(&mut self, location_data: LocationData) -> &mut Self {
        self.location_data = Some(location_data);
        self
    }

    pub fn add_data(&mut self, data: Data) -> &mut Self {
        self.data = data;
        self
    }
}

impl ToString for Token {
    fn to_string(&self) -> String {
        format!("[{}, {}]", self.kind.to_string(), self.value)
    }
}
