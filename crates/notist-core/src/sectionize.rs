use crate::item::{Ctor, Item, Value};

/// Group a document's top-level items into a section tree by heading level:
/// a heading starts a section; everything up to the next heading of equal or
/// higher level belongs to it; deeper headings nest. Content before the first
/// heading stays at the document level. The heading stays the first child of
/// its section.
pub fn sectionize(doc: &Item) -> Item {
    let mut root = Item::new(Ctor::Doc, doc.span);
    root.fields = doc.fields.clone();
    root.attrs = doc.attrs.clone();

    let mut stack: Vec<(i64, Item)> = Vec::new();
    for child in &doc.children {
        if child.ctor == Ctor::Heading {
            let level = match child.fields.get("level") {
                Some(Value::Int(n)) => *n,
                _ => 1,
            };
            while let Some(&(top, _)) = stack.last() {
                if top < level {
                    break;
                }
                let (_, section) = stack.pop().unwrap();
                match stack.last_mut() {
                    Some((_, parent)) => parent.children.push(section),
                    None => root.children.push(section),
                }
            }
            let mut section = Item::new(Ctor::Section, child.span);
            section.children.push(child.clone());
            stack.push((level, section));
        } else if let Some((_, section)) = stack.last_mut() {
            section.children.push(child.clone());
        } else {
            root.children.push(child.clone());
        }
    }
    while let Some((_, section)) = stack.pop() {
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(section),
            None => root.children.push(section),
        }
    }
    root
}
