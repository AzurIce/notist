//! Environment-independent Markup lowering for documents and Code literals.
//! The resulting IR₁ is the input to the shared content backend.
use notist_core::{annotation::Annotation, diag::Diagnostic, expr::Expr};
use notist_syntax::content::{ContentParser, ParsedContent};

/// A frontend's lowered content literal: expressions plus source-positioned
/// annotations, both in the literal's local coordinates.
#[derive(Debug, Default)]
pub struct Lowered {
    pub forest: Vec<Expr>,
    pub annotations: Vec<Annotation>,
}

pub trait ContentLowering: ContentParser {
    /// Lower the tree produced by this frontend, using the original source
    /// coordinates in `content`. No document root is introduced.
    fn lower_content(&self, content: &ParsedContent, diagnostics: &mut Vec<Diagnostic>) -> Lowered;
}
