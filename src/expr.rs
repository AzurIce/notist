use rowan::TextRange;

use crate::item::{Dict, Value};

/// Unevaluated core expression: the desugar target of markup, and (later)
/// the parse product of code mode. Unlike `Item`, a call's name is an
/// unresolved source-level name.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value, TextRange),
    Call {
        name: String,
        args: Vec<Expr>,
        fields: Dict,
        children: Vec<Expr>,
        attrs: Dict,
        span: TextRange,
    },
}

impl Expr {
    pub fn call(name: &str, span: TextRange) -> Self {
        Expr::Call {
            name: name.to_string(),
            args: Vec::new(),
            fields: Dict::default(),
            children: Vec::new(),
            attrs: Dict::default(),
            span,
        }
    }

    pub fn text(text: String, span: TextRange) -> Self {
        Expr::call("text", span).with_field("text", Value::Str(text))
    }

    pub fn with_field(mut self, key: impl Into<String>, value: Value) -> Self {
        if let Expr::Call { fields, .. } = &mut self {
            fields.insert(key, value);
        }
        self
    }

    pub fn with_children(mut self, children: Vec<Expr>) -> Self {
        if let Expr::Call { children: c, .. } = &mut self {
            *c = children;
        }
        self
    }

    pub fn span(&self) -> TextRange {
        match self {
            Expr::Literal(_, span) | Expr::Call { span, .. } => *span,
        }
    }

    pub fn set_attrs(&mut self, new_attrs: Dict) {
        if let Expr::Call { attrs, .. } = self {
            *attrs = new_attrs;
        }
    }

    pub fn literal_value(&self) -> Option<Value> {
        if let Expr::Literal(value, _) = self {
            Some(value.clone())
        } else {
            None
        }
    }
}
