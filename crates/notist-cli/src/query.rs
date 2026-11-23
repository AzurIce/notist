//! Serialization and selector queries over evaluated `notc` content trees.

use notc::{AttrValue, Block, BlockKind, CustomNode, CustomValue, Inline, InlineKind, Span};
use serde_json::{Map, Value, json};

fn attrs_json(attrs: &notc::Attrs) -> Value {
    let mut map = Map::new();
    for (k, v) in attrs.entries() {
        let jv = match v {
            AttrValue::Bool(b) => Value::Bool(*b),
            AttrValue::Int(n) => json!(n),
            AttrValue::Float(f) => json!(f),
            AttrValue::String(s) => Value::String(s.to_string()),
        };
        map.insert(k.to_string(), jv);
    }
    Value::Object(map)
}

fn span_json(span: Span) -> Value {
    json!({ "start": span.start, "end": span.end })
}

fn source_slice(source: &str, span: Span) -> &str {
    let start = span.start as usize;
    let end = span.end as usize;
    if start <= end && end <= source.len() && source.is_char_boundary(start) && source.is_char_boundary(end) {
        &source[start..end]
    } else {
        ""
    }
}

fn custom_value_json(value: &CustomValue, source: &str) -> Value {
    match value {
        CustomValue::Unit => Value::Null,
        CustomValue::Bool(b) => Value::Bool(*b),
        CustomValue::Int(n) => json!(n),
        CustomValue::Float(f) => json!(f),
        CustomValue::String(s) => Value::String(s.to_string()),
        CustomValue::Inline(inline) => serialize_inline(inline, source),
        CustomValue::Block(block) => serialize_block(block, source),
    }
}

fn custom_props_json(custom: &CustomNode, source: &str) -> Value {
    let mut map = Map::new();
    for (k, v) in custom.props.iter() {
        map.insert(k.to_string(), custom_value_json(v, source));
    }
    Value::Object(map)
}

pub fn inline_ctor_name(kind: &InlineKind) -> &str {
    match kind {
        InlineKind::Fragment(_) => "fragment",
        InlineKind::Text(_) => "text",
        InlineKind::Strong(_) => "strong",
        InlineKind::Emph(_) => "emph",
        InlineKind::Raw(_) => "raw",
        InlineKind::Math(_) => "math",
        InlineKind::Link { .. } => "link",
        InlineKind::Image { .. } => "image",
        InlineKind::Custom(custom) => custom.name.as_ref(),
    }
}

pub fn block_ctor_name(kind: &BlockKind) -> &str {
    match kind {
        BlockKind::Fragment(_) => "fragment",
        BlockKind::Paragraph(_) => "paragraph",
        BlockKind::Heading { .. } => "heading",
        BlockKind::Callout { .. } => "callout",
        BlockKind::CodeBlock { .. } => "code_block",
        BlockKind::Math(_) => "math",
        BlockKind::List { .. } => "list",
        BlockKind::Quote(_) => "quote",
        BlockKind::Divider => "divider",
        BlockKind::Table { .. } => "table",
        BlockKind::Div(_) => "div",
        BlockKind::SectionBox(_) => "section",
        BlockKind::Section { .. } => "section",
        BlockKind::Custom(custom) => custom.name.as_ref(),
    }
}

pub fn serialize_inline(inline: &Inline, source: &str) -> Value {
    let node = inline.node();
    let mut obj = Map::new();
    obj.insert("ctor".into(), json!(inline_ctor_name(&node.kind)));
    obj.insert("level".into(), json!("inline"));
    obj.insert("attrs".into(), attrs_json(&node.attrs));
    obj.insert("span".into(), span_json(node.span));
    obj.insert("text".into(), json!(source_slice(source, node.span)));
    match &node.kind {
        InlineKind::Fragment(items) => {
            obj.insert(
                "children".into(),
                Value::Array(items.iter().map(|i| serialize_inline(i, source)).collect()),
            );
        }
        InlineKind::Text(t) => {
            obj.insert("value".into(), json!(t.as_ref()));
        }
        InlineKind::Strong(inner) | InlineKind::Emph(inner) => {
            obj.insert("children".into(), Value::Array(vec![serialize_inline(inner, source)]));
        }
        InlineKind::Raw(code) | InlineKind::Math(code) => {
            obj.insert("value".into(), json!(code.as_ref()));
        }
        InlineKind::Link { target, label } => {
            obj.insert("target".into(), json!(target.as_ref()));
            obj.insert("children".into(), Value::Array(vec![serialize_inline(label, source)]));
        }
        InlineKind::Image { target, alt } => {
            obj.insert("target".into(), json!(target.as_ref()));
            obj.insert("alt".into(), json!(alt.as_ref()));
        }
        InlineKind::Custom(custom) => {
            obj.insert("module".into(), json!(custom.module.as_ref()));
            obj.insert("function".into(), json!(format!("{}::{}", custom.module, custom.name)));
            obj.insert("props".into(), custom_props_json(custom, source));
        }
    }
    Value::Object(obj)
}

