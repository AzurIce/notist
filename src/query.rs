use crate::item::{Item, Value};

/// Select descendants of `item` by an `id:` / `tag:` / `ctor:` selector.
pub fn select<'a>(
    item: &'a Item,
    selector: &str,
    sources: &crate::Sources,
) -> Result<Vec<&'a Item>, notist_core::definition_identity::DefinitionError> {
    let Some((kind, needle)) = selector.split_once(':') else {
        return Ok(Vec::new());
    };
    if kind == "function" {
        let mut result = Vec::new();
        for item in item.descendants() {
            if let Some(name) = item.ctor().function_name(sources.definitions())? {
                if name.to_string() == needle {
                    result.push(item);
                }
            }
        }
        return Ok(result);
    }
    if kind == "ctor" {
        let mut result = Vec::new();
        for item in item.descendants() {
            let ctor = item.ctor();
            let name = ctor.name(sources.definitions())?;
            if if matches!(ctor, crate::Ctor::Extension(_)) {
                name == needle
            } else {
                name.eq_ignore_ascii_case(needle)
            } {
                result.push(item);
            }
        }
        return Ok(result);
    }
    Ok(item
        .descendants()
        .filter(|item| match kind {
            "id" => matches!(item.attrs.get("id"), Some(Value::Str(id)) if id == needle),
            "tag" => item.attrs.get("tags").is_some_and(|tags| match tags {
                Value::Array(tags) => tags
                    .iter()
                    .any(|t| matches!(t, Value::Str(t) if t == needle)),
                _ => false,
            }),
            "level" => match item.level {
                crate::builtins::Level::Block => needle == "block",
                crate::builtins::Level::Inline => needle == "inline",
                crate::builtins::Level::Inherit => false,
            },
            _ => false,
        })
        .collect())
}
