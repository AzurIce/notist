use crate::{Error, Route};
use notist::{Ctor, Diagnostic, Item, SourceDiagnostic, Sources, Value};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Serialize)]
pub struct Heading {
    pub title: String,
    pub id: String,
    pub anchor: String,
    pub level: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub route: Route,
    pub title: String,
    pub attrs: BTreeMap<String, serde_json::Value>,
    pub headings: Vec<Heading>,
    pub html: String,
    /// Content-relative directory; the root is the empty string.
    pub directory: String,
    pub ancestors: Vec<String>,
}

pub(crate) struct Document {
    pub page: Page,
    pub root: Item,
    pub source: String,
    pub sources: Sources,
    pub ids: BTreeSet<String>,
}

pub(crate) fn diagnostic(path: &Path, source: &str, d: Diagnostic) -> SourceDiagnostic {
    SourceDiagnostic::locate(path, source, d)
}

fn value(input: &Value) -> serde_json::Value {
    use serde_json::{Value as Json, json};
    match input {
        Value::Unit | Value::None => Json::Null,
        // Logical resource identities are not public template values or URLs.
        Value::Path(_) | Value::Data(_) | Value::DataError(_) => Json::Null,
        Value::Bool(v) => json!(v),
        Value::Int(v) => json!(v),
        Value::Float(v) => json!(v),
        Value::Str(v) => json!(v.as_str()),
        Value::Array(v) => Json::Array(v.iter().map(value).collect()),
        Value::Dict(v) => Json::Object(v.iter().map(|(k, v)| (k.into(), value(v))).collect()),
        Value::Some(v) => value(v),
        Value::Ok(v) => json!({"ok":value(v)}),
        Value::Err(v) => json!({"err":value(v)}),
    }
}

fn text(item: &Item) -> String {
    item.descendants()
        .filter(|n| matches!(n.ctor(), Ctor::Text | Ctor::RawInline | Ctor::Math))
        .filter_map(|n| match n.field("text") {
            Some(Value::Str(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

fn slug(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() || ch == '_' {
            result.push(ch);
        } else if !result.is_empty() && !result.ends_with('-') {
            result.push('-');
        }
    }
    let result = result.trim_end_matches('-');
    if result.is_empty() {
        "section".into()
    } else {
        result.into()
    }
}

fn headings(
    item: &mut Item,
    inherited: Option<String>,
    ids: &mut BTreeSet<String>,
    out: &mut Vec<Heading>,
) {
    if item.is_error() {
        return;
    }
    if item.ctor() == Ctor::Heading {
        let title = text(item);
        let inherited_target = item.attrs.get("id").is_none() && inherited.is_some();
        let explicit = match item.attrs.get("id") {
            Some(Value::Str(id)) => Some(id.to_string()),
            _ => inherited,
        };
        let id = explicit.unwrap_or_else(|| {
            let base = slug(&title);
            let mut candidate = base.clone();
            let mut n = 2;
            while !ids.insert(candidate.clone()) {
                candidate = format!("{base}-{n}");
                n += 1;
            }
            candidate
        });
        // A section's explicit ID already provides the heading's link target.
        if !inherited_target {
            item.attrs.insert("id", Value::Str(id.clone().into()));
        }
        let level = match item.field("level") {
            Some(Value::Int(n)) => *n,
            _ => 1,
        };
        out.push(Heading {
            title,
            anchor: notist::publication::url::fragment(&id),
            id,
            level,
        });
    }
    let section_id = if item.ctor() == Ctor::Section {
        match item.attrs.get("id") {
            Some(Value::Str(id)) => Some(id.to_string()),
            _ => None,
        }
    } else {
        None
    };
    for (n, child) in item.children_mut().into_iter().flatten().enumerate() {
        headings(
            child,
            if n == 0 { section_id.clone() } else { None },
            ids,
            out,
        );
    }
}

impl Document {
    pub(crate) fn new(
        route: Route,
        mut root: Item,
        source: String,
        sources: Sources,
    ) -> Result<Self, Error> {
        let mut errors = vec![];
        for (key, value) in root.attrs.iter() {
            if value.contains_data_views() {
                errors.push(Diagnostic::new(notist::Phase::Type, root.span, format!("Data or DataError in template attribute `{key}` must be decoded to ordinary values")));
            }
        }
        let index = notist::index::Index::build(&root, &sources, &mut errors);
        if !errors.is_empty() {
            return Err(Error::Diagnostics(
                errors
                    .into_iter()
                    .map(|d| diagnostic(&route.source, &source, d))
                    .collect(),
            ));
        }
        let mut ids = index.ids().map(str::to_owned).collect();
        let mut outline = vec![];
        headings(&mut root, None, &mut ids, &mut outline);
        let attrs: BTreeMap<String, _> = root
            .attrs
            .iter()
            .map(|(k, v)| (k.into(), value(v)))
            .collect();
        for key in ["title", "template"] {
            if attrs.get(key).is_some_and(|v| !v.is_string()) {
                return Err(Error::Message(format!(
                    "{}: document `{key}` must be a string",
                    route.path.display()
                )));
            }
        }
        let title = attrs
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .or_else(|| outline.first().map(|h| h.title.clone()))
            .unwrap_or_else(|| {
                route
                    .path
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            });
        let directory = route
            .path
            .parent()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let mut ancestors: Vec<_> = route
            .path
            .parent()
            .unwrap()
            .ancestors()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        ancestors.reverse();
        Ok(Self {
            page: Page {
                route,
                title,
                attrs,
                headings: outline,
                html: String::new(),
                directory,
                ancestors,
            },
            root,
            source,
            sources,
            ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_variants_survive_template_projection() {
        let nested = Value::Ok(Box::new(Value::Err(Box::new(Value::Str("bad".into())))));
        assert_eq!(value(&nested), serde_json::json!({"ok":{"err":"bad"}}));
        let path = notist::ResourcePath::resolve("/private", "doc.not", "data.txt").unwrap();
        let nested = Value::Err(Box::new(Value::Path(path)));
        assert_eq!(value(&nested), serde_json::json!({"err":null}));
    }
}