pub fn serialize_block(block: &Block, source: &str) -> Value {
    let node = block.node();
    let mut obj = Map::new();
    obj.insert("ctor".into(), json!(block_ctor_name(&node.kind)));
    obj.insert("level".into(), json!("block"));
    obj.insert("attrs".into(), attrs_json(&node.attrs));
    obj.insert("span".into(), span_json(node.span));
    obj.insert("text".into(), json!(source_slice(source, node.span)));
    match &node.kind {
        BlockKind::Fragment(items) => {
            obj.insert(
                "children".into(),
                Value::Array(items.iter().map(|b| serialize_block(b, source)).collect()),
            );
        }
        BlockKind::Paragraph(inline) => {
            obj.insert("children".into(), Value::Array(vec![serialize_inline(inline, source)]));
        }
        BlockKind::Heading { level, title } => {
            obj.insert("heading_level".into(), json!(level));
            obj.insert("children".into(), Value::Array(vec![serialize_inline(title, source)]));
        }
        BlockKind::Callout { kind, body } => {
            obj.insert("kind".into(), json!(kind.as_ref()));
            obj.insert("children".into(), Value::Array(vec![serialize_block(body, source)]));
        }
        BlockKind::CodeBlock { lang, text } => {
            obj.insert("lang".into(), json!(lang.as_ref()));
            obj.insert("value".into(), json!(text.as_ref()));
        }
        BlockKind::Math(expr) => {
            obj.insert("value".into(), json!(expr.as_ref()));
        }
        BlockKind::List { ordered, items } => {
            obj.insert("ordered".into(), json!(ordered));
            obj.insert(
                "children".into(),
                Value::Array(items.iter().map(|b| serialize_block(b, source)).collect()),
            );
        }
        BlockKind::Quote(body) | BlockKind::Div(body) | BlockKind::SectionBox(body) => {
            obj.insert("children".into(), Value::Array(vec![serialize_block(body, source)]));
        }
        BlockKind::Divider => {}
        BlockKind::Table { headers, rows } => {
            obj.insert(
                "headers".into(),
                Value::Array(headers.iter().map(|c| serialize_inline(c, source)).collect()),
            );
            obj.insert(
                "rows".into(),
                Value::Array(
                    rows.iter()
                        .map(|r| Value::Array(r.iter().map(|c| serialize_inline(c, source)).collect()))
                        .collect(),
                ),
            );
        }
        BlockKind::Section { heading, body } => {
            obj.insert("heading".into(), serialize_block(heading, source));
            obj.insert("children".into(), Value::Array(vec![serialize_block(body, source)]));
        }
        BlockKind::Custom(custom) => {
            obj.insert("module".into(), json!(custom.module.as_ref()));
            obj.insert("function".into(), json!(format!("{}::{}", custom.module, custom.name)));
            obj.insert("props".into(), custom_props_json(custom, source));
        }
    }
    Value::Object(obj)
}

fn matches_attr_id(attrs: &notc::Attrs, needle: &str) -> bool {
    matches!(attrs.get("id"), Some(AttrValue::String(id)) if id.as_ref() == needle)
}

fn matches_attr_tag(attrs: &notc::Attrs, needle: &str) -> bool {
    match attrs.get("tags").or_else(|| attrs.get("tag")) {
        Some(AttrValue::String(s)) => s.split(',').map(str::trim).any(|t| t == needle),
        _ => false,
    }
}

fn matches_custom(custom: &CustomNode, needle: &str) -> bool {
    custom.name.as_ref() == needle || format!("{}::{}", custom.module, custom.name) == needle
}

fn collect_inline_matches(
    inline: &Inline,
    kind: &str,
    needle: &str,
    source: &str,
    out: &mut Vec<Value>,
) {
    let node = inline.node();
    let matched = match kind {
        "id" => matches_attr_id(&node.attrs, needle),
        "tag" => matches_attr_tag(&node.attrs, needle),
        "ctor" => match &node.kind {
            InlineKind::Custom(c) => matches_custom(c, needle),
            other => inline_ctor_name(other).eq_ignore_ascii_case(needle),
        },
        "function" => matches!(&node.kind, InlineKind::Custom(c) if matches_custom(c, needle)),
        "level" => needle.eq_ignore_ascii_case("inline"),
        _ => false,
    };
    if matched {
        out.push(serialize_inline(inline, source));
    }
    match &node.kind {
        InlineKind::Fragment(items) => {
            for item in items.iter() {
                collect_inline_matches(item, kind, needle, source, out);
            }
        }
        InlineKind::Strong(inner) | InlineKind::Emph(inner) => {
            collect_inline_matches(inner, kind, needle, source, out);
        }
        InlineKind::Link { label, .. } => {
            collect_inline_matches(label, kind, needle, source, out);
        }
        InlineKind::Custom(custom) => {
            for (_, v) in custom.props.iter() {
                match v {
                    CustomValue::Inline(inl) => {
                        collect_inline_matches(inl, kind, needle, source, out)
                    }
                    CustomValue::Block(blk) => {
                        collect_block_matches(blk, kind, needle, source, out)
                    }
                    _ => {}
                }
            }
        }
        InlineKind::Text(_) | InlineKind::Raw(_) | InlineKind::Math(_) | InlineKind::Image { .. } => {}
    }
}

