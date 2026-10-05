use notist::{Ctor, Item, Value};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize)]
pub struct Heading {
    pub title: String,
    pub id: String,
    pub level: i64,
}
pub(crate) struct Outline {
    pub title: String,
    pub headings: Vec<Heading>,
    pub ids: BTreeSet<String>,
}
pub(crate) fn text(item: &Item) -> String {
    let mut result = String::new();
    for node in item.descendants() {
        if matches!(node.ctor, Ctor::Text | Ctor::RawInline | Ctor::Math)
            && let Some(Value::Str(value)) = node.fields.get("text")
        {
            result.push_str(value);
        }
    }
    result
}
fn slug(title: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for ch in title.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() || ch == '_' {
            if separator && !result.is_empty() {
                result.push('-');
            }
            result.push(ch);
            separator = false;
        } else {
            separator = true;
        }
    }
    if result.is_empty() {
        "section".into()
    } else {
        result
    }
}
pub(crate) fn prepare(root: &mut Item, fallback: &str) -> Outline {
    let ids = root
        .descendants()
        .filter_map(|node| match node.attrs.get("id") {
            Some(Value::Str(id)) => Some(id.clone()),
            _ => None,
        })
        .collect();
    let mut outline = Outline {
        title: fallback.into(),
        headings: vec![],
        ids,
    };
    fn visit(node: &mut Item, inherited: Option<String>, outline: &mut Outline) {
        if node.ctor == Ctor::Heading {
            let title = text(node);
            if outline.headings.is_empty() {
                outline.title = title.clone();
            }
            let id = match node.attrs.get("id") {
                Some(Value::Str(id)) => Some(id.clone()),
                _ => inherited,
            };
            let id = id.unwrap_or_else(|| {
                let base = slug(&title);
                let mut value = base.clone();
                let mut n = 2;
                while outline.ids.contains(&value) {
                    value = format!("{base}-{n}");
                    n += 1;
                }
                outline.ids.insert(value.clone());
                node.attrs.insert("id", Value::Str(value.clone()));
                value
            });
            let level = match node.fields.get("level") {
                Some(Value::Int(level)) => *level,
                _ => 1,
            };
            outline.headings.push(Heading { title, id, level });
        }
        let section_id = if node.ctor == Ctor::Section {
            match node.attrs.get("id") {
                Some(Value::Str(id)) => Some(id.clone()),
                _ => None,
            }
        } else {
            None
        };
        for (index, child) in node.children.iter_mut().enumerate() {
            visit(
                child,
                if index == 0 { section_id.clone() } else { None },
                outline,
            );
        }
    }
    visit(root, None, &mut outline);
    outline
}
