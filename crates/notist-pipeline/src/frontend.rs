pub mod markdown;
pub mod markup;

use std::path::Path;

use self::markup::NotistFrontend;
use crate::lowering::content::ContentLowering;
use notist_core::frontend::Frontend;

/// Combines source compilation and content-literal handling for registration.
pub trait PipelineFrontend: Frontend + ContentLowering {}
impl<T: Frontend + ContentLowering> PipelineFrontend for T {}

/// A frontend registry. [`Default`] installs `.not` and Markdown;
/// register more with [`Self::with`]. Most callers can use [`crate::Pipeline`].
pub(crate) struct Frontends {
    frontends: Vec<Box<dyn PipelineFrontend>>,
}

impl Frontends {
    pub(crate) fn new() -> Self {
        Self {
            frontends: Vec::new(),
        }
    }

    /// Register a frontend; the last registration wins for overlapping extensions.
    pub(crate) fn with(mut self, frontend: impl PipelineFrontend + 'static) -> Self {
        self.frontends.push(Box::new(frontend));
        self
    }

    pub(crate) fn supports(&self, path: &Path) -> bool {
        self.get(path).is_some()
    }

    pub(crate) fn get(&self, path: &Path) -> Option<&dyn PipelineFrontend> {
        let ext = path.extension()?.to_str()?;
        self.frontends
            .iter()
            .rev()
            .find(|frontend| frontend.extensions().contains(&ext))
            .map(|frontend| frontend.as_ref())
    }
}

impl Default for Frontends {
    fn default() -> Self {
        Self::new()
            .with(NotistFrontend)
            .with(markdown::MarkdownFrontend)
    }
}
