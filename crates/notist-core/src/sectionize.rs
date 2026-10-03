use crate::expr::Expr;
use crate::item::Value;
use rowan::TextRange;

/// Group a block sequence into sections by heading level: a `heading` call
/// starts a section; everything up to the next heading of equal or higher
/// level belongs to it; deeper headings nest. Content before the first
/// heading stays at this level. The heading stays the first child of its
/// section. A section is the identity carrier of its range: its attrs are
/// transferred from the heading that opens it (a heading opens exactly one
/// section), and its span covers the whole section extent.
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
    let start = children.first().unwrap().span().start();
    let end = children.last().unwrap().span().end();
    let mut section = Expr::call("section", TextRange::new(start, end)).with_children(children);
    // identity transfer: the opening heading's attrs belong to the section
    if let Expr::Call {
        children, attrs, ..
    } = &mut section
    {
        if let Some(Expr::Call {
            attrs: heading_attrs,
            ..
        }) = children.first_mut()
        {
            *attrs = std::mem::take(heading_attrs);
        }
    }
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
