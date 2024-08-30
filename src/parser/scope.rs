pub enum Parent {
    Struct(String),
    Class(String),
    Method(String),
    Function(String),
    Block,
    Global,
}

pub struct Scope {
    vars: Vec<String>,
    moved_vars: Vec<(String, Parent)>,
    parent: Parent,
}

impl Scope {
    pub fn new(parent: Parent) -> Self {
        Self {
            vars: vec![],
            moved_vars: vec![],
            parent,
        }
    }

    pub fn add_var(&mut self, var: String) {
        self.vars.push(var);
    }

    pub fn add_moved_var(&mut self, var: String, parent: Parent) {
        self.moved_vars.push((var, parent));
    }

    pub fn get_vars(&self) -> &Vec<String> {
        &self.vars
    }

    pub fn get_moved_vars(&self) -> &Vec<(String, Parent)> {
        &self.moved_vars
    }

    pub fn get_parent(&self) -> &Parent {
        &self.parent
    }
}