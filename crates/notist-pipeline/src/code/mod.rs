//! Concrete-instance checking and integration with the standalone evaluator.
mod check;
mod content;
pub(crate) mod literal;
mod result;
pub(crate) use check::Checker;
pub(crate) use notist_core::evaluation::{CheckedProgram, InstanceRecord};
use notist_core::{
    definitions::{ComptimeValue, FunctionDef, FunctionInstance, Type},
    diag::Diagnostic,
    evaluation::{ExprKind, Invocation, PendingEvaluation, TypedExpr},
    expr::RExpr,
    item::{Dict, Item},
};
pub use notist_eval::{Evaluation, EvaluationOutcome, Limits};
use std::collections::BTreeMap;

#[cfg(test)]
mod eval_tests;

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
