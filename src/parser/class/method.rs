use super::generic::Generic;

#[derive(Debug)]
pub struct Method {
    pub name: String,
    pub(crate) return_type: String,
    pub(crate) is_private: bool,
    pub(crate) is_const: bool,
    pub(crate) generics: Vec<Generic>,
    pub(crate) args: Vec<(String, String)>,
    pub(crate) body: String,
}

impl Method {
    pub fn is_abstract(&self) -> bool {
        self.body.is_empty()
    }
}