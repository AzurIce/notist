use crate::{
    Page,
    routes::{natural_cmp, relative_url},
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
    children: BTreeMap<String, Node>,
}
pub(crate) struct Layout {
    root: Node,
    order: Vec<PathBuf>,
}
impl Layout {
    pub fn new(title: &str, pages: &[Page]) -> Self {
        let mut root = Node {
            title: title.into(),
            ..Node::default()
        };
        for page in pages {
            let mut node = &mut root;
            for segment in page.route.output.parent().unwrap().iter() {
                let label = segment.to_string_lossy().into_owned();
                node = node.children.entry(label.clone()).or_insert_with(|| Node {
                    title: label,
                    ..Node::default()
                });
            }
            node.title = page.title.clone();
            node.output = Some(page.route.output.clone());
        }
        fn order(node: &Node, paths: &mut Vec<PathBuf>) {
            if let Some(path) = &node.output {
                paths.push(path.clone());
            }
            let mut children: Vec<_> = node.children.iter().collect();
            children.sort_by(|(a, _), (b, _)| natural_cmp(a, b));
            for (_, child) in children {
                order(child, paths);
            }
        }
        let mut paths = vec![];
        order(&root, &mut paths);
        Self { root, order: paths }
    }
    pub fn navigation(&self, current: &Path) -> Navigation {
        fn convert(node: &Node, current: &Path) -> Navigation {
            let mut children: Vec<_> = node.children.iter().collect();
            children.sort_by(|(a, _), (b, _)| natural_cmp(a, b));
            Navigation {
                title: node.title.clone(),
                url: node
                    .output
                    .as_ref()
                    .map(|path| relative_url(current, path.parent().unwrap(), true)),
                current: node.output.as_deref() == Some(current),
                children: children
                    .into_iter()
                    .map(|(_, node)| convert(node, current))
                    .collect(),
            }
        }
        convert(&self.root, current)
    }
    pub fn breadcrumbs(&self, current: &Path) -> Vec<Navigation> {
        let mut nodes = vec![&self.root];
        let mut node = &self.root;
        for segment in current.parent().unwrap().iter() {
            node = &node.children[segment.to_str().unwrap()];
            nodes.push(node);
        }
        nodes
            .into_iter()
            .map(|node| Navigation {
                title: node.title.clone(),
                url: node
                    .output
                    .as_ref()
                    .map(|path| relative_url(current, path.parent().unwrap(), true)),
                current: node.output.as_deref() == Some(current),
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
