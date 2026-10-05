//! Source-to-document pipeline. All inputs are explicit; no filesystem or browser IO.
pub use analysis::{Analysis, Inspection, Pipeline, UnsupportedFormat};
pub use definitions::analyze_module;
pub use frontend::{Frontend, Frontends};
pub use notist_core::diag::{Diagnostic, Phase};
pub use notist_core::item::{Ctor, Dict, Item, Value};
pub use notist_core::registry::Registry;
pub use notist_core::{builtins, diag, dump, expr, item, registry};
use rowan::TextRange;
pub mod analysis;
pub mod definitions;
pub mod desugar;
pub mod frontend;
mod literals;
pub mod materialize;
pub mod resolve;
pub mod shape;
pub mod transforms;

/// Shared backend for any frontend's lowered Expr forest.
pub fn process(
    forest: Vec<expr::Expr>,
    span: TextRange,
    attrs: Dict,
    registry: &Registry,
    diagnostics: &mut Vec<Diagnostic>,
) -> Item {
    process_with_inspection(forest, span, attrs, registry, diagnostics, None)
}

fn process_with_inspection(
    forest: Vec<expr::Expr>,
    span: TextRange,
    attrs: Dict,
    registry: &Registry,
    diagnostics: &mut Vec<Diagnostic>,
    inspection: Option<&mut Inspection>,
) -> Item {
    let forest = resolve::resolve_with_registry(forest, registry, diagnostics);
    let forest = shape::shape(forest);
    if let Some(inspection) = inspection {
        inspection.shaped = forest.clone();
    }
    materialize::materialize_doc(&forest, span, attrs)
}
