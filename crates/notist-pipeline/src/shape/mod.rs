use notist_core::builtins::{Accepts, Level};
use notist_core::expr::{BodyFlavor, RExpr};
use notist_core::item::Ctor;

mod sectionize;

/// Shape each content mount recursively, then group block sequences into
/// sections by heading level. Resolve has already classified source fragments
/// as paragraphs or standalone blocks; Shape never splits a paragraph.
pub fn shape(forest: Vec<RExpr>) -> Vec<RExpr> {
    let forest = forest.into_iter().map(shape_children).collect();
    sectionize::sectionize(forest)
}

pub(crate) fn shape_inline(forest: Vec<RExpr>) -> Vec<RExpr> {
    forest.into_iter().map(shape_children).collect()
}

fn shape_children(mut expr: RExpr) -> RExpr {
    let RExpr {
        ctor,
        body,
        children,
        ..
    } = &mut expr;
    if matches!(
        ctor.accepts(),
        Some(Accepts::Rows | Accepts::Cells | Accepts::Items)
    ) {
        // Structural sequences do not form paragraphs or sections.
        *children = std::mem::take(children)
            .into_iter()
            .map(shape_children)
            .collect();
    } else if is_block_mount(ctor, body) {
        *children = shape(std::mem::take(children));
    } else {
        *children = shape_inline(std::mem::take(children));
    }
    expr
}

/// The public output category decides placement, independently of the input
/// mount category. Unresolved calls use their written body flavor for recovery.
pub(crate) fn is_block_element(expr: &RExpr) -> bool {
    if let Some(level) = expr.level {
        return level == Level::Block;
    }
    match expr.ctor.level() {
        Some(Level::Block) => true,
        Some(Level::Inline) => false,
        Some(Level::Inherit) | None => expr.body == BodyFlavor::Block,
    }
}

/// Whether a call's children mount is a block sequence: builtins by their
/// declared acceptance (an unconstrained mount follows the declared flavor),
/// custom recovery nodes by flavor alone.
fn is_block_mount(ctor: &Ctor, body: &BodyFlavor) -> bool {
    match ctor.accepts() {
        Some(Accepts::Block) => true,
        Some(Accepts::Inline | Accepts::Nothing) => false,
        Some(Accepts::Any) => *body != BodyFlavor::Inline,
        Some(Accepts::Rows | Accepts::Cells | Accepts::Items) => false,
        None => *body == BodyFlavor::Block,
    }
}
