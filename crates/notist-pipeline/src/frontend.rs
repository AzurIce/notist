pub mod markdown;
pub mod markup;

use std::path::Path;

use rowan::TextRange;

use self::markup::NotistFrontend;
use crate::Context;
use crate::lowering::content::ContentLowering;
use notist_core::diag::Diagnostic;
use notist_core::frontend::{Frontend, FrontendOptions};
use notist_core::item::Item;

/// Combines source compilation and content-literal handling for registration.
pub trait PipelineFrontend: Frontend + ContentLowering {}
impl<T: Frontend + ContentLowering> PipelineFrontend for T {}

/// A frontend registry. [`Self::with_defaults`] installs `.not` and Markdown;
/// register more with [`Self::with`]. Most callers can use [`crate::Pipeline`].
pub struct Frontends {
    frontends: Vec<Box<dyn PipelineFrontend>>,
}

impl Frontends {
    pub fn new() -> Self {
        Self {
            frontends: Vec::new(),
        }
    }

    /// Native Markup and Markdown / Notist Markdown.
    pub fn with_defaults() -> Self {
        Self::new()
            .with(NotistFrontend)
            .with(markdown::MarkdownFrontend)
    }

    /// Register a frontend; the last registration wins for overlapping extensions.
    pub fn with(mut self, frontend: impl PipelineFrontend + 'static) -> Self {
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

    pub(crate) fn get(&self, path: &Path) -> Option<&dyn PipelineFrontend> {
        let ext = path.extension()?.to_str()?;
        self.frontends
            .iter()
            .rev()
            .find(|frontend| frontend.extensions().contains(&ext))
            .map(|frontend| frontend.as_ref())
    }

    /// Lower `src` with the frontend matching `path`'s extension, then run
    /// the shared backend (resolve → shape → materialize).
    pub fn analyze(
        &self,
        path: &Path,
        src: &str,
        context: &Context,
    ) -> Option<(Item, Vec<Diagnostic>)> {
        let frontend = self.get(path)?;
        let output = frontend.compile(src, FrontendOptions::default());
        let mut diagnostics = output.diagnostics;
        let span = TextRange::new(0.into(), (src.len() as u32).into());
        let item = crate::process(
            output.forest,
            output.annotations,
            span,
            output.module_attrs,
            context,
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
