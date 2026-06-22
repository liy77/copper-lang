//! Ambiente de variáveis: cadeia de escopos com pai compartilhado.

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

    /// Cria um escopo filho que enxerga o pai.
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

    /// Atualiza uma variável existente no escopo onde ela foi definida.
    /// Retorna `false` se o nome não existe em nenhum escopo.
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
        // filho enxerga o pai
        assert_eq!(child.borrow().get("x"), Some(Value::Int(1)));
        // set atualiza no escopo de origem (o pai)
        assert!(child.borrow_mut().set("x", Value::Int(9)));
        assert_eq!(root.borrow().get("x"), Some(Value::Int(9)));
        // set em nome inexistente falha
        assert!(!child.borrow_mut().set("y", Value::Int(0)));
    }
}
