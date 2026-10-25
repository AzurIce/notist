//! Navigation structure shared by declared and directory-derived trees.
use crate::routes::{Route, natural_cmp};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub(crate) struct Entry {
    pub kind: NavigationKind,
    pub numbered: bool,
    pub label: Option<String>,
    pub source: Option<PathBuf>,
    pub collapsed: bool,
    pub children: Vec<Entry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NavigationKind {
    Chapter,
    Draft,
    Part,
    Separator,
}

/// Directory branches use output paths so README pages merge with their
/// directory and page/child relationships match the site's canonical routes.
/// Every branch is a chapter; its content page is optional.
#[derive(Default)]
struct Branch {
    label: Option<String>,
    source: Option<PathBuf>,
    children: BTreeMap<String, Branch>,
}

impl Branch {
    fn into_entry(self) -> Entry {
        let mut children: Vec<_> = self.children.into_iter().collect();
        children.sort_by(|(a, _), (b, _)| natural_cmp(a, b));
        Entry {
            kind: NavigationKind::Chapter,
            numbered: true,
            label: self.label,
            source: self.source,
            collapsed: false,
            children: children
                .into_iter()
                .map(|(_, branch)| branch.into_entry())
                .collect(),
        }
    }
}

/// Build a naturally ordered subtree from routes below an output directory.
pub(crate) fn directory<'a>(base: &Path, routes: impl IntoIterator<Item = &'a Route>) -> Entry {
    let mut root = Branch::default();
    for route in routes {
        let mut branch = &mut root;
        for segment in route.output.parent().unwrap().strip_prefix(base).unwrap() {
            let label = segment.to_string_lossy().into_owned();
            branch = branch
                .children
                .entry(label.clone())
                .or_insert_with(|| Branch {
                    label: Some(label),
                    ..Branch::default()
                });
        }
        branch.label = None; // Page titles are available after evaluation.
        branch.source = Some(route.source.clone());
    }
    root.into_entry()
}
