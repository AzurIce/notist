use notist_core::expr::RExpr;
use notist_core::item::{Ctor, Value};
use rowan::TextRange;

/// Group a block sequence into sections by heading level: a heading starts a
/// section; everything up to the next heading of equal or higher level
/// belongs to it; deeper headings nest. Content before the first heading
/// stays at this level. The heading stays the first child of its section, and
/// the span covers the whole section extent. Annotations are attached later,
/// so no attrs move here.
pub fn sectionize(forest: Vec<RExpr>) -> Vec<RExpr> {
    let mut root: Vec<RExpr> = Vec::new();
    let mut stack: Vec<(i64, Vec<RExpr>)> = Vec::new();
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

fn close_section(stack: &mut Vec<(i64, Vec<RExpr>)>, root: &mut Vec<RExpr>) {
    let (_, children) = stack.pop().unwrap();
    let start = children.first().unwrap().span().start();
    let end = children.last().unwrap().span().end();
    let section = RExpr::new(Ctor::Section, TextRange::new(start, end)).with_children(children);
    match stack.last_mut() {
        Some((_, parent)) => parent.push(section),
        None => root.push(section),
    }
}

fn heading_level(expr: &RExpr) -> Option<i64> {
    if expr.ctor != Ctor::Heading {
        return None;
    }
    match expr.fields.get("level") {
        Some(Value::Int(n)) => Some(*n),
        _ => Some(1),
    }
}
