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

/// A frontend registry. `Frontend::with_defaults()` ships the `.not` and
/// `.md` frontends; register more with `with_frontend`.
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
            .with(Frontend {
                extensions: &["not"],
                lower: crate::desugar::lower_not,
            })
            .with(Frontend {
                extensions: &["md", "markdown"],
                lower: notist_md::lower,
            })
    }

    pub fn with(mut self, frontend: Frontend) -> Self {
        self.frontends.push(frontend);
        self
    }

    /// Lower `src` with the frontend matching `path`'s extension, then run
    /// the shared shape and eval.
    pub fn analyze(&self, path: &Path, src: &str) -> Option<(Item, Vec<Diagnostic>)> {
        let ext = path.extension()?.to_str()?;
        let frontend = self
            .frontends
            .iter()
            .find(|frontend| frontend.extensions.contains(&ext))?;
        let (forest, module_attrs, mut diagnostics) = (frontend.lower)(src);
        let forest = crate::shape::shape(forest);
        let span = TextRange::new(0.into(), (src.len() as u32).into());
        let item = crate::eval::eval_doc(&forest, span, module_attrs, &mut diagnostics);
        Some((item, diagnostics))
    }
}

impl Default for Frontends {
    fn default() -> Self {
        Self::with_defaults()
    }
}
