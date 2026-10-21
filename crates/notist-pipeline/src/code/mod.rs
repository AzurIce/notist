//! Concrete-instance checking and a bounded pure interpreter over Item trees.
mod check;
mod content;
pub(crate) mod eval;
pub(crate) use check::Checker;
pub use eval::{Evaluation, Limits};
use notist_core::{
    definitions::{ComptimeValue, FunctionDef, FunctionInstance, Type},
    diag::Diagnostic,
    evaluation::{ExprKind, Invocation, PendingEvaluation, TypedExpr},
    expr::RExpr,
    item::{Dict, Item},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InstanceRecord {
    pub definition: FunctionDef,
    pub body: Option<TypedExpr>,
    pub diagnostics: Vec<Diagnostic>,
    pub checking: bool,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct CheckedProgram {
    pub instances: BTreeMap<String, InstanceRecord>,
}
