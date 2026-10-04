//! Lossless syntax for Markup documents and Code declaration modules.
//!
//! The two entry points accept source text and perform no file or package IO.
//! Both return a CST and syntax diagnostics; typed AST accessors preserve
//! incomplete declarations for tooling.
//!
//! ```
//! use notist_syntax::{ast::Module, parse_module};
//!
//! let parsed = parse_module("fn badge(label: String) -> InlineContent;");
//! assert!(parsed.diagnostics.is_empty());
//! let module = Module::cast(parsed.syntax()).unwrap();
//! let function = module.functions().next().unwrap();
//! assert_eq!(function.name().unwrap().text(), "badge");
//! ```

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod syntax;

pub use parser::{Diagnostic, Parse, parse_document, parse_module};
