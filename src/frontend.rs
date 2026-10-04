use std::path::Path;

use rowan::TextRange;

use crate::diag::Diagnostic;
use crate::expr::Expr;
use crate::item::{Dict, Item};

/// A frontend: file extensions it handles and the lowering function that
/// turns sources into the shared Expr IR. Eval is shared and runs in the
/// pipeline, not in the frontend.
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
/// register more with [`Self::with`]. Most callers can use [`crate::Notist`].
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

    /// Lower `src` with the frontend matching `path`'s extension, then run
    /// the shared backend (resolve → shape → materialize).
    pub fn analyze(&self, path: &Path, src: &str) -> Option<(Item, Vec<Diagnostic>)> {
        let ext = path.extension()?.to_str()?;
        let frontend = self
            .frontends
            .iter()
            .rev()
            .find(|frontend| frontend.extensions.contains(&ext))?;
        let (forest, module_attrs, mut diagnostics) = (frontend.lower)(src);
        let span = TextRange::new(0.into(), (src.len() as u32).into());
        let item = notist_core::analyze(forest, span, module_attrs, &mut diagnostics);
        Some((item, diagnostics))
    }
}

impl Default for Frontends {
    fn default() -> Self {
        Self::with_defaults()
    }
}
