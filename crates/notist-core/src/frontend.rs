//! Source frontend contracts. Concrete frontends own parsing and AST lowering.
use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use crate::annotation::{Annotation, LiteralDict};
use crate::diag::Diagnostic;
use crate::expr::Expr;

/// Optional parse-tree capture for tools; ordinary compilation retains only IR₁.
#[derive(Debug, Default, Clone, Copy)]
pub struct FrontendOptions {
    pub capture_syntax: bool,
}

/// In-process identity of a deterministic frontend implementation and all of its
/// configuration. Not a display name or an instance address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FrontendCacheKey {
    implementation: &'static str,
    version: &'static str,
    configuration: Arc<str>,
}

impl FrontendCacheKey {
    pub fn new<T: ?Sized>(version: &'static str, configuration: impl Into<Arc<str>>) -> Self {
        Self {
            implementation: std::any::type_name::<T>(),
            version,
            configuration: configuration.into(),
        }
    }
}

/// A frontend-owned parse tree. Tools can inspect its concrete representation
/// without making core or the pipeline depend on a language's AST types.
pub trait FrontendSyntax: Any + Debug {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Debug> FrontendSyntax for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Static imports, unresolved document expressions, node annotations, root
/// attributes and frontend diagnostics.
#[derive(Debug, Default)]
pub struct FrontendOutput {
    pub imports: Vec<crate::definitions::Import>,
    pub forest: Vec<Expr>,
    /// Node annotations in source order; attached after shaping.
    pub annotations: Vec<Annotation>,
    pub module_attrs: LiteralDict,
    pub diagnostics: Vec<Diagnostic>,
    pub syntax: Option<Box<dyn FrontendSyntax>>,
}

/// Parse source and lower the frontend's AST to the common IR₁.
/// Name resolution, shaping and materialization run in the shared backend.
pub trait Frontend: Send + Sync {
    fn extensions(&self) -> &[&str];
    fn compile(&self, source: &str, options: FrontendOptions) -> FrontendOutput;

    /// Opt into module compilation caching. Equal keys promise identical results
    /// for equal explicit inputs, including ContentParser/ContentLowering behavior.
    /// Include every configuration value; hidden IO/state must not affect output.
    /// Custom frontends are uncached unless they explicitly make this promise.
    fn cache_key(&self) -> Option<FrontendCacheKey> {
        None
    }
}
