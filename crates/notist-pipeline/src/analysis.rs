use std::path::{Path, PathBuf};

use crate::{Diagnostic, Frontend, Frontends, Item};

/// A configured source-to-IR pipeline.
///
/// [`Default`] installs the `.not` and Markdown frontends. [`Self::new`]
/// starts empty, so callers can choose exactly which frontends to install.
///
/// ```
/// use notist_pipeline::{Frontend, Pipeline};
///
/// let notist = Pipeline::new().with_frontend(Frontend::markdown());
/// assert!(notist.analyze("example.md", "Text", notist_pipeline::builtins::registry()).is_ok());
/// assert!(notist.analyze("example.not", "Text", notist_pipeline::builtins::registry()).is_err());
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
    pub fn with_frontend(mut self, frontend: Frontend) -> Self {
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
        registry: &crate::Registry,
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
    /// Collect debug stages during the same source-to-IR pass. Custom frontends
    /// still run exactly once; their debug result has no Notist CST.
    pub fn inspect(
        &self,
        path: impl AsRef<Path>,
        src: &str,
        registry: &crate::Registry,
    ) -> Result<(Analysis, Inspection), UnsupportedFormat> {
        let path = path.as_ref();
        let frontend = self
            .frontends
            .get(path)
            .ok_or_else(|| UnsupportedFormat { path: path.into() })?;
        let syntax =
            std::ptr::fn_addr_eq(frontend.lower, crate::desugar::lower_not as fn(&str) -> _)
                .then(|| notist_syntax::parse_document(src));
        let (forest, attrs, mut diagnostics) = match &syntax {
            Some(parse) => crate::desugar::lower_parsed(parse),
            None => (frontend.lower)(src),
        };
        let mut inspection = Inspection {
            syntax: syntax.map(|parse| parse.syntax()),
            lowered: forest.clone(),
            shaped: Vec::new(),
        };
        let span = rowan::TextRange::new(0.into(), (src.len() as u32).into());
        let root = crate::process_with_inspection(
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

/// Optional debugging data, absent from ordinary analysis and rendering outputs.
#[derive(Debug)]
pub struct Inspection {
    pub syntax: Option<notist_syntax::syntax::SyntaxNode>,
    pub lowered: Vec<crate::expr::Expr>,
    pub shaped: Vec<crate::expr::RExpr>,
}
