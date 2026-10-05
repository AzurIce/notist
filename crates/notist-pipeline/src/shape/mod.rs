use notist_core::builtins::Accepts;
use notist_core::expr::{BodyFlavor, Expr, RExpr};
use notist_core::item::Ctor;

pub(crate) mod reflow;
mod sectionize;

/// Shaping an `IR₂` forest into its final structure. Per block sequence (the
/// document body and every block-level children mount): `reflow` splits
/// paragraphs around block-level elements, the exposed block mounts are
/// shaped recursively, then `sectionize` groups the flat sequence into
/// sections by heading level.
pub fn shape(forest: Vec<RExpr>) -> Vec<RExpr> {
    let forest = reflow::reflow(forest);
    let forest = forest.into_iter().map(shape_children).collect();
    sectionize::sectionize(forest)
}

fn shape_children(mut expr: RExpr) -> RExpr {
    let Expr::Call {
        name,
        body,
        children,
        ..
    } = &mut expr
    else {
        return expr;
    };
    if matches!(
        name.accepts(),
        Some(Accepts::Rows | Accepts::Cells | Accepts::Items)
    ) {
        // Expose item/row/cell calls from paragraph candidates, then recurse
        // into their content without sectionizing the structural sequence.
        *children = reflow::reflow(std::mem::take(children))
            .into_iter()
            .map(shape_children)
            .collect();
    } else if is_block_mount(name, body) {
        *children = shape(std::mem::take(children));
    }
    expr
}

/// Whether a call's children mount is a block sequence: builtins by their
/// declared acceptance (an unconstrained mount follows the declared flavor),
/// custom recovery nodes by flavor alone.
fn is_block_mount(ctor: &Ctor, body: &BodyFlavor) -> bool {
    match ctor.accepts() {
        Some(Accepts::Content) => true,
        Some(Accepts::Inline | Accepts::Nothing) => false,
        Some(Accepts::Any) => *body != BodyFlavor::Inline,
        Some(Accepts::Rows | Accepts::Cells | Accepts::Items) => false,
        None => *body == BodyFlavor::Block,
    }
}
