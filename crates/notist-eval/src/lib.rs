//! Bounded evaluation of checked Notist expressions and content trees.
//!
//! This crate has no parser, module loader, filesystem or output backend.
//! A compiler supplies checked IR and implements [`EvaluationContext`] for
//! execution mappings and any specializations discovered during evaluation.
//! Source merging and document extension hooks remain the pipeline's job.
mod eval;
mod instance_cache;

pub use eval::{
    Evaluation, EvaluationOutcome, Failure, Limits, binary, constant, evaluate, run, unary,
};

use notist_core::{
    definitions::{FunctionId, FunctionInstance},
    diag::Diagnostic,
    evaluation::CheckedProgram,
    registry::Registry,
};

/// Immutable execution definitions plus a compiler callback for missing
/// specializations. The evaluator owns the returned program for this run;
/// implementations must retain the supplied program's instances and data
/// retention evidence, and return diagnostics in checking order.
///
/// Evaluation uses a generic context, so normal dispatch does not introduce
/// trait-object calls on the per-expression/per-constructor hot paths.
pub trait EvaluationContext {
    fn registry(&self) -> &Registry;
    fn resource_root(&self) -> &str;
    fn target(&self, id: &FunctionId) -> FunctionId;
    fn specialize(
        &self,
        instance: &FunctionInstance,
        program: CheckedProgram,
    ) -> (CheckedProgram, Vec<Diagnostic>);
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use eval::test_support;
