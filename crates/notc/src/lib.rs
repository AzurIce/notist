//! Standalone implementation of the notc language.
//!
//! Programs can be parsed from `.notc` source text, checked as a single module
//! or as an explicit in-memory multi-file module graph, and lowered by entry
//! reachability before evaluation; no document host or filesystem IO is required.

pub mod ast;
mod builtin;
mod check;
pub mod content;
mod content_parser;
mod eval;
pub mod frontend;
pub mod ide;
pub mod json;
mod lexer;
mod module;
mod parser;

pub use ast::{
    AttrEntry, AttrLiteral, BinaryOp, BlockPiece, DOC_ENTRY, Decl, Expr, ExprKind, Function,
    Import, ImportKind, ImportMember, InlinePiece, Method, Module, Param, Program, Span, Type,
    TypeMatchArm, TypeParam, TypePattern, UnaryOp, WhereExpr, WhereExprKind,
};
pub use builtin::BuiltinFn;
pub use check::{
    CheckedProgram, Diagnostic, TypeError, check, format_type, format_type_error,
};
pub use content::{
    AttrValue, Attrs, Block, BlockKind, BlockNode, CustomNode, CustomValue, Inline, InlineKind,
    InlineNode,
};
pub use eval::{ExecutionError, FunctionValue, Value};
pub use frontend::{DocumentFrontend, NotistFrontend, build_document_module, embedded};
pub use json::{DataHost, JsonData, JsonValue, NoDataHost};
pub use lexer::ParseDiagnostic;
pub use module::{
    CheckedProject, DocumentError, ProjectDiagnostic, ProjectDiagnosticKind, check_parsed_modules,
    check_project, normalize_path, resolve_import_path,
};
pub use parser::{ParseResult, parse, parse_document, parse_file, parse_file_with_frontends};
