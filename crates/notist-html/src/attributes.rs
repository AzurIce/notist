use notist_core::item::{Dict, Value};

use crate::escape;

pub(crate) fn has_html_attrs(attrs: &Dict) -> bool {
    attrs
        .iter()
        .any(|(key, value)| value_of(key, value).is_some())
}

pub(crate) fn has_title(attrs: &Dict) -> bool {
    matches!(attrs.get("title"), Some(Value::Str(_)))
}

pub(crate) fn write(output: &mut String, attrs: &Dict, base_class: &str) {
    let extra_class = attrs
        .get("class")
        .and_then(|value| value_of("class", value));
    let classes = [Some(base_class), extra_class.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !classes.is_empty() {
        attribute(output, "class", &classes);
    }
    for (key, value) in attrs.iter() {
        if key == "class" {
            continue;
        }
        if let Some(value) = value_of(key, value) {
            attribute(
                output,
                if key == "tags" {
                    "data-notist-tags"
                } else {
                    key
                },
                &value,
            );
        }
    }
}

pub(crate) fn attribute(output: &mut String, key: &str, value: &str) {
    output.push(' ');
    output.push_str(key);
    output.push_str("=\"");
    escape::write(output, value, true);
    output.push('"');
}

fn value_of(key: &str, value: &Value) -> Option<String> {
    if matches!(key, "class" | "tags") {
        return match value {
            Value::Str(s) if key == "class" => Some(s.clone()),
            Value::Array(values) => Some(
                values
                    .iter()
                    .filter_map(|v| {
                        if let Value::Str(s) = v {
                            Some(s.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        };
    }
    if matches!(key, "id" | "title" | "lang" | "dir") {
        return if let Value::Str(s) = value {
            Some(s.clone())
        } else {
            None
        };
    }
    // Source attrs are metadata, not arbitrary HTML attributes. In
    // particular, event handlers, style and resource URLs are not forwarded.
    if (key.starts_with("data-") && !key.starts_with("data-notist-"))
        || (key.starts_with("aria-") && key != "aria-level")
    {
        if !key
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return None;
        }
        return match value {
            Value::Str(s) => Some(s.clone()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Int(n) => Some(n.to_string()),
            Value::Float(n) if n.is_finite() => Some(n.to_string()),
            _ => None,
        };
    }
    None
}
