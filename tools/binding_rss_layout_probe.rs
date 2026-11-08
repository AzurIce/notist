#![allow(dead_code)]
use std::collections::BTreeSet;
// Same fields as the private SSG document; Rust field order is not an ABI promise.
struct DocumentShape {
    page: notist_ssg::Page,
    root: notist::Item,
    source: String,
    sources: notist::Sources,
    ids: BTreeSet<String>,
}
fn main() {
    macro_rules! report { ($($ty:ty),*) => { $(println!("{} {}",stringify!($ty),size_of::<$ty>());)* }; }
    report!(notist::Sources, notist::Source, notist::Analysis, notist::Context,
        notist::Environment, notist::DocumentHandle, notist_pipeline::CodeProgram,
        notist_core::evaluation::CheckedProgram, notist_core::evaluation::InstanceRecord,
        notist_core::evaluation::TypedExpr, notist_core::evaluation::Invocation,
        notist_pipeline::Evaluation, notist_core::registry::Registry, notist_pipeline::ModuleScope, DocumentShape);
}
