//! Source-positioned metadata carried alongside IR forests until shaping ends.

use rowan::TextRange;

use crate::expr::Expr;
use crate::item::Dict;

/// Literal-only metadata before Resolve. Entries retain source expressions and
/// duplicate keys; Resolve preserves Dict's first-position/last-value semantics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiteralDict(Vec<(String, Expr)>);

impl LiteralDict {
    pub fn insert(&mut self, key: impl Into<String>, value: crate::item::Value) {
        self.insert_expr(key, Expr::Literal(value, TextRange::default()));
    }
    pub fn insert_expr(&mut self, key: impl Into<String>, value: Expr) {
        self.0.push((key.into(), value));
    }
    pub fn get(&self, key: &str) -> Option<&Expr> {
        Some(&self.0.iter().rev().find(|(k, _)| k == key)?.1)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Expr)> {
        self.0.iter().map(|(key, value)| (key.as_str(), value))
    }
    pub fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn map_spans(&mut self, map: &mut impl FnMut(TextRange) -> TextRange) {
        for (_, value) in &mut self.0 {
            value.map_spans(map);
        }
    }
}

impl From<Dict> for LiteralDict {
    fn from(dict: Dict) -> Self {
        let mut result = Self::default();
        for (key, value) in dict.iter() {
            result.insert(key, value.clone());
        }
        result
    }
}

// Preserve the existing command-extension wire spelling for ordinary literal
// metadata. Only unresolved/new expressions need the explicit wrapper.
#[cfg(feature = "serde")]
mod literal_serde {
    use super::*;
    use crate::item::Value;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Value(Value),
        Expression { expression: Expr },
    }

    fn plain_value(expr: &Expr) -> Option<Value> {
        match expr {
            Expr::Literal(value, _) => Some(value.clone()),
            Expr::Array(items, _) => items
                .iter()
                .map(plain_value)
                .collect::<Option<crate::shared::Array>>()
                .map(Value::Array),
            Expr::Dict(fields, _) => fields
                .iter()
                .map(|(key, value)| Some((key.clone(), plain_value(value)?)))
                .collect::<Option<Dict>>()
                .map(Value::Dict),
            _ => None,
        }
    }

    impl Serialize for LiteralDict {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.0
                .iter()
                .map(|(key, expr)| {
                    let value = match plain_value(expr) {
                        Some(value) => Stored::Value(value),
                        None => Stored::Expression {
                            expression: expr.clone(),
                        },
                    };
                    (key, value)
                })
                .collect::<Vec<_>>()
                .serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for LiteralDict {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let fields = Vec::<(String, Stored)>::deserialize(deserializer)?;
            Ok(Self(
                fields
                    .into_iter()
                    .map(|(key, value)| {
                        (
                            key,
                            match value {
                                Stored::Value(value) => Expr::Literal(value, TextRange::default()),
                                Stored::Expression { expression } => expression,
                            },
                        )
                    })
                    .collect(),
            ))
        }
    }
}

/// Where an annotation sits relative to the syntax around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AnnotationKind {
    /// A standalone line; it targets the first block after its position.
    Block,
    /// Written inside an inline sequence; it targets the immediately
    /// following inline element.
    Inline,
}

/// A node annotation waiting to be attached after shaping.
///
/// Annotations are not content nodes: their source span and literal dict travel
/// beside the forest until Resolve creates a [`ResolvedAnnotation`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Annotation {
    pub dict: LiteralDict,
    #[cfg_attr(feature = "serde", serde(with = "crate::evaluation::span"))]
    pub span: TextRange,
    pub kind: AnnotationKind,
}

impl Annotation {
    pub fn map_spans(&mut self, map: &mut impl FnMut(TextRange) -> TextRange) {
        self.span = map(self.span);
        self.dict.map_spans(map);
    }
}

/// Metadata after literal Resolve, consumed without evaluation by Materialize.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAnnotation {
    pub dict: Dict,
    pub span: TextRange,
    pub kind: AnnotationKind,
}