fn collect_block_matches(
    block: &Block,
    kind: &str,
    needle: &str,
    source: &str,
    out: &mut Vec<Value>,
) {
    let node = block.node();
    let matched = match kind {
        "id" => matches_attr_id(&node.attrs, needle),
        "tag" => matches_attr_tag(&node.attrs, needle),
        "ctor" => match &node.kind {
            BlockKind::Custom(c) => matches_custom(c, needle),
            other => block_ctor_name(other).eq_ignore_ascii_case(needle),
        },
        "function" => matches!(&node.kind, BlockKind::Custom(c) if matches_custom(c, needle)),
        "level" => needle.eq_ignore_ascii_case("block"),
        _ => false,
    };
    if matched {
        out.push(serialize_block(block, source));
    }
    match &node.kind {
        BlockKind::Fragment(items) | BlockKind::List { items, .. } => {
            for item in items.iter() {
                collect_block_matches(item, kind, needle, source, out);
            }
        }
        BlockKind::Paragraph(inline) | BlockKind::Heading { title: inline, .. } => {
            collect_inline_matches(inline, kind, needle, source, out);
        }
        BlockKind::Callout { body, .. }
        | BlockKind::Quote(body)
        | BlockKind::Div(body)
        | BlockKind::SectionBox(body) => {
            collect_block_matches(body, kind, needle, source, out);
        }
        BlockKind::Table { headers, rows } => {
            for cell in headers.iter() {
                collect_inline_matches(cell, kind, needle, source, out);
            }
            for row in rows.iter() {
                for cell in row.iter() {
                    collect_inline_matches(cell, kind, needle, source, out);
                }
            }
        }
        BlockKind::Section { heading, body } => {
            collect_block_matches(heading, kind, needle, source, out);
            collect_block_matches(body, kind, needle, source, out);
        }
        BlockKind::Custom(custom) => {
            for (_, v) in custom.props.iter() {
                match v {
                    CustomValue::Inline(inl) => {
                        collect_inline_matches(inl, kind, needle, source, out)
                    }
                    CustomValue::Block(blk) => {
                        collect_block_matches(blk, kind, needle, source, out)
                    }
                    _ => {}
                }
            }
        }
        BlockKind::CodeBlock { .. } | BlockKind::Math(_) | BlockKind::Divider => {}
    }
}

/// Select descendants of `block` using an `id:` / `tag:` / `ctor:` / `function:` / `level:`
/// selector and render matches as a JSON array.
pub fn select_json(block: &Block, selector: &str, source: &str) -> Result<String, serde_json::Error> {
    let mut matches = Vec::new();
    if let Some((kind, needle)) = selector.split_once(':') {
        collect_block_matches(block, kind, needle, source, &mut matches);
    }
    serde_json::to_string(&matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_selects_by_id_tag_ctor_and_level_on_notc_block() {
        let source = "@(id: \"x\", tags: \"a, b\")_目标_\n\n*粗*\n";
        let project = notc::check_project(&[("test.not", source)]).expect("checks");
        let block = project.eval_document("test.not").expect("evaluates");

        let by_id: Value = serde_json::from_str(&select_json(&block, "id:x", source).unwrap()).unwrap();
        assert_eq!(by_id.as_array().unwrap().len(), 1);
        assert_eq!(by_id[0]["ctor"], "emph");
        assert_eq!(by_id[0]["attrs"]["tags"], "a, b");
        assert!(by_id[0]["text"].as_str().unwrap().contains("目标"));

        let by_tag: Value = serde_json::from_str(&select_json(&block, "tag:b", source).unwrap()).unwrap();
        assert_eq!(by_tag.as_array().unwrap().len(), 1);
        assert_eq!(by_tag[0]["ctor"], "emph");

        let by_ctor: Value = serde_json::from_str(&select_json(&block, "ctor:strong", source).unwrap()).unwrap();
        assert_eq!(by_ctor.as_array().unwrap().len(), 1);
        assert_eq!(by_ctor[0]["level"], "inline");
    }

    #[test]
    fn output_serialization_handles_deep_notc_blocks() {
        let mut block = Block::empty(Span::DUMMY);
        for _ in 0..80 {
            block = Block::new(BlockKind::Div(block), notc::Attrs::empty(), Span::DUMMY);
        }
        let json = serde_json::to_string(&serialize_block(&block, "")).unwrap();
        assert!(json.contains("\"ctor\":\"div\""));
    }
}
