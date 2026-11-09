use super::structure::{self, Entry, NavigationKind};
use crate::Page;
use notist::publication::url::relative_url;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn url(current: &Path, target: &Path) -> String {
    if target.file_name().is_some_and(|s| s == "index.html") {
        relative_url(current, target.parent().unwrap(), true)
    } else {
        relative_url(current, target, false)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Navigation {
    pub id: usize,
    pub output: Option<PathBuf>,
    pub kind: NavigationKind,
    pub title: String,
    pub attrs: BTreeMap<String, serde_json::Value>,
    pub url: Option<String>,
    pub current: bool,
    /// Initial expansion state, opening ancestors to reveal the current page.
    pub expanded: bool,
    pub children: Vec<Navigation>,
}
#[derive(Debug, Clone, Serialize)]
pub struct PageLink {
    pub title: String,
    pub url: String,
}
struct Node {
    id: usize,
    kind: NavigationKind,
    title: String,
    attrs: BTreeMap<String, serde_json::Value>,
    output: Option<PathBuf>,
    collapsed: bool,
    children: Vec<Node>,
}
fn order(node: &Node, paths: &mut Vec<PathBuf>) {
    if let Some(path) = &node.output {
        paths.push(path.clone());
    }
    for child in &node.children {
        order(child, paths);
    }
}
fn declared_node(entry: &Entry, pages: &BTreeMap<PathBuf, &Page>) -> Node {
    let page = entry.source.as_ref().map(|source| pages[source]);
    Node {
        id: 0,
        kind: entry.kind,
        title: entry
            .label
            .clone()
            .or_else(|| page.map(|page| page.title.clone()))
            .unwrap_or_default(),
        attrs: page.map(|page| page.attrs.clone()).unwrap_or_default(),
        output: page.map(|page| page.route.output.clone()),
        collapsed: entry.collapsed,
        children: entry
            .children
            .iter()
            .map(|child| declared_node(child, pages))
            .collect(),
    }
}
pub(crate) struct Layout {
    roots: Vec<Node>,
    order: Vec<PathBuf>,
}
impl Layout {
    pub fn new(title: &str, pages: &[Page]) -> Self {
        let mut entry = structure::directory(Path::new(""), pages.iter().map(|page| &page.route));
        if entry.source.is_none() {
            entry.label = Some(title.into());
        }
        let by_source = pages
            .iter()
            .map(|page| (page.route.source.clone(), page))
            .collect();
        let root = declared_node(&entry, &by_source);
        Self::from_roots(vec![root])
    }
    /// Preserve the declared forest, including the root page's position and children.
    pub fn declared(entries: &[Entry], pages: &[Page]) -> Self {
        let by_source: BTreeMap<PathBuf, &Page> = pages
            .iter()
            .map(|page| (page.route.source.clone(), page))
            .collect();
        Self::from_roots(
            entries
                .iter()
                .map(|entry| declared_node(entry, &by_source))
                .collect(),
        )
    }
    fn from_roots(mut roots: Vec<Node>) -> Self {
        fn identify(nodes: &mut [Node], next: &mut usize) {
            for node in nodes {
                node.id = *next;
                *next += 1;
                identify(&mut node.children, next);
            }
        }
        identify(&mut roots, &mut 0);
        let mut paths = vec![];
        for root in &roots {
            order(root, &mut paths);
        }
        Self {
            roots,
            order: paths,
        }
    }
    pub fn navigation(&self, current: &Path) -> Vec<Navigation> {
        fn convert(node: &Node, current: &Path) -> (Navigation, bool) {
            let mut contains_current = false;
            let children = node
                .children
                .iter()
                .map(|child| {
                    let (navigation, active) = convert(child, current);
                    contains_current |= active;
                    navigation
                })
                .collect();
            (
                Navigation {
                    id: node.id,
                    output: node.output.clone(),
                    kind: node.kind,
                    title: node.title.clone(),
                    attrs: node.attrs.clone(),
                    url: node.output.as_ref().map(|path| url(current, path)),
                    current: node.output.as_deref() == Some(current),
                    expanded: !node.collapsed || contains_current,
                    children,
                },
                contains_current || node.output.as_deref() == Some(current),
            )
        }
        self.roots
            .iter()
            .map(|root| convert(root, current).0)
            .collect()
    }
    pub fn breadcrumbs(&self, current: &Path) -> Vec<Navigation> {
        fn find<'a>(node: &'a Node, current: &Path, trail: &mut Vec<&'a Node>) -> bool {
            trail.push(node);
            if node.output.as_deref() == Some(current) {
                return true;
            }
            for child in &node.children {
                if find(child, current, trail) {
                    return true;
                }
            }
            trail.pop();
            false
        }
        let mut trail = vec![];
        self.roots
            .iter()
            .any(|root| find(root, current, &mut trail));
        trail
            .into_iter()
            .map(|node| Navigation {
                id: node.id,
                output: node.output.clone(),
                kind: node.kind,
                title: node.title.clone(),
                attrs: node.attrs.clone(),
                url: node.output.as_ref().map(|path| url(current, path)),
                current: node.output.as_deref() == Some(current),
                expanded: true,
                children: vec![],
            })
            .collect()
    }
    pub fn adjacent(&self, current: &Path, pages: &[Page]) -> (Option<PageLink>, Option<PageLink>) {
        let Some(index) = self.order.iter().position(|path| path == current) else {
            return (None, None);
        };
        let link = |path: &PathBuf| {
            let page = pages
                .iter()
                .find(|page| &page.route.output == path)
                .unwrap();
            PageLink {
                title: page.title.clone(),
                url: url(current, path),
            }
        };
        (
            index.checked_sub(1).map(|index| link(&self.order[index])),
            self.order.get(index + 1).map(link),
        )
    }
    pub fn home_url(&self, current: &Path) -> String {
        self.order.first().map_or_else(
            || url(current, Path::new("index.html")),
            |path| url(current, path),
        )
    }
    pub fn order(&self) -> &[PathBuf] {
        &self.order
    }
}
