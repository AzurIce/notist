use rowan::TextRange;

use crate::item::{Ctor, Dict, Value};

/// How a call's `[...]` body was written: hugging the brackets (`[x]`,
/// inline) or padded on both ends (`[ x ]`, block). `None` marks
/// structurally constructed calls without a source body flavor; their
/// fields and content contracts are still checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFlavor {
    None,
    Inline,
    Block,
}

/// Unmaterialized document expression: the desugar target of markup.
/// Code declarations use DefinitionModule instead. `Expr<String>` (IR₁) carries unresolved
/// source-level names; `Expr<Ctor>` (IR₂) is the resolved form produced by
/// `resolve`, still carrying the declared body flavor for shaping.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr<N = String> {
    Literal(Value, TextRange),
    Call {
        name: N,
        args: Vec<Expr<N>>,
        fields: Dict,
        children: Vec<Expr<N>>,
        body: BodyFlavor,
        attrs: Dict,
        span: TextRange,
    },
}

/// IR₂: a resolved expression — names are core constructors.
pub type RExpr = Expr<Ctor>;

impl Expr<String> {
    pub fn call(name: &str, span: TextRange) -> Self {
        Expr::Call {
            name: name.to_string(),
            args: Vec::new(),
            fields: Dict::default(),
            children: Vec::new(),
            body: BodyFlavor::None,
            attrs: Dict::default(),
            span,
        }
    }

    pub fn text(text: String, span: TextRange) -> Self {
        Self::call("text", span).with_field("text", Value::Str(text))
    }
}

impl RExpr {
    pub fn resolved(ctor: Ctor, span: TextRange) -> Self {
        Expr::Call {
            name: ctor,
            args: Vec::new(),
            fields: Dict::default(),
            children: Vec::new(),
            body: BodyFlavor::None,
            attrs: Dict::default(),
            span,
        }
    }
}

impl<N> Expr<N> {
    pub fn with_field(mut self, key: impl Into<String>, value: Value) -> Self {
        if let Expr::Call { fields, .. } = &mut self {
            fields.insert(key, value);
        }
        self
    }

    pub fn with_children(mut self, children: Vec<Expr<N>>) -> Self {
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
