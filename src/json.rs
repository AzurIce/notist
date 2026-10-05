//! Common IR JSON representation, independent of command output.
use crate::{Item, Value};

pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(crate) fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Unit => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(x) => out.push_str(&if x.is_finite() {
            x.to_string()
        } else {
            "null".into()
        }),
        Value::Str(s) => out.push_str(&json_escape(s)),
        Value::Array(items) => {
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, v);
            }
            out.push(']');
        }
        Value::Dict(dict) => {
            out.push('{');
            for (i, (k, v)) in dict.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&json_escape(k));
                out.push(':');
                write_value(out, v);
            }
            out.push('}');
        }
    }
}

fn value(value: &Value) -> serde_json::Value {
    match value {
        Value::Unit => serde_json::Value::Null,
        Value::Bool(value) => (*value).into(),
        Value::Int(value) => (*value).into(),
        Value::Float(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Str(value) => value.clone().into(),
        Value::Array(values) => serde_json::Value::Array(values.iter().map(self::value).collect()),
        Value::Dict(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.to_owned(), self::value(value)))
                .collect(),
        ),
    }
}

fn metadata(item: &Item) -> serde_json::Value {
    serde_json::json!({
        "function": item.ctor.function_id().map(|id| id.to_string()),
        "level": if item.level == crate::builtins::Level::Block { "block" } else { "inline" },
        "fields": value(&Value::Dict(item.fields.clone())),
        "typed_fields": notist_html::components::value_json(&Value::Dict(item.fields.clone())),
        "typed_attrs": notist_html::components::value_json(&Value::Dict(item.attrs.clone())),
    })
}

pub(crate) fn write_metadata(out: &mut String, item: &Item) {
    let object = metadata(item).to_string();
    out.push_str(&object[1..object.len() - 1]);
}

/// Serialize one final IR node with its identity, byte range, metadata and
/// descendants. Typed values preserve numeric bits and dictionary order.
pub fn item(item: &Item) -> serde_json::Value {
    let mut object = metadata(item);
    object["ctor"] = item.ctor.name().as_ref().into();
    object["start"] = u32::from(item.span.start()).into();
    object["end"] = u32::from(item.span.end()).into();
    object["attrs"] = value(&Value::Dict(item.attrs.clone()));
    object["children"] = serde_json::Value::Array(item.children.iter().map(self::item).collect());
    object
}
