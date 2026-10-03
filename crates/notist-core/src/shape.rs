use crate::builtins::{self, Accepts};
use crate::expr::{BodyFlavor, Expr};
use crate::{reflow, sectionize};

/// IR₁ → IR₂: shape the candidate structure into its final form. Per block
/// sequence (the document body and every block-level children mount):
/// `reflow` splits paragraph candidates around block-level elements, the
/// exposed block mounts are shaped recursively, then `sectionize` groups
/// the flat sequence into sections by heading level.
pub fn shape(forest: Vec<Expr>) -> Vec<Expr> {
    let forest = reflow::reflow(forest);
    let forest = forest.into_iter().map(shape_children).collect();
    sectionize::sectionize(forest)
}

fn shape_children(mut expr: Expr) -> Expr {
    let Expr::Call {
        name,
        body,
        children,
        ..
    } = &mut expr
    else {
        return expr;
    };
    if is_block_mount(name, body) {
        *children = shape(std::mem::take(children));
    }
    expr
}

/// Whether a call's children mount is a block sequence: builtins by their
/// declared acceptance (an unconstrained mount follows the declared flavor),
/// custom constructors by flavor alone.
fn is_block_mount(name: &str, body: &BodyFlavor) -> bool {
    match builtins::builtin_signature(name) {
        Some(sig) => match sig.accepts {
            Accepts::Content => true,
            Accepts::Inline | Accepts::Nothing => false,
            Accepts::Any => *body != BodyFlavor::Inline,
        },
        None => *body == BodyFlavor::Block,
    }
}
