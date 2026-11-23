use crate::{Error, Route};
use notc::{AttrValue, Block, BlockKind};
use notist::{Diagnostic, SourceDiagnostic};
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
    pub root: Block,
    pub ids: BTreeSet<String>,
    pub heading_ids: Vec<String>,
}

pub(crate) fn diagnostic(path: &Path, source: &str, d: Diagnostic) -> SourceDiagnostic {
    SourceDiagnostic::locate(path, source, d)
}

fn attr_to_json(value: &AttrValue) -> serde_json::Value {
    match value {
        AttrValue::Bool(b) => serde_json::Value::Bool(*b),
        AttrValue::Int(n) => serde_json::Value::Number((*n).into()),
        AttrValue::Float(f) => serde_json::Number::from_f64(*f)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        AttrValue::String(s) => serde_json::Value::String(s.to_string()),
        AttrValue::List(items) => {
            serde_json::Value::Array(items.iter().map(attr_to_json).collect())
        }
    }
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

fn collect_headings(
    block: &Block,
    ids: &mut BTreeSet<String>,
    heading_ids: &mut Vec<String>,
    out: &mut Vec<Heading>,
) {
    match &block.node().kind {
        BlockKind::Fragment(children) => {
            for child in children.iter() {
                collect_headings(child, ids, heading_ids, out);
            }
        }
        BlockKind::Heading { level, title } => {
            let title_text = title.plain_text();
            let explicit = match block.node().attrs.get("id") {
                Some(AttrValue::String(id)) => Some(id.to_string()),
                _ => None,
            };
            let id = explicit.unwrap_or_else(|| {
                let base = slug(&title_text);
                let mut candidate = base.clone();
                let mut n = 2;
                while !ids.insert(candidate.clone()) {
                    candidate = format!("{base}-{n}");
                    n += 1;
                }
                candidate
            });
            ids.insert(id.clone());
            heading_ids.push(id.clone());
            out.push(Heading {
                title: title_text,
                anchor: notist::publication::url::fragment(&id),
                id,
                level: i64::from(*level),
            });
        }
        BlockKind::Section { heading, body } => {
            if let BlockKind::Heading { level, title } = &heading.node().kind {
                let title_text = title.plain_text();
                let explicit = match block
                    .node()
                    .attrs
                    .get("id")
                    .or_else(|| heading.node().attrs.get("id"))
                {
                    Some(AttrValue::String(id)) => Some(id.to_string()),
                    _ => None,
                };
                let id = explicit.unwrap_or_else(|| {
                    let base = slug(&title_text);
                    let mut candidate = base.clone();
                    let mut n = 2;
                    while !ids.insert(candidate.clone()) {
                        candidate = format!("{base}-{n}");
                        n += 1;
                    }
                    candidate
                });
                ids.insert(id.clone());
                heading_ids.push(id.clone());
                out.push(Heading {
                    title: title_text,
                    anchor: notist::publication::url::fragment(&id),
                    id,
                    level: i64::from(*level),
                });
            } else {
                collect_headings(heading, ids, heading_ids, out);
            }
            collect_headings(body, ids, heading_ids, out);
        }
        BlockKind::Callout { body, .. }
        | BlockKind::Quote(body)
        | BlockKind::Div(body)
        | BlockKind::SectionBox(body) => {
            collect_headings(body, ids, heading_ids, out);
        }
        BlockKind::List { items, .. } => {
            for item in items.iter() {
                collect_headings(item, ids, heading_ids, out);
            }
        }
        BlockKind::Custom(custom) => {
            for (_, val) in custom.props.iter() {
                if let notc::CustomValue::Block(b) = val {
                    collect_headings(b, ids, heading_ids, out);
                }
            }
        }
        BlockKind::Paragraph(_)
        | BlockKind::CodeBlock { .. }
        | BlockKind::Table { .. }
        | BlockKind::Math(_)
        | BlockKind::Divider => {}
    }
}

impl Document {
    pub(crate) fn from_block(
        route: Route,
        block: Block,
        site_title: &str,
    ) -> Result<Self, Error> {
        let root = block.sectionize();

        let attrs: BTreeMap<String, serde_json::Value> = root
            .node()
            .attrs
            .entries()
            .iter()
            .map(|(k, v)| (k.to_string(), attr_to_json(v)))
            .collect();
        for key in ["title", "template"] {
            if attrs.get(key).is_some_and(|v| !v.is_string()) {
                return Err(Error::Message(format!(
                    "{}: document `{key}` must be a string",
                    route.path.display()
                )));
            }
        }

        let mut ids: BTreeSet<String> = root
            .ids()
            .into_iter()
            .map(|(id, _)| id.to_string())
            .collect();
        let mut heading_ids = Vec::new();
        let mut outline = Vec::new();
        collect_headings(&root, &mut ids, &mut heading_ids, &mut outline);

        let title = attrs
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .or_else(|| outline.first().map(|h| h.title.clone()))
            .or_else(|| (route.url == "/").then(|| site_title.to_owned()))
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
            ids,
            heading_ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notc::check_project;
    use std::path::PathBuf;

    #[test]
    fn extracts_explicit_and_generated_heading_ids() {
        let doc = r#"
@!(title: "Doc Title")

@(id: "custom")
= Same title

== Same title

== Same title
"#;
        let project = check_project(&[("index.not", doc)]).unwrap();
        let block = project.eval_document("index.not").unwrap();
        let route = Route::plan(PathBuf::from("content/index.not"), Path::new("content"), true).unwrap();
        let document = Document::from_block(route, block, "Site").unwrap();
        assert_eq!(document.page.title, "Doc Title");
        assert_eq!(
            document
                .page
                .headings
                .iter()
                .map(|h| h.id.as_str())
                .collect::<Vec<_>>(),
            ["custom", "same-title", "same-title-2"]
        );
    }
}
