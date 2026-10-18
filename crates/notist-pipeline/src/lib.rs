//! Source-to-document pipeline. All inputs are explicit; no filesystem or browser IO.
mod attach;
mod code;
mod context;
pub use code::{Evaluation, Limits};
mod definitions;

pub mod frontend;
pub mod lowering;
pub mod materialize;
pub mod resolve;
pub mod shape;
pub mod transforms;

pub use context::Context;
pub use definitions::{CodeModule, CodeProgram, analyze_module};

use std::path::{Path, PathBuf};

use crate::frontend::{Frontends, PipelineFrontend};
pub use crate::lowering::content::{ContentLowering, Lowered, LoweredContent};
use notist_core::diag::Diagnostic;
use notist_core::expr::{Expr, RExpr};
use notist_core::frontend::{FrontendOptions, FrontendSyntax};
use notist_core::item::{Dict, Item};
use rowan::TextRange;

/// A configured source-to-IR pipeline.
///
/// [`Default`] installs the `.not` and Markdown frontends. [`Self::new`]
/// starts empty, so callers can choose exactly which frontends to install.
///
/// ```
/// use notist_pipeline::{Context, Pipeline};
///
/// let notist = Pipeline::new().with_frontend(notist_pipeline::frontend::markdown::MarkdownFrontend);
/// let context = Context::default();
/// assert!(notist.analyze("example.md", "Text", &context).is_ok());
/// assert!(notist.analyze("example.not", "Text", &context).is_err());
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
    pub fn with_frontend(mut self, frontend: impl PipelineFrontend + 'static) -> Self {
        self.frontends = self.frontends.with(frontend);
        self
    }

    pub fn supports(&self, path: &Path) -> bool {
        self.frontends.supports(path)
    }

    /// Parse and lower a Code content-literal prefix using the dialect selected
    /// by `dialect_path` (`.not` or `.md` for the default frontends). The result
    /// is a forest without a Doc root; the Code CST and frontend tree remain
    /// available separately. Does not type-check, shape, materialize or evaluate.
    pub fn lower_content_literal(
        &self,
        dialect_path: impl AsRef<Path>,
        source: &str,
    ) -> Result<(LoweredContent, notist_syntax::Parse), UnsupportedFormat> {
        let path = dialect_path.as_ref();
        let frontend = self
            .frontends
            .get(path)
            .ok_or_else(|| UnsupportedFormat { path: path.into() })?;
        let parsed = notist_syntax::code::parse_content_literal(source, frontend);
        let lowered = crate::lowering::content::lower_content_literal(&parsed, frontend);
        Ok((lowered, parsed))
    }

    /// Analyze in-memory source, selecting a frontend by `path`'s extension.
    ///
    /// `path` is only used for dispatch; this does not read the filesystem.
    /// Source errors are collected in [`Analysis::diagnostics`], alongside
    /// the recovered tree. `Err` means no frontend handles the file format.
    /// Diagnostics cover lowering and the shared backend; document indexes
    /// and cross-document link checks are separate operations.
    /// Context supplies function definitions; output transforms are applied
    /// separately to the returned analysis tree.
    pub fn analyze(
        &self,
        path: impl AsRef<Path>,
        src: &str,
        context: &Context,
    ) -> Result<Analysis, UnsupportedFormat> {
        self.compile(path.as_ref(), src, context, false)
            .map(|(analysis, _)| analysis)
    }
    pub fn compile_module(
        &self,
        dialect_path: impl AsRef<Path>,
        package: &str,
        source_path: &str,
        source: &str,
    ) -> Result<CodeModule, Vec<Diagnostic>> {
        let Some(frontend) = self.frontends.get(dialect_path.as_ref()) else {
            return Err(vec![Diagnostic::new(
                notist_core::diag::Phase::Syntax,
                TextRange::empty(0.into()),
                "unsupported content dialect",
            )]);
        };
        definitions::compile_module(package, source_path, source, frontend)
    }
    /// Check all Code definitions whose compile-time parameters have defaults.
    /// Other generic instances are checked when called with concrete arguments.
    pub fn check(&self, context: &Context) -> Vec<Diagnostic> {
        code::Checker::new(context, "<module>").check_definitions()
    }

    /// Check one package in its installed dependency context. Generic bodies
    /// retain syntax/lowering diagnostics even before a concrete instance exists.
    pub fn check_module(&self, package: &str, context: &Context) -> Vec<Diagnostic> {
        code::Checker::new(context, "<module>").check_module(package)
    }

    /// Evaluate pending calls and forest interpolations directly on the Item tree.
    /// Existing Item structure is never reshaped. A mapping target may first
    /// check its own content literals; the semantic tree remains available.
    pub fn evaluate(&self, analysis: &Analysis, context: &Context) -> Evaluation {
        self.evaluate_with_limits(analysis, context, Limits::default())
    }
    pub fn evaluate_with_limits(
        &self,
        analysis: &Analysis,
        context: &Context,
        limits: Limits,
    ) -> Evaluation {
        code::eval::evaluate(&analysis.root, &analysis.program, context, limits)
    }

    /// Collect frontend-owned syntax and debug stages in the same source-to-IR pass.
    /// The selected frontend runs exactly once.
    pub fn inspect(
        &self,
        path: impl AsRef<Path>,
        src: &str,
        context: &Context,
    ) -> Result<(Analysis, Inspection), UnsupportedFormat> {
        self.compile(path.as_ref(), src, context, true)
    }
    fn compile(
        &self,
        path: &Path,
        src: &str,
        context: &Context,
        capture: bool,
    ) -> Result<(Analysis, Inspection), UnsupportedFormat> {
        let frontend = self
            .frontends
            .get(path)
            .ok_or_else(|| UnsupportedFormat { path: path.into() })?;
        let output = frontend.compile(
            src,
            FrontendOptions {
                capture_syntax: capture,
            },
        );
        let mut diagnostics = output.diagnostics;
        let lowered = if capture {
            output.forest.clone()
        } else {
            Vec::new()
        };
        let span = TextRange::new(0.into(), (src.len() as u32).into());
        let (root, program, errors, shaped) = code::Checker::new(context, &path.to_string_lossy())
            .document(
                &output.forest,
                &output.annotations,
                span,
                output.module_attrs,
            );
        diagnostics.extend(errors);
        let inspection = Inspection {
            syntax: output.syntax,
            lowered,
            shaped: if capture { shaped } else { Vec::new() },
        };
        Ok((
            Analysis {
                root,
                diagnostics,
                program,
                sources: {
                    let mut s = context.program().sources.clone();
                    s.insert(path.to_string_lossy().into_owned(), src.into());
                    s
                },
            },
            inspection,
        ))
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self {
            frontends: Frontends::default(),
        }
    }
}

