use std::collections::HashMap;

use rowan::TextRange;

use crate::diag::{Diagnostic, Phase};
use crate::item::{Item, Value};

/// A queryable index over a document's `id` / `tags` attrs.
#[derive(Debug, Default)]
pub struct Index {
    by_id: HashMap<String, TextRange>,
    by_tag: HashMap<String, Vec<TextRange>>,
}

impl Index {
    /// Walk the tree and collect `id` / `tags` attrs. A duplicate `id` is
    /// diagnosed; the first occurrence wins.
    pub fn build(item: &Item, diags: &mut Vec<Diagnostic>) -> Self {
        let mut index = Index::default();
        for node in item.descendants() {
            if let Some(Value::Str(id)) = node.attrs.get("id") {
                use std::collections::hash_map::Entry;
                match index.by_id.entry(id.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(node.span);
                    }
                    Entry::Occupied(_) => diags.push(Diagnostic::new(
                        Phase::Semantic,
                        node.span,
                        format!("duplicate id `{id}`"),
                    )),
                }
            }
            if let Some(Value::Array(tags)) = node.attrs.get("tags") {
                for tag in tags {
                    if let Value::Str(tag) = tag {
                        index.by_tag.entry(tag.clone()).or_default().push(node.span);
                    }
                }
            }
        }
        index
    }

    pub fn by_id(&self, id: &str) -> Option<TextRange> {
        self.by_id.get(id).copied()
    }

    pub fn by_tag(&self, tag: &str) -> &[TextRange] {
        self.by_tag.get(tag).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.by_id.keys().map(String::as_str)
    }

    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.by_tag.keys().map(String::as_str)
    }
}
