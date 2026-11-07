//! Checked expressions and pending computations carried by content Items.
//! These are pure IR data; checking and interpretation belong to the pipeline.

use crate::{
    definitions::{ComptimeValue, FunctionDef, FunctionInstance, Type},
    item::{Dict, Item},
};
use rowan::TextRange;
use std::{collections::BTreeMap, sync::Arc};

/// One checked specialization shared by analysis and evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceRecord {
    pub definition: FunctionDef,
    pub body: Option<TypedExpr>,
    pub diagnostics: Vec<crate::diag::Diagnostic>,
    pub checking: bool,
}

/// Checked instances and retained data backing evidence, independent of the
/// compiler and evaluator implementations. Inserting instances uses COW.
#[derive(Debug, Clone, Default)]
pub struct CheckedProgram {
    pub instances: Arc<BTreeMap<String, Arc<InstanceRecord>>>,
    pub data_retention: crate::data::DataRetention,
}

impl PartialEq for CheckedProgram {
    fn eq(&self, other: &Self) -> bool {
        self.instances == other.instances
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TypedExpr {
    pub ty: Option<Type>,
    pub known: Option<ComptimeValue>,
    pub kind: ExprKind,
    #[cfg_attr(feature = "serde", serde(with = "span"))]
    pub span: TextRange,
    pub source: String,
    /// Local checked forest evidence, not a public function return contract.
    #[cfg_attr(feature = "serde", serde(default))]
    pub shape: ContentShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ContentShape {
    Empty,
    Rows,
    Cells,
    Items,
    #[default]
    Other,
}
impl ContentShape {
    pub fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Empty, s) | (s, Self::Empty) => s,
            (a, b) if a == b => a,
            _ => Self::Other,
        }
    }
    pub fn of_items(items: &[Item]) -> Self {
        use crate::item::Ctor;
        items.iter().fold(Self::Empty, |shape, item| {
            shape.join(match item.ctor {
                Ctor::TableRow => Self::Rows,
                Ctor::TableCell => Self::Cells,
                Ctor::ListItem => Self::Items,
                Ctor::Node => Self::of_items(&item.children),
                Ctor::Interpolation => match item.pending.as_deref() {
                    Some(PendingEvaluation::Splice(expr) | PendingEvaluation::Repeat(expr)) => {
                        expr.shape
                    }
                    _ => Self::Other,
                },
                _ => Self::Other,
            })
        })
    }
    pub fn of_expr(kind: &ExprKind) -> Self {
        match kind {
            ExprKind::Content(items) => Self::of_items(items),
            ExprKind::Block(_, value) => value.shape,
            ExprKind::For { body, .. } => body.shape,
            ExprKind::If(_, yes, no) => {
                if yes.ty == Some(Type::Never) {
                    no.shape
                } else if no.ty == Some(Type::Never) {
                    yes.shape
                } else {
                    yes.shape.join(no.shape)
                }
            }
            ExprKind::MatchResult { arms, .. } => arms
                .iter()
                .filter(|arm| arm.body.ty != Some(Type::Never))
                .fold(Self::Empty, |shape, arm| shape.join(arm.body.shape)),
            _ => Self::Other,
        }
    }
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
    For {
        binding: String,
        subject: Box<TypedExpr>,
        body: Box<TypedExpr>,
    },
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
    /// The call's contract snapshot. Captures share it; editing one invocation
    /// detaches its copy instead of changing another call's contract.
    pub definition: crate::shared::Shared<FunctionDef>,
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
    /// Captured content traversal, interpreted only when the forest is mounted.
    Repeat(TypedExpr),
    Failure {
        ty: Option<Type>,
        message: String,
    },
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
