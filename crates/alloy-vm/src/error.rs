//! Erros e fluxo de controle não-local da VM.

use crate::value::Value;
use copper_syntax::ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeError {
    pub message: String,
    pub span: Span,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

/// Resultado de avaliar um statement: ou seguiu normalmente, ou disparou
/// controle de fluxo não-local (return/break/continue), ou erro.
#[derive(Debug, Clone, PartialEq)]
pub enum Flow {
    /// Seguiu normal.
    Normal,
    Return(Value),
    Break,
    Continue,
    Err(RuntimeError),
}