/// The semantic Item tree, checked function instances, sources and diagnostics.
///
/// The root is a [`notist_core::item::Ctor::Doc`] node. Its children are in document
/// order; [`Item::descendants`] walks the root and its descendants in preorder.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    root: Item,
    diagnostics: Vec<Diagnostic>,
    program: code::CheckedProgram,
    sources: std::collections::BTreeMap<String, String>,
}

impl Analysis {
    /// Sources retained for document/definition diagnostics without host IO.
    pub fn sources(&self) -> &std::collections::BTreeMap<String, String> {
        &self.sources
    }

    /// The semantic tree; evaluate before rendering Code implementations.
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
    annotations: Vec<notist_core::annotation::Annotation>,
    span: TextRange,
    attrs: Dict,
    context: &Context,
    diagnostics: &mut Vec<Diagnostic>,
) -> Item {
    process_with_inspection(forest, annotations, span, attrs, context, diagnostics, None)
}

fn process_with_inspection(
    forest: Vec<Expr>,
    annotations: Vec<notist_core::annotation::Annotation>,
    span: TextRange,
    attrs: Dict,
    context: &Context,
    diagnostics: &mut Vec<Diagnostic>,
    inspection: Option<&mut Inspection>,
) -> Item {
    let forest = crate::resolve::resolve(forest, context, diagnostics);
    let mut forest = crate::shape::shape(forest);
    if let Some(inspection) = inspection {
        inspection.shaped = forest.clone();
    }
    crate::attach::attach(&mut forest, &annotations, diagnostics);
    crate::materialize::materialize_doc(&forest, span, attrs)
}
