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
                Value::Array(tags) => tags.iter().any(|t| matches!(t, Value::Str(t) if t == needle)),
                _ => false,
            }),
            "ctor" => item.ctor.name().eq_ignore_ascii_case(needle),
            _ => false,
        })
        .collect()
}

fn json_escape(s: &str) -> String {
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

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Unit => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(x) => out.push_str(&x.to_string()),
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

/// Render query matches as a JSON array.
pub fn render_json(src: &str, matches: &[&Item]) -> String {
    let mut out = String::from("[");
    for (i, item) in matches.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let start = u32::from(item.span.start());
        let end = u32::from(item.span.end());
        out.push_str(&format!(
            "{{\"ctor\":{},\"start\":{start},\"end\":{end},\"attrs\":{{",
            json_escape(&item.ctor.name().to_lowercase()),
        ));
        for (j, (k, v)) in item.attrs.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&json_escape(k));
            out.push(':');
            write_value(&mut out, v);
        }
        let text = &src[start as usize..end as usize];
        out.push_str(&format!("}},\"text\":{}}}", json_escape(text)));
    }
    out.push(']');
    out
}
