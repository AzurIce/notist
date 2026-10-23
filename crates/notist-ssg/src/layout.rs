use crate::{
    Page,
    routes::relative_url,
    structure::{self, Entry},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize)]
pub struct Navigation {
    pub title: String,
    pub url: Option<String>,
    pub current: bool,
    /// Initial expansion state, including automatic expansion of the current path.
    pub expanded: bool,
    pub children: Vec<Navigation>,
}
#[derive(Debug, Clone, Serialize)]
pub struct PageLink {
    pub title: String,
    pub url: String,
}
#[derive(Default)]
struct Node {
    title: String,
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
        title: entry
            .label
            .clone()
            .or_else(|| page.map(|page| page.title.clone()))
            .unwrap_or_default(),
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
    root: Node,
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
        let mut paths = vec![];
        order(&root, &mut paths);
        Self { root, order: paths }
    }
    /// A declared structure replaces the filesystem-derived tree. A first
    /// entry targeting the root page merges into the navigation root, so the
    /// shell keeps its shape.
    pub fn declared(title: &str, entries: &[Entry], pages: &[Page]) -> Self {
        let by_source: BTreeMap<PathBuf, &Page> = pages
            .iter()
            .map(|page| (page.route.source.clone(), page))
            .collect();
        let mut root = Node {
            title: title.into(),
            ..Node::default()
        };
        let mut rest = entries;
        if let Some(first) = entries.first()
            && let Some(source) = &first.source
            && by_source[source].route.output == Path::new("index.html")
        {
            root = declared_node(first, &by_source);
            rest = &entries[1..];
        }
        root.children
            .extend(rest.iter().map(|entry| declared_node(entry, &by_source)));
        let mut paths = vec![];
        order(&root, &mut paths);
        Self { root, order: paths }
    }
    pub fn navigation(&self, current: &Path) -> Navigation {
        fn convert(node: &Node, current: &Path) -> (Navigation, bool) {
            let mut active = node.output.as_deref() == Some(current);
            let children = node
                .children
                .iter()
                .map(|child| {
                    let (navigation, contains_current) = convert(child, current);
                    active |= contains_current;
                    navigation
                })
                .collect();
            (
                Navigation {
                    title: node.title.clone(),
                    url: node
                        .output
                        .as_ref()
                        .map(|path| relative_url(current, path.parent().unwrap(), true)),
                    current: node.output.as_deref() == Some(current),
                    expanded: !node.collapsed || active,
                    children,
                },
                active,
            )
        }
        convert(&self.root, current).0
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
        find(&self.root, current, &mut trail);
        trail
            .into_iter()
            .map(|node| Navigation {
                title: node.title.clone(),
                url: node
                    .output
                    .as_ref()
                    .map(|path| relative_url(current, path.parent().unwrap(), true)),
                current: node.output.as_deref() == Some(current),
                expanded: true,
                children: vec![],
            })
            .collect()
    }
    pub fn adjacent(&self, current: &Path, pages: &[Page]) -> (Option<PageLink>, Option<PageLink>) {
        let index = self.order.iter().position(|path| path == current).unwrap();
        let link = |path: &PathBuf| {
            let page = pages
                .iter()
                .find(|page| &page.route.output == path)
                .unwrap();
            PageLink {
                title: page.title.clone(),
                url: relative_url(current, path.parent().unwrap(), true),
            }
        };
        (
            index.checked_sub(1).map(|index| link(&self.order[index])),
            self.order.get(index + 1).map(link),
        )
    }
    pub fn home_url(&self, current: &Path) -> String {
        relative_url(current, self.order[0].parent().unwrap(), true)
    }
    pub fn ordered_pages(&self, pages: Vec<Page>) -> Vec<Page> {
        let mut pages: BTreeMap<_, _> = pages
            .into_iter()
            .map(|page| (page.route.output.clone(), page))
            .collect();
        self.order
            .iter()
            .map(|path| pages.remove(path).unwrap())
            .collect()
    }
}
