//! Valores em runtime da VM Alloy.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
    /// Vec/array — mutável e compartilhável.
    Vec(Rc<RefCell<Vec<Value>>>),
    /// Tupla `(a, b, ...)`.
    Tuple(Vec<Value>),
    /// Instância de struct/class: `name` + campos.
    Struct {
        name: String,
        fields: Rc<RefCell<HashMap<String, Value>>>,
    },
    /// Enum, incluindo `Option` (Some/None) e `Result` (Ok/Err):
    /// `ty="Option"`, `variant="Some"`, `payload=[v]`.
    Enum {
        ty: String,
        variant: String,
        payload: Vec<Value>,
    },
}

impl Value {
    /// Nome do tipo para mensagens de erro.
    pub fn type_name(&self) -> String {
        match self {
            Value::Int(_) => "int".into(),
            Value::Float(_) => "float".into(),
            Value::Bool(_) => "bool".into(),
            Value::Str(_) => "str".into(),
            Value::Unit => "unit".into(),
            Value::Vec(_) => "vec".into(),
            Value::Tuple(_) => "tuple".into(),
            Value::Struct { name, .. } => name.clone(),
            Value::Enum { ty, .. } => ty.clone(),
        }
    }

    /// Verdade de um valor em contexto booleano (só `Bool` é válido).
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Helpers de construção para `Option`/`Result` (usados pela stdlib e por
    /// `Some`/`None`/`Ok`/`Err`).
    pub fn some(v: Value) -> Value {
        Value::Enum {
            ty: "Option".into(),
            variant: "Some".into(),
            payload: vec![v],
        }
    }
    pub fn none() -> Value {
        Value::Enum {
            ty: "Option".into(),
            variant: "None".into(),
            payload: vec![],
        }
    }
    pub fn ok(v: Value) -> Value {
        Value::Enum {
            ty: "Result".into(),
            variant: "Ok".into(),
            payload: vec![v],
        }
    }
    pub fn err(v: Value) -> Value {
        Value::Enum {
            ty: "Result".into(),
            variant: "Err".into(),
            payload: vec![v],
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Unit => write!(f, "()"),
            Value::Vec(items) => {
                let inner = items
                    .borrow()
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "[{inner}]")
            }
            Value::Tuple(items) => {
                let inner = items
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "({inner})")
            }
            Value::Struct { name, fields } => {
                let inner = fields
                    .borrow()
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{name} {{ {inner} }}")
            }
            Value::Enum {
                variant, payload, ..
            } => {
                if payload.is_empty() {
                    write!(f, "{variant}")
                } else {
                    let inner = payload
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    write!(f, "{variant}({inner})")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_type_name() {
        assert_eq!(Value::Int(42).to_string(), "42");
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::Str("oi".into()).to_string(), "oi");
        assert_eq!(Value::Int(1).type_name(), "int");
        assert_eq!(Value::Bool(false).as_bool(), Some(false));
        assert_eq!(Value::Int(1).as_bool(), None);
    }

    #[test]
    fn composite_display() {
        let v = Value::Vec(Rc::new(RefCell::new(vec![Value::Int(1), Value::Int(2)])));
        assert_eq!(v.to_string(), "[1, 2]");
        assert_eq!(
            Value::Tuple(vec![Value::Int(1), Value::Int(2)]).to_string(),
            "(1, 2)"
        );
        assert_eq!(Value::none().to_string(), "None");
        assert_eq!(Value::some(Value::Int(7)).to_string(), "Some(7)");
        assert_eq!(Value::ok(Value::Int(1)).to_string(), "Ok(1)");
    }
}
