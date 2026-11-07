//! Compile with the frozen before/after notist_core rlib, outside measured runs.
use notist_core::{diag::Diagnostic, evaluation::{Invocation, TypedExpr}, item::{Node, Origin, SourceLocation}};

fn main() {
    println!("{{\"Node\":{},\"Origin\":{},\"SourceLocation\":{},\"Invocation\":{},\"TypedExpr\":{},\"Diagnostic\":{}}}",
        size_of::<Node>(), size_of::<Origin>(), size_of::<SourceLocation>(),
        size_of::<Invocation>(), size_of::<TypedExpr>(), size_of::<Diagnostic>());
}
