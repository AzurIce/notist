use std::fmt::Write;

use crate::item::Item;

pub fn dump(
    item: &Item,
    sources: &crate::source::Sources,
) -> Result<String, crate::definition_identity::DefinitionError> {
    let mut out = String::new();
    write_item(&mut out, item, 0, sources)?;
    Ok(out)
}

fn write_item(
    out: &mut String,
    item: &Item,
    indent: usize,
    sources: &crate::source::Sources,
) -> Result<(), crate::definition_identity::DefinitionError> {
    let pad = "  ".repeat(indent);
    let mut line = format!(
        "{pad}({} @{}..{}",
        if matches!(item.ctor(), crate::item::Ctor::Extension(_)) {
            item.ctor().name(sources.definitions())?.into_owned()
        } else {
            item.ctor().name(sources.definitions())?.to_lowercase()
        },
        u32::from(item.span.start()),
        u32::from(item.span.end()),
    );
    if matches!(item.ctor(), crate::item::Ctor::Extension(_)) {
        write!(
            line,
            " [{}]",
            if item.level == crate::builtins::Level::Block {
                "block"
            } else {
                "inline"
            }
        )
        .unwrap();
    }
    for (key, value) in item.fields().iter() {
        write!(line, " :{key} {value}").unwrap();
    }
    for (key, value) in item.attrs.iter() {
        write!(line, " @{key} {value}").unwrap();
    }
    if item.children().is_empty() {
        line.push(')');
        out.push_str(&line);
        out.push('\n');
    } else {
        out.push_str(&line);
        out.push('\n');
        for child in item.children() {
            write_item(out, child, indent + 1, sources)?;
        }
        out.push_str(&pad);
        out.push_str(")\n");
    }
    Ok(())
}
