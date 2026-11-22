//! Standalone implementation of the notc language.
//!
//! Programs can be parsed from `.notc` source text, checked as a single module
//! or as an explicit in-memory multi-file module graph, and lowered by entry
//! reachability before evaluation; no document host or filesystem IO is required.

pub mod ast;
mod check;
mod eval;
mod lexer;
mod module;
mod parser;

pub use ast::{
    BinaryOp, Decl, Expr, ExprKind, Function, Import, ImportKind, ImportMember, Method, Module,
    Param, Program, Span, Type, UnaryOp,
};
pub use check::{CheckedProgram, Diagnostic, TypeError, check};
pub use eval::{ExecutionError, FunctionValue, Value};
pub use lexer::ParseDiagnostic;
pub use module::{
    CheckedProject, ProjectDiagnostic, ProjectDiagnosticKind, check_parsed_modules, check_project,
    normalize_path, resolve_import_path,
};
pub use parser::{ParseResult, parse};
