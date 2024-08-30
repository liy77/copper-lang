#[derive(Debug)]
pub struct Field {
    pub name: String,
    pub(crate) field_type: String,
    pub(crate) is_private: bool,
}