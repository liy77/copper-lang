use super::scope::{Parent, Scope};

pub struct Scopes {
    scopes: Vec<Scope>,
    ends: Vec<Scope>,
    current: usize,
}

impl Scopes {
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::new(Parent::Global)],
            ends: vec![],
            current: 0,
        }
    }

    pub fn add_scope(&mut self, parent: Parent) {
        self.scopes.push(Scope::new(parent));
        self.current += 1;
    }

    pub fn end_scope(&mut self) {
        self.ends.push(self.scopes.pop().unwrap());
        self.current -= 1;
    }

    pub fn get_current_scope(&self) -> &Scope {
        self.scopes.get(self.current).unwrap()
    }

    pub fn get_current_scope_mut(&mut self) -> &mut Scope {
        self.scopes.get_mut(self.current).unwrap()
    }

    pub fn get_current_parent(&self) -> &Parent {
        self.scopes.get(self.current).unwrap().get_parent()
    }

    pub fn get_current_vars(&self) -> &Vec<String> {
        self.scopes.get(self.current).unwrap().get_vars()
    }

    pub fn get_current_moved_vars(&self) -> &Vec<(String, Parent)> {
        self.scopes.get(self.current).unwrap().get_moved_vars()
    }

    pub fn get_ends(&self) -> &Vec<Scope> {
        &self.ends
    }
}