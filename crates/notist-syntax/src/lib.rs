//! Lossless native Markup and Code syntax, without semantic IR or lowering.
//!
//! Parsers accept source text and perform no file or package IO.
//! They return a CST and syntax diagnostics; typed AST accessors preserve
//! incomplete declarations for tooling.
//!
//! ```
//! use notist_syntax::code::{ast::Module, parse_module};
//!
//! let parsed = parse_module("fn badge(label: String) -> InlineContent;", &notist_syntax::markup::MarkupContentParser);
//! assert!(parsed.diagnostics.is_empty());
//! let module = Module::cast(parsed.syntax()).unwrap();
//! let function = module.functions().next().unwrap();
//! assert_eq!(function.name().unwrap().text(), "badge");
//! ```

pub mod code;
pub mod content;
pub mod lexed;
pub mod markup;
mod parse;
pub mod tree;

pub use parse::{Diagnostic, Parse};
