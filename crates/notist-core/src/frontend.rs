//! Source frontend contracts. Concrete frontends own parsing and AST lowering.
use std::any::Any;
use std::fmt::Debug;

use crate::diag::Diagnostic;
use crate::expr::Expr;
use crate::item::Dict;

/// Optional parse-tree capture for tools; ordinary compilation retains only IR₁.
#[derive(Debug, Default, Clone, Copy)]
pub struct FrontendOptions {
    pub capture_syntax: bool,
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

/// Unresolved document expressions, root attributes and frontend diagnostics.
#[derive(Debug, Default)]
pub struct FrontendOutput {
    pub forest: Vec<Expr>,
    pub module_attrs: Dict,
    pub diagnostics: Vec<Diagnostic>,
    pub syntax: Option<Box<dyn FrontendSyntax>>,
}

/// Parse source and lower the frontend's AST to the common IR₁.
/// Name resolution, shaping and materialization run in the shared backend.
pub trait Frontend: Send + Sync {
    fn extensions(&self) -> &[&str];
    fn compile(&self, source: &str, options: FrontendOptions) -> FrontendOutput;
}
