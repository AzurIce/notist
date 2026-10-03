use crate::expr::Expr;
use crate::item::Value;

/// Group a block sequence into sections by heading level: a `heading` call
/// starts a section; everything up to the next heading of equal or higher
/// level belongs to it; deeper headings nest. Content before the first
/// heading stays at this level. The heading stays the first child of its
/// section, and the section borrows the heading's span. Sections are
/// structural calls (`BodyFlavor::None`), like paragraph candidates.
pub fn sectionize(forest: Vec<Expr>) -> Vec<Expr> {
    let mut root: Vec<Expr> = Vec::new();
    let mut stack: Vec<(i64, Vec<Expr>)> = Vec::new();
    for expr in forest {
        if let Some(level) = heading_level(&expr) {
            while let Some(&(top, _)) = stack.last() {
                if top < level {
                    break;
                }
                close_section(&mut stack, &mut root);
            }
            stack.push((level, vec![expr]));
        } else if let Some((_, content)) = stack.last_mut() {
            content.push(expr);
        } else {
            root.push(expr);
        }
    }
    while !stack.is_empty() {
        close_section(&mut stack, &mut root);
    }
    root
}

fn close_section(stack: &mut Vec<(i64, Vec<Expr>)>, root: &mut Vec<Expr>) {
    let (_, children) = stack.pop().unwrap();
    let span = children.first().unwrap().span();
    let section = Expr::call("section", span).with_children(children);
    match stack.last_mut() {
        Some((_, parent)) => parent.push(section),
        None => root.push(section),
    }
}

fn heading_level(expr: &Expr) -> Option<i64> {
    let Expr::Call { name, fields, .. } = expr else {
        return None;
    };
    if name != "heading" {
        return None;
    }
    match fields.get("level") {
        Some(Value::Int(n)) => Some(*n),
        _ => Some(1),
    }
}
