//! Materialize a shaped content forest: attach annotations and build Items.
//! This stage preserves the final structure and never resolves or evaluates calls.

use notist_core::annotation::{AnnotationKind, ResolvedAnnotation as Annotation};
use notist_core::diag::{Diagnostic, Phase};
use notist_core::expr::{BodyFlavor, RExpr};
use notist_core::item::{Ctor, Item, NodeContent, Origin};
use notist_core::shared::Text;
use rowan::{TextRange, TextSize};

use crate::shape::is_block_element;

/// Attach source-positioned annotations, report invalid targets, and generate
/// the Item forest with source origins. The shaped IR retains the attached
/// attributes for inspection; neither attachment nor conversion changes its structure.
pub fn materialize(
    forest: &mut [RExpr],
    annotations: &[Annotation],
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Item> {
    attach_annotations(forest, annotations, diagnostics);
    let source: Text = source.into();
    forest
        .iter()
        .map(|expr| build_item(expr, &source))
        .collect()
}

fn build_item(expr: &RExpr, source: &Text) -> Item {
    let mut item = Item::new(expr.ctor.clone(), expr.span);
    let children = expr
        .children
        .iter()
        .map(|child| build_item(child, source))
        .collect::<Vec<_>>();
    if let Some(notist_core::evaluation::PendingEvaluation::Failure { message, .. }) =
        expr.pending.as_deref()
    {
        // Keep damaged source structure available to inspection and language
        // services. Error content does not execute its children; evaluation
        // exports a leaf error at this boundary.
        let data = item.as_item_mut().expect("new content node");
        data.ctor = Ctor::Error(message.clone());
        data.fields = expr.fields.clone();
        data.children = children;
    } else if let Some(pending) = expr.pending.as_deref() {
        use notist_core::evaluation::{ExprKind, PendingEvaluation, TypedExpr};
        let value = match pending {
            PendingEvaluation::Call(call) => {
                let mut call = call.clone();
                if call.definition.children_type.is_some() || !children.is_empty() {
                    call.children = Some(Box::new(TypedExpr {
                        ty: call.definition.children_type.clone(),
                        known: None,
                        shape: notist_core::evaluation::ContentShape::of_items(&children),
                        kind: ExprKind::Content(children),
                        span: expr.span,
                        source: source.to_string(),
                    }));
                }
                let ty = Some(call.definition.returns.clone());
                let kind = ExprKind::Call(Box::new(call));
                let shape = notist_core::evaluation::ContentShape::of_expr(&kind);
                TypedExpr {
                    ty,
                    known: None,
                    kind,
                    span: expr.span,
                    source: source.to_string(),
                    shape,
                }
            }
            PendingEvaluation::Splice(value) | PendingEvaluation::Repeat(value) => value.clone(),
            PendingEvaluation::Failure { ty, message } => TypedExpr {
                ty: ty.clone(),
                known: None,
                kind: ExprKind::Failure(message.clone()),
                span: expr.span,
                source: source.to_string(),
                shape: Default::default(),
            },
        };
        item.content = NodeContent::Expr(Box::new(value));
    } else {
        let data = item.as_item_mut().expect("new content node");
        data.instance = expr.instance.clone().map(Into::into);
        data.fields = expr.fields.clone();
        data.children = children;
    }
    item.attrs = expr.attrs.clone();
    item.level = if is_block_element(expr) {
        notist_core::builtins::Level::Block
    } else {
        notist_core::builtins::Level::Inline
    };
    item.origin = Some(Origin {
        source: source.clone(),
        start: expr.span.start().into(),
        end: expr.span.end().into(),
        expansion: Default::default(),
    });
    item
}

struct Group {
    kind: AnnotationKind,
    dict: notist_core::item::Dict,
    span: TextRange,
    end: TextSize,
}

fn attach_annotations(
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
    if expr.ctor == Ctor::Interpolation {
        return matches!(
            expr.pending.as_deref(),
            Some(notist_core::evaluation::PendingEvaluation::Splice(value))
                if value.ty.as_ref().and_then(notist_core::definitions::Type::level).is_some()
        );
    }
    expr.pending.is_some() && expr.ctor != Ctor::Text
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

#[cfg(test)]
mod source_tests {
    use super::*;

    #[test]
    fn one_materialization_shares_its_path_across_all_boundaries() {
        let span = TextRange::new(0.into(), 3.into());
        let mut forest = [RExpr::new(Ctor::Paragraph, span).with_children(vec![
            RExpr::new(Ctor::Text, span),
            RExpr::new(Ctor::Emph, span),
        ])];
        let mut diagnostics = Vec::new();
        let items = materialize(&mut forest, &[], "docs/source.not", &mut diagnostics);
        assert!(diagnostics.is_empty());
        let root = items[0].origin.as_ref().unwrap();
        for child in items[0].children() {
            let origin = child.origin.as_ref().unwrap();
            assert_eq!(origin.source, "docs/source.not");
            assert!(root.source.shares_storage(&origin.source));
            assert_eq!((origin.start, origin.end), (0, 3));
        }
    }
}
