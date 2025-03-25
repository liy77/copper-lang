#[derive(Debug)]
pub enum Generic {
    Type(String),
    Lifetime(String),
}

impl ToString for Generic {
    fn to_string(&self) -> String {
        match self {
            Generic::Type(t) => t.clone(),
            Generic::Lifetime(l) => l.clone(),
        }
    }
}