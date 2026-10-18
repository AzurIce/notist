//! Native `.not` syntax and typed AST views. Content lowering lives outside
//! this crate; Code fragments use the sibling `code` parser entry points.
pub mod ast;
pub mod lexer;
pub mod parser;

pub use parser::parse_document;

/// Native content syntax for a caller-selected Code content literal.
#[derive(Debug, Default)]
pub struct MarkupContentParser;

impl crate::content::ContentParser for MarkupContentParser {
    fn parse_content(&self, source: &str, open: rowan::TextSize) -> crate::content::ParsedContent {
        parser::parse_content(source, open)
    }
}
