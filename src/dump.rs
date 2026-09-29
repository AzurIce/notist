use std::fmt::Write;

use crate::item::Item;

pub fn dump(item: &Item) -> String {
    let mut out = String::new();
    write_item(&mut out, item, 0);
    out
}

fn write_item(out: &mut String, item: &Item, indent: usize) {
    let pad = "  ".repeat(indent);
    let mut line = format!(
        "{pad}({} @{}..{}",
        item.ctor.name().to_lowercase(),
        u32::from(item.span.start()),
        u32::from(item.span.end()),
    );
    for (key, value) in item.fields.iter() {
        write!(line, " :{key} {value}").unwrap();
    }
    if item.children.is_empty() {
        line.push(')');
        out.push_str(&line);
        out.push('\n');
    } else {
        out.push_str(&line);
        out.push('\n');
        for child in &item.children {
            write_item(out, child, indent + 1);
        }
        out.push_str(&pad);
        out.push_str(")\n");
    }
}
