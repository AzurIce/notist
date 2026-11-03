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
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InstanceRecord {
    pub definition: FunctionDef,
    pub body: Option<TypedExpr>,
    pub diagnostics: Vec<Diagnostic>,
    pub checking: bool,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct CheckedProgram {
    pub instances: Arc<BTreeMap<String, Arc<InstanceRecord>>>,
    pub data_retention: notist_core::data::DataRetention,
}
impl PartialEq for CheckedProgram {
    fn eq(&self, other: &Self) -> bool {
        self.instances == other.instances
    }
}

#[cfg(test)]
mod tests {
    use crate::{Context, Pipeline};
    use std::sync::Arc;

    #[test]
    fn successful_empty_program_plans_share_records_but_scope_changes_detach() {
        let pipeline = Pipeline::default();
        let code = pipeline
            .compile_module(
                "content.not",
                "demo",
                "/demo.notc",
                "fn show() -> InlineContent { [#text(text: \"shared\")] }",
            )
            .unwrap();
        let context = Context::default().with_module(code).unwrap();
        let first = pipeline
            .analyze("first.not", "#self.show()", &context)
            .unwrap();
        let second = pipeline
            .analyze("second.not", "#self.show()", &context)
            .unwrap();
        assert!(first.diagnostics().is_empty());
        assert!(second.diagnostics().is_empty());
        assert!(Arc::ptr_eq(
            &first.program.instances,
            &second.program.instances
        ));
        let changed = context.with_scope("demo", std::iter::empty());
        let third = pipeline
            .analyze("third.not", "#self.show()", &changed)
            .unwrap();
        assert!(third.diagnostics().is_empty());
        assert!(!Arc::ptr_eq(
            &first.program.instances,
            &third.program.instances
        ));
        let snapshot = first.clone();
        assert!(pipeline.evaluate(&first, &changed).diagnostics.is_empty());
        assert_eq!(first, snapshot);
    }
}
