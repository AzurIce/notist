use rowan::TextRange;

use crate::expr::{Expr, RExpr};
use crate::item::{Ctor, Dict, Item};

/// M1 materialization: no environments, no computation — resolution,
/// checking, and normalization happened in `resolve`; shaping in `shape`.
/// This is the thin structural conversion from a shaped `IR₂` forest to the
/// core tree.
pub fn materialize(expr: &RExpr) -> Item {
    match expr {
        Expr::Literal(..) => unreachable!("literals insert as text at resolve"),
        Expr::Call {
            name,
            fields,
            children,
            attrs,
            span,
            ..
        } => {
            let mut item = Item::new(name.clone(), *span);
            item.fields = fields.clone();
            item.children = children.iter().map(materialize).collect();
            item.attrs = attrs.clone();
            item.level = if crate::shape::reflow::is_block_element(expr) {
                crate::builtins::Level::Block
            } else {
                crate::builtins::Level::Inline
            };
            item
        }
    }
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
