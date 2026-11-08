//! Concrete-instance checking and integration with the standalone evaluator.
mod check;
mod content;
pub(crate) mod literal;
mod result;
pub(crate) use check::Checker;
pub(crate) use notist_core::evaluation::{CheckedProgram, InstanceRecord};
use notist_core::{
    definitions::{ComptimeValue, FunctionDef, ResolvedInstance as FunctionInstance, Type},
    diag::Diagnostic,
    evaluation::{ExprKind, Invocation, TypedExpr},
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
    fn cached_programs_do_not_retain_unowned_caller_binding_metadata() {
        use notist_core::{definitions::ResolvedInstance, source::Source};
        let pipeline = Pipeline::default();
        let module = pipeline
            .compile_module(
                "doc.not",
                "binding-cache",
                "binding-cache.notc",
                "fn show() -> InlineContent { [#text(text: \"cached\")] }",
            )
            .unwrap();
        let context = Context::default().with_module(module).unwrap();
        let caller = Source::metadata("binding-cache-caller.not").unwrap();
        let mut checker = super::Checker::new(&context, &caller);
        let local = checker.declare("caller_local", Default::default()).unwrap();
        let instance = ResolvedInstance {
            function: context.resolve("self.show").unwrap().id.def_id(),
            arguments: Vec::new(),
        };
        checker.install(&instance);
        assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        assert!(checker.program.sources.bindings().get(local).is_ok());
        let key = instance.key(context.registry().identities()).unwrap();
        let cached = context.checked_program(&key).unwrap();
        assert!(cached.sources.source(caller.id()).is_err());
        assert!(cached.sources.bindings().get(local).is_err());
        cached.sources.validate_program(&cached).unwrap();
    }

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
        let changed = context.with_scope("demo", std::iter::empty()).unwrap();
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
