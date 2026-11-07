//! Compile against each frozen core rlib, outside all measured runs.
use notist_core::{
    diag::{Diagnostic, Sources},
    evaluation::{CheckedProgram, Invocation, TypedExpr},
    item::{Node, Origin, SourceLocation},
};
fn main() {
    println!("{{\"Node\":{},\"Origin\":{},\"SourceLocation\":{},\"TypedExpr\":{},\"Invocation\":{},\"Diagnostic\":{},\"Sources\":{},\"CheckedProgram\":{}}}",
        size_of::<Node>(),size_of::<Origin>(),size_of::<SourceLocation>(),size_of::<TypedExpr>(),
        size_of::<Invocation>(),size_of::<Diagnostic>(),size_of::<Sources>(),size_of::<CheckedProgram>());
}
