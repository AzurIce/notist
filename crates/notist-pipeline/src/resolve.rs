use crate::Context;
use notist_core::{
    builtins::Accepts,
    diag::{Diagnostic, Phase},
    expr::{Expr, RExpr},
    item::{Ctor, Value},
};
pub fn resolve(
    forest: Vec<Expr>,
    context: &Context,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<RExpr> {
    let mut checker = crate::code::Checker::new(context, "<content>");
    let result = checker.resolve(&forest);
    diagnostics.extend(checker.diagnostics);
    result
}
pub(crate) fn validate_structure(r: &mut RExpr, name: &str, diags: &mut Vec<Diagnostic>) {
    r.children = structural_children(std::mem::take(&mut r.children));
    match r.ctor.accepts() {
        Some(Accepts::Rows) => check_structural_children(&r.children, &Ctor::TableRow, name, diags),
        Some(Accepts::Cells) => {
            check_structural_children(&r.children, &Ctor::TableCell, name, diags)
        }
        Some(Accepts::Items) => {
            check_structural_children(&r.children, &Ctor::ListItem, name, diags)
        }
        _ => {}
    }
}
/// Source fragments have been removed on structural axes. Only transparent
/// node calls may group rows, cells or items without changing their role.
fn check_structural_children(
    children: &[RExpr],
    expected: &Ctor,
    parent: &str,
    diags: &mut Vec<Diagnostic>,
) {
    for child in children {
        match child {
            RExpr {
                ctor: Ctor::Node,
                children,
                ..
            } => {
                check_structural_children(children, expected, parent, diags);
            }
            RExpr { ctor, .. } if ctor == expected => {}
            _ => diags.push(Diagnostic::new(
                Phase::Type,
                child.span(),
                format!(
                    "`{parent}` takes only {} children",
                    if *expected == Ctor::TableRow {
                        "row"
                    } else if *expected == Ctor::TableCell {
                        "cell"
                    } else {
                        "item"
                    }
                ),
            )),
        }
    }
}

/// Whitespace between structural calls is a separator, not an element.
fn structural_children(children: Vec<RExpr>) -> Vec<RExpr> {
    children
        .into_iter()
        .filter_map(|mut child| {
            let RExpr {
                ctor,
                fields,
                attrs,
                children,
                ..
            } = &mut child;
            if *ctor == Ctor::Text
                && attrs.is_empty()
                && matches!(fields.get("text"), Some(Value::Str(text)) if text.trim().is_empty())
            {
                return None;
            }
            if *ctor == Ctor::Node {
                *children = structural_children(std::mem::take(children));
                if children.is_empty() && attrs.is_empty() && fields.is_empty() {
                    return None;
                }
            }
            Some(child)
        })
        .collect()
}
