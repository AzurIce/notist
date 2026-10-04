use crate::builtins::Level;
use crate::expr::{BodyFlavor, Expr, RExpr};
use crate::item::{Ctor, Value};
use rowan::TextRange;

/// Signature-aware restructuring of an `IR₂` forest: paragraphs that contain
/// block-level elements are split into `paragraph | element | paragraph`
/// siblings around them. Everything else passes through unchanged.
pub fn reflow(forest: Vec<RExpr>) -> Vec<RExpr> {
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

fn split_paragraph(paragraph: RExpr, out: &mut Vec<RExpr>) {
    let Expr::Call {
        children,
        attrs,
        fields,
        span,
        ..
    } = paragraph
    else {
        unreachable!();
    };
    let mut fragments = Vec::new();
    let mut run: Vec<RExpr> = Vec::new();
    for child in children {
        if is_block_element(&child) {
            flush_run(&mut fragments, &mut run);
            fragments.push(child);
        } else {
            run.push(child);
        }
    }
    flush_run(&mut fragments, &mut run);
    if attrs.is_empty() && fields.is_empty() {
        out.extend(fragments);
    } else {
        // One annotated range remains one identity even after it splits.
        out.push(Expr::Call {
            name: Ctor::Group,
            args: Vec::new(),
            fields,
            children: fragments,
            body: BodyFlavor::Block,
            attrs,
            span,
        });
    }
}

/// A non-empty run of inline children becomes a paragraph item (a run that is
/// itself a single paragraph passes through as-is).
fn flush_run(out: &mut Vec<RExpr>, run: &mut Vec<RExpr>) {
    match run.len() {
        0 => {}
        1 => {
            let child = run.pop().unwrap();
            if is_paragraph(&child) {
                out.push(child);
            } else {
                let span = child.span();
                out.push(RExpr::resolved(Ctor::Paragraph, span).with_children(vec![child]));
            }
        }
        _ => {
            let span = span_of(run);
            out.push(RExpr::resolved(Ctor::Paragraph, span).with_children(std::mem::take(run)));
        }
    }
}

fn span_of(run: &[RExpr]) -> TextRange {
    let start = run.first().unwrap().span().start();
    let end = run.last().unwrap().span().end();
    TextRange::new(start, end)
}

fn is_paragraph(expr: &RExpr) -> bool {
    matches!(expr, Expr::Call { name, .. } if *name == Ctor::Paragraph)
}

fn contains_block_child(expr: &RExpr) -> bool {
    let Expr::Call { children, .. } = expr else {
        return false;
    };
    children.iter().any(is_block_element)
}

/// Whether this element is block-level: builtins by their static signature
/// (`group` inherits the declared flavor), custom recovery nodes by the
/// declared flavor — a block-written body's block-sequence children must
/// stay in a block-level position to remain coherent.
pub(crate) fn is_block_element(expr: &RExpr) -> bool {
    let Expr::Call {
        name, body, fields, ..
    } = expr
    else {
        return false;
    };
    // fenced raw blocks are block elements regardless of the inline-raw row
    if *name == Ctor::RawInline && fields.get("block") == Some(&Value::Bool(true)) {
        return true;
    }
    match name.level() {
        Some(Level::Block) => true,
        Some(Level::Inline) => false,
        Some(Level::Inherit) | None => *body == BodyFlavor::Block,
    }
}
