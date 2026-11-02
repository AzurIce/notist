//! Checked expressions and pending computations carried by content Items.
//! These are pure IR data; checking and interpretation belong to the pipeline.

use crate::{
    definitions::{ComptimeValue, FunctionDef, FunctionInstance, Type},
    item::{Dict, Item},
};
use rowan::TextRange;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TypedExpr {
    pub ty: Option<Type>,
    pub known: Option<ComptimeValue>,
    pub kind: ExprKind,
    #[cfg_attr(feature = "serde", serde(with = "span"))]
    pub span: TextRange,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ExprKind {
    Constant(ComptimeValue),
    Input(String),
    Array(Vec<TypedExpr>),
    Dict(Vec<(String, TypedExpr)>),
    Unary(String, Box<TypedExpr>),
    Binary(String, Box<TypedExpr>, Box<TypedExpr>),
    Block(Vec<(String, TypedExpr)>, Box<TypedExpr>),
    If(Box<TypedExpr>, Box<TypedExpr>, Box<TypedExpr>),
    /// The `some` constructor: wraps its data payload as Value::Some.
    Some(Box<TypedExpr>),
    /// Payload of a matched Value::Some; evaluation fails on none.
    Unwrap(Box<TypedExpr>),
    Ok(Box<TypedExpr>),
    Err(Box<TypedExpr>),
    /// Evaluate the subject once, then bind only the selected variant payload.
    MatchResult {
        subject: Box<TypedExpr>,
        arms: Vec<ResultArm>,
    },
    ValueCall(Box<Invocation>),
    Content(Vec<Item>),
    Failure(String),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResultArm {
    pub pattern: ResultPattern,
    pub body: TypedExpr,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ResultPattern {
    Ok(String),
    Err(String),
    Wildcard,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Invocation {
    pub instance: FunctionInstance,
    pub definition: Box<FunctionDef>,
    pub arguments: Vec<(String, TypedExpr)>,
    /// Value-call children. Content-call children live on the enclosing Item.
    pub children: Option<Box<TypedExpr>>,
    pub attrs: Dict,
    #[cfg_attr(feature = "serde", serde(with = "span"))]
    pub span: TextRange,
    pub source: String,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "CallSite::is_markup")
    )]
    pub site: CallSite,
    /// Captured only for resource-target inputs, including deferred URL strings.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub resource_root: Option<String>,
    pub invalid: Option<String>,
}

/// Lexical call context, independent of file names, aliases or module labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CallSite {
    #[default]
    Markup,
    Code,
}
impl CallSite {
    pub fn is_markup(&self) -> bool {
        *self == Self::Markup
    }
}

/// A checked content call or forest interpolation awaiting evaluation.
/// Ordinary structural Items have no pending computation.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PendingEvaluation {
    Call(Invocation),
    Splice(TypedExpr),
    Failure { ty: Option<Type>, message: String },
}

#[cfg(feature = "serde")]
pub(crate) mod span {
    use rowan::TextRange;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(range: &TextRange, serializer: S) -> Result<S::Ok, S::Error> {
        (u32::from(range.start()), u32::from(range.end())).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<TextRange, D::Error> {
        let (start, end) = <(u32, u32)>::deserialize(deserializer)?;
        if start > end {
            return Err(serde::de::Error::custom("reversed source range"));
        }
        Ok(TextRange::new(start.into(), end.into()))
    }
}
