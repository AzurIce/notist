//! Code declarations, expression fragments and typed AST views.
//! Embedded content is supplied through the syntax-only `ContentParser` trait.
pub mod ast;
pub mod lexer;
pub mod parser;

pub use parser::{
    is_markup_import, parse_annotation, parse_annotation_prefix, parse_call_header,
    parse_call_prefix, parse_content_literal, parse_import, parse_import_prefix, parse_module,
};
