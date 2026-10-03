use crate::builtins::{self, Level};
use crate::expr::{BodyFlavor, Expr};
use rowan::TextRange;

/// Signature-aware restructuring of an `IR₁` forest: paragraphs that contain
/// block-level elements are split into `paragraph | element | paragraph`
/// siblings around them. Everything else passes through unchanged.
pub fn reflow(forest: Vec<Expr>) -> Vec<Expr> {
    let mut out = Vec::new();
    for expr in forest {
        if is_paragraph(&expr) && contains_block_child(&expr) {
            split_paragraph(expr, &mut out);
        } else {
            out.push(expr);
        }
    }
    out
}

fn split_paragraph(paragraph: Expr, out: &mut Vec<Expr>) {
    let Expr::Call { children, .. } = paragraph else {
        unreachable!();
    };
    let mut run: Vec<Expr> = Vec::new();
    for child in children {
        if is_block_element(&child) {
            flush_run(out, &mut run);
            out.push(child);
        } else {
            run.push(child);
        }
    }
    flush_run(out, &mut run);
}

/// A non-empty run of inline children becomes a paragraph item (a run that is
/// itself a single paragraph passes through as-is).
fn flush_run(out: &mut Vec<Expr>, run: &mut Vec<Expr>) {
    match run.len() {
        0 => {}
        1 => {
            let child = run.pop().unwrap();
            if is_paragraph(&child) {
                out.push(child);
            } else {
                let span = child.span();
                out.push(Expr::call("paragraph", span).with_children(vec![child]));
            }
        }
        _ => {
            let span = span_of(run);
            out.push(Expr::call("paragraph", span).with_children(std::mem::take(run)));
        }
    }
}

fn span_of(run: &[Expr]) -> TextRange {
    let start = run.first().unwrap().span().start();
    let end = run.last().unwrap().span().end();
    TextRange::new(start, end)
}

fn is_paragraph(expr: &Expr) -> bool {
    matches!(expr, Expr::Call { name, .. } if name == "paragraph")
}

fn contains_block_child(expr: &Expr) -> bool {
    let Expr::Call { children, .. } = expr else {
        return false;
    };
    children.iter().any(is_block_element)
}

/// Whether this element is block-level: builtins by their static signature
/// table, `group` / custom constructors by the declared body flavor.
fn is_block_element(expr: &Expr) -> bool {
    let Expr::Call {
        name,
        body,
        fields,
        ..
    } = expr
    else {
        return false;
    };
    // fenced raw blocks are block elements regardless of the inline-raw row
    if name == "raw" && fields.get("block") == Some(&crate::item::Value::Bool(true)) {
        return true;
    }
    match builtins::builtin_signature(name) {
        Some(sig) => match sig.level {
            Level::Block => true,
            Level::Inline => false,
            Level::Inherit => *body == BodyFlavor::Block,
        },
        None => *body == BodyFlavor::Block,
    }
}
