//! Concrete-instance checking and a bounded pure interpreter over Item trees.
mod check;
mod content;
pub(crate) mod eval;
pub(crate) mod literal;
mod result;
pub(crate) use check::Checker;
pub use eval::{Evaluation, EvaluationOutcome, Limits};
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
#[derive(Debug, Clone, Default)]
pub(crate) struct CheckedProgram {
    pub instances: BTreeMap<String, InstanceRecord>,
    pub data_retention: notist_core::data::DataRetention,
}
impl PartialEq for CheckedProgram {
    fn eq(&self, other: &Self) -> bool {
        self.instances == other.instances
    }
}
