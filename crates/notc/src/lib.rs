//! Standalone implementation of the notc language.
//!
//! Programs can be parsed from `.notc` source text or supplied directly as an
//! AST. Checking resolves names and types across all declarations before the
//! evaluator runs; no document host is required.

pub mod ast;
mod check;
mod eval;
mod lexer;
mod parser;

pub use ast::{Decl, Expr, ExprKind, Function, Method, Module, Param, Program, Span, Type};
pub use check::{CheckedProgram, Diagnostic, TypeError, check};
pub use eval::{ExecutionError, FunctionValue, Value};
pub use lexer::ParseDiagnostic;
pub use parser::{ParseResult, parse};
