use std::path::{Path, PathBuf};

use crate::{Diagnostic, Frontend, Frontends, Item};

/// A configured source-to-IR pipeline.
///
/// [`Default`] installs the `.not` and Markdown frontends. [`Self::new`]
/// starts empty, so callers can choose exactly which frontends to install.
///
/// ```
/// use notist::{Frontend, Notist};
///
/// let notist = Notist::new().with_frontend(Frontend::markdown());
/// assert!(notist.analyze("example.md", "Text").is_ok());
/// assert!(notist.analyze("example.not", "Text").is_err());
/// ```
pub struct Notist {
    frontends: Frontends,
    registry: crate::Registry,
}

impl Notist {
    /// An empty pipeline. Use [`Self::default`] for the built-in frontends.
    pub fn new() -> Self {
        Self {
            frontends: Frontends::new(),
            registry: crate::builtins::registry().clone(),
        }
    }

    /// Install a frontend. The last registration wins for overlapping extensions.
    pub fn with_frontend(mut self, frontend: Frontend) -> Self {
        self.frontends = self.frontends.with(frontend);
        self
    }

    /// Use an explicitly assembled signature environment. Performs no IO.
    pub fn with_registry(mut self, registry: crate::Registry) -> Self {
        self.registry = registry;
        self
    }

    pub fn registry(&self) -> &crate::Registry {
        &self.registry
    }

    /// Analyze in-memory source, selecting a frontend by `path`'s extension.
    ///
    /// `path` is only used for dispatch; this does not read the filesystem.
    /// Source errors are collected in [`Analysis::diagnostics`], alongside
    /// the recovered tree. `Err` means no frontend handles the file format.
    /// Diagnostics cover lowering and the shared backend; document indexes
    /// and cross-document link checks are separate operations.
    pub fn analyze(
        &self,
        path: impl AsRef<Path>,
        src: &str,
    ) -> Result<Analysis, UnsupportedFormat> {
        let path = path.as_ref();
        let (root, diagnostics) = self
            .frontends
            .analyze_with_registry(path, src, &self.registry)
            .ok_or_else(|| UnsupportedFormat {
                path: path.to_path_buf(),
            })?;
        Ok(Analysis { root, diagnostics })
    }
}

impl Default for Notist {
    fn default() -> Self {
        Self::new()
            .with_frontend(Frontend::notist())
            .with_frontend(Frontend::markdown())
    }
}

/// The final document IR and all diagnostics produced while analyzing it.
///
/// The root is a [`crate::Ctor::Doc`] node. Its children are in document
/// order; [`Item::descendants`] walks the root and its descendants in preorder.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    root: Item,
    diagnostics: Vec<Diagnostic>,
}

impl Analysis {
    /// The final tree, suitable for passing to a renderer.
    pub fn root(&self) -> &Item {
        &self.root
    }

    /// Syntax, semantic, and type diagnostics collected by the pipeline.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Take ownership of the tree and diagnostics.
    pub fn into_parts(self) -> (Item, Vec<Diagnostic>) {
        (self.root, self.diagnostics)
    }
}

/// No registered frontend handles this path's extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedFormat {
    pub path: PathBuf,
}

impl std::fmt::Display for UnsupportedFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported file format: {}", self.path.display())
    }
}

impl std::error::Error for UnsupportedFormat {}
