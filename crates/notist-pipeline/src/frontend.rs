use std::path::Path;

use rowan::TextRange;

use crate::diag::Diagnostic;
use crate::expr::Expr;
use crate::item::{Dict, Item};

/// A frontend: file extensions it handles and the lowering function that
/// turns sources into shared Expr IR. Resolution, shaping and materialization
/// run in the pipeline.
pub struct Frontend {
    pub extensions: &'static [&'static str],
    pub lower: fn(&str) -> (Vec<Expr>, Dict, Vec<Diagnostic>),
}

impl Frontend {
    /// The `.not` frontend.
    pub fn notist() -> Self {
        Self {
            extensions: &["not"],
            lower: crate::desugar::lower_not,
        }
    }

    /// The `.md` and `.markdown` frontend.
    pub fn markdown() -> Self {
        Self {
            extensions: &["md", "markdown"],
            lower: notist_md::lower,
        }
    }
}

/// A frontend registry. [`Self::with_defaults`] installs `.not` and Markdown;
/// register more with [`Self::with`]. Most callers can use [`crate::Pipeline`].
pub struct Frontends {
    frontends: Vec<Frontend>,
}

impl Frontends {
    pub fn new() -> Self {
        Self {
            frontends: Vec::new(),
        }
    }

    /// `.not` (notist-syntax + desugar) and `.md`/`.markdown` (rushdown).
    pub fn with_defaults() -> Self {
        Self::new()
            .with(Frontend::notist())
            .with(Frontend::markdown())
    }

    /// Register a frontend; the last registration wins for overlapping extensions.
    pub fn with(mut self, frontend: Frontend) -> Self {
        self.frontends.push(frontend);
        self
    }

    pub fn supports(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                self.frontends
                    .iter()
                    .any(|frontend| frontend.extensions.contains(&ext))
            })
    }

    pub(crate) fn get(&self, path: &Path) -> Option<&Frontend> {
        let ext = path.extension()?.to_str()?;
        self.frontends
            .iter()
            .rev()
            .find(|frontend| frontend.extensions.contains(&ext))
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
        let (forest, module_attrs, mut diagnostics) = (frontend.lower)(src);
        let span = TextRange::new(0.into(), (src.len() as u32).into());
        let item = crate::process(forest, span, module_attrs, registry, &mut diagnostics);
        Some((item, diagnostics))
    }
}

impl Default for Frontends {
    fn default() -> Self {
        Self::with_defaults()
    }
}
