use rowan::TextRange;

use notist_core::expr::RExpr;
use notist_core::item::{Ctor, Dict, Item};

/// M1 materialization: no environments, no computation — resolution,
/// checking, and normalization happened in `resolve`; shaping in `shape`.
/// This is the thin structural conversion from a shaped `IR₂` forest to the
/// core tree.
pub fn materialize(expr: &RExpr) -> Item {
    let mut item = Item::new(expr.ctor.clone(), expr.span);
    item.instance = expr.instance.clone();
    item.pending = expr.pending.clone();
    item.fields = expr.fields.clone();
    item.children = expr.children.iter().map(materialize).collect();
    item.attrs = expr.attrs.clone();
    item.level = if crate::shape::is_block_element(expr) {
        notist_core::builtins::Level::Block
    } else {
        notist_core::builtins::Level::Inline
    };
    item
}

/// Materialize a document: the shaped forest becomes the children of a `Doc`
/// root carrying the module attrs — `Doc` is the module boundary, not a
/// constructor anyone can call.
pub fn materialize_doc(forest: &[RExpr], span: TextRange, module_attrs: Dict) -> Item {
    let mut doc = Item::new(Ctor::Doc, span);
    doc.children = forest.iter().map(materialize).collect();
    doc.attrs = module_attrs;
    doc
}
