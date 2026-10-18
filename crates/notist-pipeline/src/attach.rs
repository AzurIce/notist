//! Attach source-positioned annotations to the shaped content tree.
//!
//! Annotations stay out of shaping: they travel beside the IR and are resolved
//! against the final node order. A block annotation targets the first block
//! after its position; an inline annotation targets the immediately following
//! inline element. No reshaping or implicit wrapper is introduced here.

use notist_core::annotation::{Annotation, AnnotationKind};
use notist_core::diag::{Diagnostic, Phase};
use notist_core::expr::{BodyFlavor, RExpr};
use notist_core::item::Ctor;
use rowan::{TextRange, TextSize};

use crate::shape::is_block_element;

struct Group {
    kind: AnnotationKind,
    dict: notist_core::item::Dict,
    span: TextRange,
    end: TextSize,
}

pub(crate) fn attach(
    forest: &mut [RExpr],
    annotations: &[Annotation],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut sorted: Vec<&Annotation> = annotations.iter().collect();
    sorted.sort_by_key(|annotation| annotation.span.start());

    let mut groups: Vec<Group> = Vec::new();
    for annotation in sorted {
        if let Some(last) = groups.last_mut()
            && last.kind == annotation.kind
            && last.end == annotation.span.start()
        {
            last.dict.extend(annotation.dict.clone());
            last.end = annotation.span.end();
            last.span = last.span.cover(annotation.span);
            continue;
        }
        groups.push(Group {
            kind: annotation.kind,
            dict: annotation.dict.clone(),
            span: annotation.span,
            end: annotation.span.end(),
        });
    }

    for group in groups {
        match group.kind {
            AnnotationKind::Block => attach_block(forest, &group, diagnostics),
            AnnotationKind::Inline => attach_inline(forest, &group, diagnostics),
        }
    }
}

fn attach_block(forest: &mut [RExpr], group: &Group, diagnostics: &mut Vec<Diagnostic>) {
    let seq = match block_sequence(forest, group.span) {
        Ok(seq) => seq,
        Err(()) => {
            diagnostics.push(Diagnostic::new(
                Phase::Semantic,
                group.span,
                "block annotation is not allowed inside inline content",
            ));
            return;
        }
    };

    if let Some(target) = seq
        .iter_mut()
        .find(|expr| is_block_element(expr) && expr.span().start() >= group.end)
    {
        target.attrs.extend(group.dict.clone());
    } else {
        diagnostics.push(Diagnostic::new(
            Phase::Semantic,
            group.span,
            "block annotation has no following block",
        ));
    }
}

/// The innermost block sequence containing `span`; `Err` means the span sits
/// inside inline content, where a block annotation has no place.
#[allow(clippy::needless_range_loop)] // indexing keeps the returned child slice borrow valid
fn block_sequence(seq: &mut [RExpr], span: TextRange) -> Result<&mut [RExpr], ()> {
    for index in 0..seq.len() {
        if contains(&seq[index], span) {
            if !children_are_block(&seq[index]) {
                return Err(());
            }
            return block_sequence(&mut seq[index].children, span);
        }
    }
    Ok(seq)
}

fn attach_inline(forest: &mut [RExpr], group: &Group, diagnostics: &mut Vec<Diagnostic>) {
    if let Some(target) = find_inline(forest, group.end) {
        target.attrs.extend(group.dict.clone());
    } else {
        diagnostics.push(Diagnostic::new(
            Phase::Semantic,
            group.span,
            "inline annotation must be immediately followed by an element",
        ));
    }
}

/// Find the content element whose span starts exactly at `pos`.
///
/// Containers that start at the same position (a paragraph and its first
/// child) are descended into, so the annotation lands on the element itself,
/// not on the generated paragraph.
fn find_inline(seq: &mut [RExpr], pos: TextSize) -> Option<&mut RExpr> {
    for expr in seq.iter_mut() {
        let span = expr.span();
        if span.end() <= pos {
            continue;
        }
        if span.start() > pos {
            return None;
        }
        if span.start() == pos && is_content_element(expr) {
            return Some(expr);
        }
        if let Some(target) = find_inline(&mut expr.children, pos) {
            return Some(target);
        }
        if span.start() == pos {
            return None;
        }
    }
    None
}

fn is_content_element(expr: &RExpr) -> bool {
    expr.pending.is_some() && !matches!(expr.ctor, Ctor::Text | Ctor::Interpolation)
}

fn contains(expr: &RExpr, span: TextRange) -> bool {
    let outer = expr.span();
    outer.start() <= span.start() && span.end() <= outer.end()
}

fn children_are_block(expr: &RExpr) -> bool {
    if !expr.children.is_empty() {
        return expr.children.iter().any(is_block_element);
    }
    matches!(
        expr.ctor,
        Ctor::Section
            | Ctor::List
            | Ctor::Table
            | Ctor::TableRow
            | Ctor::TableCell
            | Ctor::ListItem
    ) || matches!(expr.body, BodyFlavor::Block)
}
