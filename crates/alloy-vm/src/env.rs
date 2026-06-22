//! Variable environment: a chain of scopes with a shared parent.

use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Default)]
pub struct Env {
    vars: HashMap<String, Value>,
    parent: Option<Rc<RefCell<Env>>>,
}

impl Env {
    pub fn new() -> Rc<RefCell<Env>> {
        Rc::new(RefCell::new(Env::default()))
    }

    /// Creates a child scope that can see the parent.
    pub fn child(parent: &Rc<RefCell<Env>>) -> Rc<RefCell<Env>> {
        Rc::new(RefCell::new(Env {
            vars: HashMap::new(),
            parent: Some(Rc::clone(parent)),
        }))
    }

    pub fn define(&mut self, name: impl Into<String>, value: Value) {
        self.vars.insert(name.into(), value);
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.vars.get(name) {
            Some(v.clone())
        } else if let Some(p) = &self.parent {
            p.borrow().get(name)
        } else {
            None
        }
    }

    /// Updates an existing variable in the scope where it was defined.
    /// Returns `false` if the name does not exist in any scope.
    pub fn set(&mut self, name: &str, value: Value) -> bool {
        if self.vars.contains_key(name) {
            self.vars.insert(name.to_string(), value);
            true
        } else if let Some(p) = &self.parent {
            p.borrow_mut().set(name, value)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_sees_parent_and_set_updates_origin() {
        let root = Env::new();
        root.borrow_mut().define("x", Value::Int(1));

        let child = Env::child(&root);
        // child can see parent
        assert_eq!(child.borrow().get("x"), Some(Value::Int(1)));
        // set updates in the origin scope (the parent)
        assert!(child.borrow_mut().set("x", Value::Int(9)));
        assert_eq!(root.borrow().get("x"), Some(Value::Int(9)));
        // set on a nonexistent name fails
        assert!(!child.borrow_mut().set("y", Value::Int(0)));
    }
}
