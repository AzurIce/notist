//! Shared checked expression IR and specialization contracts.
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
    pub sources: crate::source::Sources,
}

impl PartialEq for CheckedProgram {
    fn eq(&self, other: &Self) -> bool {
        self.instances == other.instances
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedExpr {
    pub ty: Option<Type>,
    pub known: Option<ComptimeValue>,
    pub kind: ExprKind,
    pub span: TextRange,
    pub source: crate::source::SourceId,
    /// Local checked forest evidence, not a public function return contract.
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
            shape.join(match item.ctor() {
                Ctor::TableRow => Self::Rows,
                Ctor::TableCell => Self::Cells,
                Ctor::ListItem => Self::Items,
                Ctor::Node | Ctor::Group => Self::of_items(item.children()),
                Ctor::Interpolation => item.as_expr().map(|expr| expr.shape).unwrap_or(Self::Other),
                _ => Self::Other,
            })
        })
    }
    pub fn of_expr(kind: &ExprKind) -> Self {
        match kind {
            ExprKind::Call(call)
                if call.definition.implementation
                    == crate::definitions::FunctionImpl::Native(
                        crate::definitions::NativeFunction::Construct,
                    )
                    && call.definition.id.module == "notist" =>
            {
                match call.definition.id.name.as_str() {
                    "row" => Self::Rows,
                    "cell" => Self::Cells,
                    "item" => Self::Items,
                    "node" => call
                        .children
                        .as_ref()
                        .map(|e| e.shape)
                        .unwrap_or(Self::Empty),
                    _ => Self::Other,
                }
            }
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
    /// Function application, independent of whether its result is data or content.
    Call(crate::shared::Shared<Invocation>),
    Content(crate::shared::Shared<Vec<Item>>),
    For {
        binding: String,
        subject: crate::shared::Shared<TypedExpr>,
        body: crate::shared::Shared<TypedExpr>,
    },
    Failure(String),
}

#[derive(Debug, Clone, PartialEq)]
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
pub struct Invocation {
    pub instance: FunctionInstance,
    /// The call's contract snapshot. Captures share it; editing one invocation
    /// detaches its copy instead of changing another call's contract.
    pub definition: crate::shared::Shared<FunctionDef>,
    pub arguments: Vec<(String, TypedExpr)>,
    /// Checked children expression, for both data and content calls.
    pub children: Option<Box<TypedExpr>>,
    pub attrs: Dict,
    pub span: TextRange,
    pub source: crate::source::SourceId,
    pub site: CallSite,
    /// Captured only for resource-target inputs, including deferred URL strings.
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
