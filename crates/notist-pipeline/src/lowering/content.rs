//! Content lowering is separate from the syntax-only embedding interface.
use notist_core::{
    annotation::Annotation,
    diag::{Diagnostic, Phase},
    expr::{BodyFlavor, Expr},
};
use notist_syntax::{
    Parse,
    content::{ContentFlavor, ContentParser, ParsedContent},
};
use rowan::TextRange;

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

#[derive(Debug)]
pub struct LoweredContent {
    pub forest: Vec<Expr>,
    pub annotations: Vec<Annotation>,
    pub body: BodyFlavor,
    pub span: TextRange,
    pub diagnostics: Vec<Diagnostic>,
}

/// Lower a content-literal prefix already parsed with the selected frontend.
/// Syntax diagnostics are collected once; the frontend adds lowering errors.
pub fn lower_content_literal(parsed: &Parse, frontend: &dyn ContentLowering) -> LoweredContent {
    let mut diagnostics = parsed
        .diagnostics
        .iter()
        .map(|d| Diagnostic::new(Phase::Syntax, d.span, d.message.clone()))
        .collect();
    let content = parsed
        .contents
        .iter()
        .find(|content| content.range.start() == 0.into());
    let (lowered, body) = if let Some(content) = content {
        (
            frontend.lower_content(content, &mut diagnostics),
            match content.flavor {
                ContentFlavor::Inline => BodyFlavor::Inline,
                ContentFlavor::Block => BodyFlavor::Block,
            },
        )
    } else {
        (Lowered::default(), BodyFlavor::None)
    };
    LoweredContent {
        forest: lowered.forest,
        annotations: lowered.annotations,
        body,
        span: TextRange::new(0.into(), parsed.consumed()),
        diagnostics,
    }
}
