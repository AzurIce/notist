//! Standalone implementation of the notc language.
//!
//! Programs are supplied as an AST, without requiring a parser. Checking resolves
//! names before the evaluator runs; no parser or document host is required.

pub mod ast;
mod check;
mod eval;

pub use ast::{Decl, Expr, Function, Module, Program, Type};
pub use check::{CheckedProgram, Diagnostic, TypeError, check};
pub use eval::{ExecutionError, FunctionValue, Value};
