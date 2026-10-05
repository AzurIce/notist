use crate::item::{Item, Value};

/// Select descendants of `item` by an `id:` / `tag:` / `ctor:` selector.
pub fn select<'a>(item: &'a Item, selector: &str) -> Vec<&'a Item> {
    let Some((kind, needle)) = selector.split_once(':') else {
        return Vec::new();
    };
    item.descendants()
        .filter(|item| match kind {
            "id" => matches!(item.attrs.get("id"), Some(Value::Str(id)) if id == needle),
            "tag" => item.attrs.get("tags").is_some_and(|tags| match tags {
                Value::Array(tags) => tags
                    .iter()
                    .any(|t| matches!(t, Value::Str(t) if t == needle)),
                _ => false,
            }),
            "ctor" if matches!(item.ctor, crate::Ctor::Extension(_)) => item.ctor.name() == needle,
            "ctor" => item.ctor.name().eq_ignore_ascii_case(needle),
            "function" => item
                .ctor
                .function_id()
                .is_some_and(|id| id.to_string() == needle),
            "level" => match item.level {
                crate::builtins::Level::Block => needle == "block",
                crate::builtins::Level::Inline => needle == "inline",
                crate::builtins::Level::Inherit => false,
            },
            _ => false,
        })
        .collect()
}
