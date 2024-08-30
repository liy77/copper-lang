use super::kind::TokenKind;

#[derive(Debug, PartialEq, Clone)]
pub enum Data {
    None,
    String(String),
}

#[derive(Debug, PartialEq, Clone)]
pub struct LocationData {
    pub(crate) first_line: isize,
    pub(crate) first_column: usize,
    pub(crate) last_line: isize,
    pub(crate) last_column: usize,
    pub(crate) range: (usize, usize)
}

#[derive(Debug, PartialEq, Clone)]
pub struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) value: String,
    pub(crate) length: usize,
    pub(crate) data: Data,
    pub (crate) generated: bool,
    pub(crate) origin: Option<Box<Token>>,
    pub(crate) location_data: Option<LocationData>,
}

impl Token {
    pub fn new(kind: TokenKind, value: String, length: usize, data: Data, generated: bool) -> Self {
        Self {
            kind,
            value,
            length,
            data,
            generated,
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