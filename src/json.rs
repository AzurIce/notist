//! Common IR JSON representation, independent of command output.
use crate::{Item, Value};

pub(crate) fn value(value: &Value) -> serde_json::Value {
    match value {
        Value::Unit => serde_json::Value::Null,
        Value::Bool(value) => (*value).into(),
        Value::Int(value) => (*value).into(),
        Value::Float(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Str(value) => value.to_string().into(),
        Value::Path(path) => serde_json::json!({"$path":{"root":path.root(),"path":path.path()}}),
        Value::Data(data) => {
            serde_json::json!({"$data":{"kind":data.kind(),"nodes":data.node_count(),"bytes":data.logical_size()}})
        }
        Value::DataError(error) => {
            serde_json::json!({"$data_error":{"kind":error.kind().as_str()}})
        }
        Value::Array(values) => serde_json::Value::Array(values.iter().map(self::value).collect()),
        Value::Dict(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.to_owned(), self::value(value)))
                .collect(),
        ),
        Value::Some(value) => self::value(value),
        Value::None => serde_json::Value::Null,
        Value::Ok(v) => serde_json::json!({"ok":self::value(v)}),
        Value::Err(v) => serde_json::json!({"err":self::value(v)}),
    }
}

fn instance_json(
    i: Option<&notist_core::definitions::ResolvedInstance>,
    sources: &crate::Sources,
) -> Result<serde_json::Value, serde_json::Error> {
    let Some(i) = i else {
        return Ok(serde_json::Value::Null);
    };
    let name = sources
        .definitions()
        .function(i.function)
        .map_err(<serde_json::Error as serde::ser::Error>::custom)?;
    Ok(
        serde_json::json!({"function":name.to_string(),"comptime":i.arguments.iter().map(|v|match v{notist_core::definitions::ComptimeValue::Type(t)=>serde_json::json!({"type":t.to_string()}),notist_core::definitions::ComptimeValue::Data(v)=>notist_html::components::value_json(v)}).collect::<Vec<_>>()}),
    )
}

pub(crate) fn metadata(
    item: &Item,
    sources: &crate::Sources,
) -> Result<serde_json::Value, serde_json::Error> {
    let expression = serde_json::to_value(sources.wire(&item.as_expr()))?;
    let origin = serde_json::to_value(sources.wire(&item.origin))?;
    Ok(serde_json::json!({
        "expression": expression,
        "function": item.ctor().function_name(sources.definitions()).map_err(<serde_json::Error as serde::ser::Error>::custom)?.map(ToString::to_string),
        "instance":instance_json(item.instance(), sources)?,
        "error": match item.ctor() { crate::Ctor::Error(message) => Some(message), _ => None },
        "origin":origin,
        "level": if item.level == crate::builtins::Level::Block { "block" } else { "inline" },
        "fields": value(&Value::Dict(item.fields())),
        "typed_fields": notist_html::components::value_json(&Value::Dict(item.fields())),
        "typed_attrs": notist_html::components::value_json(&Value::Dict(item.attrs.clone())),
    }))
}

/// Serialize one final IR node with its identity, byte range, metadata and
/// descendants. Typed values preserve numeric bits and dictionary order.
pub fn item(item: &Item, sources: &crate::Sources) -> Result<serde_json::Value, serde_json::Error> {
    let mut object = metadata(item, sources)?;
    object["ctor"] = item
        .ctor()
        .name(sources.definitions())
        .map_err(<serde_json::Error as serde::ser::Error>::custom)?
        .as_ref()
        .into();
    object["start"] = u32::from(item.span.start()).into();
    object["end"] = u32::from(item.span.end()).into();
    object["attrs"] = value(&Value::Dict(item.attrs.clone()));
    object["children"] = serde_json::Value::Array(
        item.children()
            .iter()
            .map(|node| self::item(node, sources))
            .collect::<Result<_, _>>()?,
    );
    Ok(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_tags_survive_debug_json_serialization() {
        for (value, expected) in [
            (
                Value::Ok(Box::new(Value::Some(Box::new(Value::Str("\"\\\n".into()))))),
                serde_json::json!({"ok":"\"\\\n"}),
            ),
            (
                Value::Err(Box::new(Value::Ok(Box::new(Value::Int(i64::MAX))))),
                serde_json::json!({"err":{"ok":i64::MAX}}),
            ),
        ] {
            assert_eq!(super::value(&value), expected);
            let out = super::value(&value).to_string();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&out).unwrap(),
                expected
            );
        }
    }
}
