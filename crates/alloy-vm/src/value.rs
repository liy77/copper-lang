//! Runtime values of the Alloy VM.

use crate::env::Env;
use copper_syntax::expr::Expr;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

/// A closure: parameters, body and the captured environment.
#[derive(Debug)]
pub struct ClosureData {
    pub params: Vec<String>,
    pub body: Expr,
    pub env: Rc<RefCell<Env>>,
}

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
    /// Vec/array — mutable and shareable.
    Vec(Rc<RefCell<Vec<Value>>>),
    /// Tuple `(a, b, ...)`.
    Tuple(Vec<Value>),
    /// Struct/class instance: `name` + fields.
    Struct {
        name: String,
        fields: Rc<RefCell<HashMap<String, Value>>>,
    },
    /// Enum, including `Option` (Some/None) and `Result` (Ok/Err):
    /// `ty="Option"`, `variant="Some"`, `payload=[v]`.
    Enum {
        ty: String,
        variant: String,
        payload: Vec<Value>,
    },
    /// `|x| body` — with captured environment.
    Closure(Rc<ClosureData>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (self, other) {
            (Int(a), Int(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Bool(a), Bool(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Unit, Unit) => true,
            (Vec(a), Vec(b)) => *a.borrow() == *b.borrow(),
            (Tuple(a), Tuple(b)) => a == b,
            (
                Struct {
                    name: n1,
                    fields: f1,
                },
                Struct {
                    name: n2,
                    fields: f2,
                },
            ) => n1 == n2 && *f1.borrow() == *f2.borrow(),
            (
                Enum {
                    ty: t1,
                    variant: v1,
                    payload: p1,
                },
                Enum {
                    ty: t2,
                    variant: v2,
                    payload: p2,
                },
            ) => t1 == t2 && v1 == v2 && p1 == p2,
            // Closures are never equal.
            _ => false,
        }
    }
}

impl Value {
    /// Type name for error messages.
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
            Value::Closure(_) => "closure".into(),
        }
    }

    /// Truthiness of a value in boolean context (only `Bool` is valid).
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Construction helpers for `Option`/`Result` (used by stdlib and
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
            Value::Closure(_) => write!(f, "<closure>"),
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
