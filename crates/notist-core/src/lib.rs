use rowan::TextRange;

use crate::diag::Diagnostic;
use crate::expr::Expr;
use crate::item::{Dict, Item};

pub mod builtins;
pub mod diag;
pub mod dump;
pub mod expr;
pub mod index;
pub mod item;
pub mod materialize;
pub mod resolve;
pub mod shape;

/// The shared backend pipeline: an `IR₁` forest → resolve → shape →
/// materialize → core tree. Frontends (`.not`, Markdown, …) only produce
/// the forest and the module attrs.
pub fn analyze(
    forest: Vec<Expr>,
    span: TextRange,
    module_attrs: Dict,
    diagnostics: &mut Vec<Diagnostic>,
) -> Item {
    let forest = resolve::resolve(forest, diagnostics);
    let forest = shape::shape(forest);
    materialize::materialize_doc(&forest, span, module_attrs)
}
