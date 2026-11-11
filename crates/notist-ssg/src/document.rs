use crate::{Error, Route};
use notist::publication::presentation_value as value;
use notist::{Ctor, Diagnostic, Item, SourceDiagnostic, Sources, Value};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Heading {
    pub title: String,
    pub id: String,
    pub anchor: String,
    pub level: i64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
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
    // Mutable child access detaches shared storage, even for an empty Vec.
    if !item.children().is_empty() {
        for (n, child) in item.children_mut().into_iter().flatten().enumerate() {
            headings(
                child,
                if n == 0 { section_id.clone() } else { None },
                ids,
                out,
            );
        }
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
#[path = "document_allocation_tests.rs"]
mod allocation_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_keep_leaf_storage_and_inherited_and_generated_targets() {
        let heading = || {
            Item::new(Ctor::Heading, Default::default())
                .with_field("level", Value::Int(2))
                .with_children(vec![Item::text("Same title".into(), Default::default())])
        };
        let mut section =
            Item::new(Ctor::Section, Default::default()).with_children(vec![heading()]);
        section.attrs.insert("id", Value::Str("section-id".into()));
        let mut root = Item::new(Ctor::Doc, Default::default()).with_children(vec![
            section,
            heading(),
            heading(),
            Item::new(Ctor::Error("ignored".into()), Default::default())
                .with_children(vec![heading()]),
            Item::new(Ctor::Divider, Default::default()),
        ]);
        let original = root.clone();
        let mut ids = BTreeSet::from(["section-id".into()]);
        let mut out = Vec::new();
        headings(&mut root, None, &mut ids, &mut out);
        assert_eq!(
            out.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
            ["section-id", "same-title", "same-title-2"]
        );
        assert!(out.iter().all(|h| h.title == "Same title" && h.level == 2));
        assert!(root.children()[0].children()[0].attrs.is_empty());
        assert!(original.children()[1].attrs.is_empty());
        for (changed, original) in root.descendants().zip(original.descendants()) {
            assert_eq!(changed.origin, original.origin);
            if original.children().is_empty() {
                assert!(
                    changed
                        .as_item()
                        .unwrap()
                        .children
                        .shares_storage(&original.as_item().unwrap().children)
                );
            }
        }
    }

    #[test]
    fn result_variants_survive_template_projection() {
        let nested = Value::Ok(Box::new(Value::Err(Box::new(Value::Str("bad".into())))));
        assert_eq!(value(&nested), serde_json::json!({"ok":{"err":"bad"}}));
        let path = notist::ResourcePath::resolve("/private", "doc.not", "data.txt").unwrap();
        let nested = Value::Err(Box::new(Value::Path(path)));
        assert_eq!(value(&nested), serde_json::json!({"err":null}));
    }
}
