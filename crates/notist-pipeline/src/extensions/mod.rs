//! Experimental ordered document extension execution and stage validation.
mod replacements;
use notist_core::{
    builtins::Level,
    definitions::FunctionId,
    diag::{Diagnostic, Phase},
    extensions::{
        DocumentContext, DocumentExtension, ExtensionResult, LoweredDocument, Replacement,
    },
    item::{Ctor, EvaluationState, Item},
    registry::Registry,
};
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct ExtensionPlan {
    entries: Vec<Arc<dyn DocumentExtension>>,
    replacements: Vec<Replacement>,
}
impl ExtensionPlan {
    pub fn with_extension(
        mut self,
        extension: Arc<dyn DocumentExtension>,
        registry: &Registry,
    ) -> Result<Self, Vec<Diagnostic>> {
        extension
            .validate(registry)
            .and_then(|()| replacements::validate(extension.replacements(), registry))
            .map_err(|errors| describe(errors, extension.as_ref(), "registration"))?;
        self.replacements
            .extend_from_slice(extension.replacements());
        self.entries.push(extension);
        Ok(self)
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Each rule selects its target once. Mappings never affect comptime checks.
    pub fn target(&self, source: &FunctionId) -> FunctionId {
        let mut id = source.clone();
        for rule in &self.replacements {
            if id == rule.from {
                id = rule.to.clone();
            }
        }
        id
    }

    pub(crate) fn before_check(
        &self,
        context: &DocumentContext<'_>,
        document: &mut LoweredDocument,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> bool {
        for extension in &self.entries {
            let mut candidate = document.clone();
            let result = extension
                .before_check(context, &mut candidate)
                .and_then(|()| validate_lowered(context, &mut candidate));
            if !report(
                result,
                extension.as_ref(),
                "before_check",
                context,
                diagnostics,
            ) {
                return false;
            }
            *document = candidate;
        }
        true
    }
    pub(crate) fn after_check(
        &self,
        context: &DocumentContext<'_>,
        document: &Item,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> bool {
        for extension in &self.entries {
            if !report(
                extension.after_check(context, document),
                extension.as_ref(),
                "after_check",
                context,
                diagnostics,
            ) {
                return false;
            }
        }
        true
    }
    pub(crate) fn after_evaluate(
        &self,
        context: &DocumentContext<'_>,
        document: &mut Item,
        diagnostics: &mut Vec<Diagnostic>,
        limit: usize,
    ) {
        for extension in &self.entries {
            let mut candidate = document.clone();
            let result = extension
                .after_evaluate(context, &mut candidate)
                .and_then(|()| {
                    if candidate == *document {
                        Ok(())
                    } else {
                        validate_output(&candidate, limit)
                    }
                });
            if !report(
                result,
                extension.as_ref(),
                "after_evaluate",
                context,
                diagnostics,
            ) {
                return;
            }
            *document = candidate;
        }
    }
}

fn report(
    result: ExtensionResult,
    extension: &dyn DocumentExtension,
    stage: &str,
    context: &DocumentContext<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    let Err(errors) = result else {
        return true;
    };
    for mut error in describe(errors, extension, stage) {
        error.source.get_or_insert_with(|| context.path.into());
        error.recovered = false;
        diagnostics.push(error);
    }
    false
}

fn describe(
    mut errors: Vec<Diagnostic>,
    extension: &dyn DocumentExtension,
    stage: &str,
) -> Vec<Diagnostic> {
    if errors.is_empty() {
        errors.push(Diagnostic::new(
            Phase::Semantic,
            Default::default(),
            "extension failed",
        ));
    }
    for error in &mut errors {
        error.message = format!(
            "extension `{}` ({stage}): {}",
            extension.name(),
            error.message
        );
        error.recovered = false;
    }
    errors
}

fn validate_lowered(
    context: &DocumentContext<'_>,
    document: &mut LoweredDocument,
) -> ExtensionResult {
    let source = context
        .sources
        .get(context.path)
        .expect("document source exists");
    let mut invalid = false;
    let mut check = |span: rowan::TextRange| {
        invalid |= !source.is_char_boundary(u32::from(span.start()) as usize)
            || !source.is_char_boundary(u32::from(span.end()) as usize);
        span
    };
    for expr in &mut document.forest {
        expr.map_spans(&mut check);
    }
    for annotation in &mut document.annotations {
        annotation.map_spans(&mut check);
    }
    if invalid {
        Err(vec![Diagnostic::new(
            Phase::Semantic,
            Default::default(),
            "generated IR contains a source range outside this document's UTF-8 boundaries",
        )])
    } else {
        Ok(())
    }
}

fn validate_output(root: &Item, limit: usize) -> ExtensionResult {
    let problem = if root.ctor != Ctor::Doc || root.level != Level::Block {
        Some("output root must remain a block document")
    } else if root
        .descendants()
        .skip(1)
        .any(|node| node.ctor == Ctor::Doc)
    {
        Some("output cannot contain nested document roots")
    } else if root.descendants().any(|node| {
        node.pending.is_some()
            || (node.ctor == Ctor::Interpolation
                && !matches!(node.evaluation, Some(EvaluationState::Error(_))))
    }) {
        Some("output contains unevaluated content; insert expressions in before_check instead")
    } else if root
        .descendants()
        .filter(|node| !matches!(node.evaluation, Some(EvaluationState::Error(_))))
        .count()
        > limit
    {
        Some("output exceeds the evaluation content size limit")
    } else {
        None
    };
    if let Some(message) = problem {
        return Err(vec![Diagnostic::new(Phase::Semantic, root.span, message)]);
    }
    let mut errors = vec![];
    notist_core::index::Index::build(root, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
