//! Document extension contracts. Stage inputs are explicit and contain no host IO.
use crate::{
    annotation::Annotation,
    definitions::FunctionId,
    diag::{Diagnostic, Sources},
    expr::Expr,
    item::{Dict, Item},
    registry::Registry,
};
use rowan::TextRange;
use std::fmt::Debug;

pub type ExtensionResult = Result<(), Vec<Diagnostic>>;

/// Immutable context for one document, shared by all its extension stages.
pub struct DocumentContext<'a> {
    pub path: &'a str,
    pub sources: &'a Sources,
    pub registry: &'a Registry,
}

/// IR₁ before name resolution and checking. Static imports have already been
/// loaded by the host and cannot be edited here. New names use that same scope.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct LoweredDocument {
    pub forest: Vec<Expr>,
    pub annotations: Vec<Annotation>,
    pub attrs: Dict,
}

/// One same-signature implementation selection, applied only during evaluation.
#[derive(Debug, Clone)]
pub struct Replacement {
    pub from: FunctionId,
    pub to: FunctionId,
    pub span: TextRange,
}

/// Extensions execute once per stage, in registration order. Mutating stages
/// commit only on success; failure preserves the previous stage value and stops
/// subsequent hooks for this document. A checked tree is read-only because it carries checked
/// instance identities. Output edits do not re-enter checking or evaluation.
/// Registered execution mappings remain part of the Context during recovery.
pub trait DocumentExtension: Debug + Send + Sync {
    fn name(&self) -> &str;

    /// Validate extension configuration after all module signatures are loaded.
    fn validate(&self, _registry: &Registry) -> ExtensionResult {
        Ok(())
    }

    /// Execution mappings participate in the same registration order as hooks.
    /// The pipeline validates their complete signatures and structural roles.
    fn replacements(&self) -> &[Replacement] {
        &[]
    }

    fn before_check(
        &self,
        _context: &DocumentContext<'_>,
        _document: &mut LoweredDocument,
    ) -> ExtensionResult {
        Ok(())
    }

    fn after_check(&self, _context: &DocumentContext<'_>, _document: &Item) -> ExtensionResult {
        Ok(())
    }

    /// Modify a complete output tree. Inserted content must already be concrete;
    /// pending calls and nested document roots are rejected by the pipeline.
    fn after_evaluate(
        &self,
        _context: &DocumentContext<'_>,
        _document: &mut Item,
    ) -> ExtensionResult {
        Ok(())
    }
}

/// Built-in document extension selecting same-signature execution targets.
#[derive(Debug, Clone)]
pub struct Replace {
    rules: Vec<Replacement>,
}
impl Replace {
    pub fn new(rules: impl IntoIterator<Item = Replacement>) -> Self {
        Self {
            rules: rules.into_iter().collect(),
        }
    }
}
impl DocumentExtension for Replace {
    fn name(&self) -> &str {
        "replace"
    }
    fn replacements(&self) -> &[Replacement] {
        &self.rules
    }
}
