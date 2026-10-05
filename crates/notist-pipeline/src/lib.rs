//! Source-to-document pipeline. All inputs are explicit; no filesystem or browser IO.
mod definitions;
mod notist;

pub mod frontend;
pub mod materialize;
pub mod resolve;
pub mod shape;
pub mod transforms;

pub use definitions::analyze_module;
pub use notist::NotistFrontend;

use std::path::{Path, PathBuf};

use crate::frontend::Frontends;
use notist_core::diag::Diagnostic;
use notist_core::expr::{Expr, RExpr};
use notist_core::frontend::{Frontend, FrontendOptions, FrontendSyntax};
use notist_core::item::{Dict, Item};
use notist_core::registry::Registry;
use rowan::TextRange;

/// A configured source-to-IR pipeline.
///
/// [`Default`] installs the `.not` and Markdown frontends. [`Self::new`]
/// starts empty, so callers can choose exactly which frontends to install.
///
/// ```
/// use notist_pipeline::Pipeline;
///
/// let notist = Pipeline::new().with_frontend(notist_md::MarkdownFrontend);
/// assert!(notist.analyze("example.md", "Text", notist_core::builtins::registry()).is_ok());
/// assert!(notist.analyze("example.not", "Text", notist_core::builtins::registry()).is_err());
/// ```
pub struct Pipeline {
    frontends: Frontends,
}

impl Pipeline {
    /// An empty pipeline. Use [`Self::default`] for the built-in frontends.
    pub fn new() -> Self {
        Self {
            frontends: Frontends::new(),
        }
    }

    /// Install a frontend. The last registration wins for overlapping extensions.
    pub fn with_frontend(mut self, frontend: impl Frontend + 'static) -> Self {
        self.frontends = self.frontends.with(frontend);
        self
    }

    pub fn supports(&self, path: &Path) -> bool {
        self.frontends.supports(path)
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
        registry: &Registry,
    ) -> Result<Analysis, UnsupportedFormat> {
        let path = path.as_ref();
        let (root, diagnostics) = self
            .frontends
            .analyze_with_registry(path, src, registry)
            .ok_or_else(|| UnsupportedFormat {
                path: path.to_path_buf(),
            })?;
        Ok(Analysis { root, diagnostics })
    }
    /// Collect frontend-owned syntax and debug stages in the same source-to-IR pass.
    /// Every registered frontend runs exactly once.
    pub fn inspect(
        &self,
        path: impl AsRef<Path>,
        src: &str,
        registry: &Registry,
    ) -> Result<(Analysis, Inspection), UnsupportedFormat> {
        let path = path.as_ref();
        let frontend = self
            .frontends
            .get(path)
            .ok_or_else(|| UnsupportedFormat { path: path.into() })?;
        let output = frontend.compile(
            src,
            FrontendOptions {
                capture_syntax: true,
            },
        );
        let forest = output.forest;
        let attrs = output.module_attrs;
        let mut diagnostics = output.diagnostics;
        let mut inspection = Inspection {
            syntax: output.syntax,
            lowered: forest.clone(),
            shaped: Vec::new(),
        };
        let span = rowan::TextRange::new(0.into(), (src.len() as u32).into());
        let root = process_with_inspection(
            forest,
            span,
            attrs,
            registry,
            &mut diagnostics,
            Some(&mut inspection),
        );
        Ok((Analysis { root, diagnostics }, inspection))
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self {
            frontends: Frontends::default(),
        }
    }
}

/// The final document IR and all diagnostics produced while analyzing it.
///
/// The root is a [`notist_core::item::Ctor::Doc`] node. Its children are in document
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

    /// Syntax, semantic, and type diagnostics, including attached host checks.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Attach diagnostics from checks that require host context, preserving IR.
    pub fn extend_diagnostics(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>) {
        self.diagnostics.extend(diagnostics);
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

/// Optional debugging data, absent from ordinary analysis and rendering outputs.
#[derive(Debug)]
pub struct Inspection {
    pub syntax: Option<Box<dyn FrontendSyntax>>,
    pub lowered: Vec<Expr>,
    pub shaped: Vec<RExpr>,
}

/// Shared backend for any frontend's lowered Expr forest.
pub fn process(
    forest: Vec<Expr>,
    span: TextRange,
    attrs: Dict,
    registry: &Registry,
    diagnostics: &mut Vec<Diagnostic>,
) -> Item {
    process_with_inspection(forest, span, attrs, registry, diagnostics, None)
}

fn process_with_inspection(
    forest: Vec<Expr>,
    span: TextRange,
    attrs: Dict,
    registry: &Registry,
    diagnostics: &mut Vec<Diagnostic>,
    inspection: Option<&mut Inspection>,
) -> Item {
    let forest = crate::resolve::resolve_with_registry(forest, registry, diagnostics);
    let forest = crate::shape::shape(forest);
    if let Some(inspection) = inspection {
        inspection.shaped = forest.clone();
    }
    crate::materialize::materialize_doc(&forest, span, attrs)
}
