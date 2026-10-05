use std::path::Path;

use rowan::TextRange;

use crate::NotistFrontend;
use notist_core::diag::Diagnostic;
use notist_core::frontend::{Frontend, FrontendOptions};
use notist_core::item::Item;

/// A frontend registry. [`Self::with_defaults`] installs `.not` and Markdown;
/// register more with [`Self::with`]. Most callers can use [`crate::Pipeline`].
pub struct Frontends {
    frontends: Vec<Box<dyn Frontend>>,
}

impl Frontends {
    pub fn new() -> Self {
        Self {
            frontends: Vec::new(),
        }
    }

    /// `.not` (notist-syntax + notist-lowering) and Markdown / Notist Markdown (rushdown).
    pub fn with_defaults() -> Self {
        Self::new()
            .with(NotistFrontend)
            .with(notist_md::MarkdownFrontend)
    }

    /// Register a frontend; the last registration wins for overlapping extensions.
    pub fn with(mut self, frontend: impl Frontend + 'static) -> Self {
        self.frontends.push(Box::new(frontend));
        self
    }

    pub fn supports(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                self.frontends
                    .iter()
                    .any(|frontend| frontend.extensions().contains(&ext))
            })
    }

    pub(crate) fn get(&self, path: &Path) -> Option<&dyn Frontend> {
        let ext = path.extension()?.to_str()?;
        self.frontends
            .iter()
            .rev()
            .find(|frontend| frontend.extensions().contains(&ext))
            .map(|frontend| frontend.as_ref())
    }

    /// Lower `src` with the frontend matching `path`'s extension, then run
    /// the shared backend (resolve → shape → materialize).
    pub fn analyze(&self, path: &Path, src: &str) -> Option<(Item, Vec<Diagnostic>)> {
        self.analyze_with_registry(path, src, notist_core::builtins::registry())
    }

    pub fn analyze_with_registry(
        &self,
        path: &Path,
        src: &str,
        registry: &notist_core::registry::Registry,
    ) -> Option<(Item, Vec<Diagnostic>)> {
        let frontend = self.get(path)?;
        let output = frontend.compile(src, FrontendOptions::default());
        let mut diagnostics = output.diagnostics;
        let span = TextRange::new(0.into(), (src.len() as u32).into());
        let item = crate::process(
            output.forest,
            span,
            output.module_attrs,
            registry,
            &mut diagnostics,
        );
        Some((item, diagnostics))
    }
}

impl Default for Frontends {
    fn default() -> Self {
        Self::with_defaults()
    }
}
