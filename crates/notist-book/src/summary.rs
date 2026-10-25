//! SUMMARY.not or SUMMARY.md: an optional declarative book structure at the Vault root.
//!
//! The file is interpreted as data, never evaluated or rendered: an optional
//! document title, unnumbered prefix/suffix links, part headings, chapter lists
//! and separators follow mdBook's SUMMARY structure. Directory discovery,
//! unlinked chapters and collapsed branches extend the chapter-list syntax.
use notist::{Ctor, Diagnostic, Item, Phase, Resources, TextRange, Value, resources::ResourceKind};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use crate::{
    routes::{self, Routes},
    structure::{self, NavigationKind},
};

pub(crate) const FILE_NAMES: [&str; 2] = ["SUMMARY.not", "SUMMARY.md"];

/// A declared entry before target resolution. A `None` target is an unlinked
/// chapter; an empty target is a draft entry.
#[derive(Debug)]
pub(crate) struct Entry {
    pub kind: NavigationKind,
    pub numbered: bool,
    pub label: Option<String>,
    pub target: Option<String>,
    pub collapsed: bool,
    pub span: TextRange,
    pub children: Vec<Entry>,
}

enum Target {
    Label,
    Page(PathBuf),
    Directory(PathBuf),
}

struct Resolved {
    kind: NavigationKind,
    numbered: bool,
    label: Option<String>,
    target: Target,
    collapsed: bool,
    children: Vec<Resolved>,
}

#[derive(Default)]
struct Claims {
    pages: BTreeSet<PathBuf>,
    directories: BTreeSet<PathBuf>,
}

/// Validated declarations select sources before any publication routes exist.
pub(crate) struct Plan {
    entries: Vec<Resolved>,
    claims: Claims,
}

impl Plan {
    pub fn includes(&self, source: &Path) -> bool {
        self.claims.pages.contains(source)
            || self
                .claims
                .directories
                .iter()
                .any(|directory| source.starts_with(directory))
    }
}

fn diagnostic(span: TextRange, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Phase::Semantic, span, message)
}

fn text_of(item: &Item) -> String {
    let mut out = String::new();
    fn walk(item: &Item, out: &mut String) {
        if item.ctor == Ctor::Text
            && let Some(Value::Str(text)) = item.fields.get("text")
        {
            out.push_str(text);
        }
        for child in &item.children {
            walk(child, out);
        }
    }
    walk(item, &mut out);
    out.trim().to_string()
}

fn blank(item: &Item) -> bool {
    item.ctor == Ctor::Text
        && matches!(&item.fields.get("text"), Some(Value::Str(text)) if text.trim().is_empty())
}

/// Transparent containers carry no structure of their own.
fn flatten<'a>(item: &'a Item, blocks: &mut Vec<&'a Item>) {
    if matches!(item.ctor, Ctor::Section | Ctor::Node) {
        for child in &item.children {
            flatten(child, blocks);
        }
    } else {
        blocks.push(item);
    }
}

fn interpret_label(
    paragraph: &Item,
    errors: &mut Vec<Diagnostic>,
) -> Option<(Option<String>, Option<String>)> {
    let mut links = vec![];
    for node in paragraph.descendants() {
        match node.ctor {
            Ctor::Link => links.push(node),
            Ctor::Embed => {
                errors.push(diagnostic(
                    node.span,
                    "structure entries do not embed resources",
                ));
                return None;
            }
            _ => {}
        }
    }
    match links.as_slice() {
        [] => {
            let label = text_of(paragraph);
            if label.is_empty() {
                errors.push(diagnostic(
                    paragraph.span,
                    "structure entries need a link or a plain label",
                ));
                None
            } else {
                Some((Some(label), None))
            }
        }
        [link] => {
            let meaningful: Vec<_> = paragraph
                .children
                .iter()
                .filter(|child| !blank(child))
                .collect();
            if meaningful.len() != 1 || meaningful[0].ctor != Ctor::Link {
                errors.push(diagnostic(
                    paragraph.span,
                    "a structure link must be the whole label",
                ));
                return None;
            }
            let label = text_of(link);
            let target = match link.fields.get("target") {
                Some(Value::Str(target)) => target.clone(),
                _ => String::new(),
            };
            Some(((!label.is_empty()).then_some(label), Some(target)))
        }
        _ => {
            errors.push(diagnostic(
                paragraph.span,
                "a structure entry has at most one link",
            ));
            None
        }
    }
}

fn chapter_list(list: &Item, errors: &mut Vec<Diagnostic>) -> Vec<Entry> {
    if matches!(list.fields.get("ordered"), Some(Value::Bool(true))) {
        errors.push(diagnostic(list.span, "structure chapter lists use the `-` marker; chapter numbers are assigned automatically"));
    }
    list.children
        .iter()
        .filter_map(|item| entry(item, errors))
        .collect()
}

fn entry(item: &Item, errors: &mut Vec<Diagnostic>) -> Option<Entry> {
    if item.ctor != Ctor::ListItem {
        errors.push(diagnostic(
            item.span,
            "structure entries must be list items",
        ));
        return None;
    }
    let mut label = None;
    let mut nested = None;
    for child in &item.children {
        match child.ctor {
            Ctor::Paragraph if label.is_none() => label = Some(child),
            Ctor::List if nested.is_none() => nested = Some(child),
            _ => errors.push(diagnostic(
                child.span,
                "structure entries contain a label and at most one nested list",
            )),
        }
    }
    let Some(label) = label else {
        errors.push(diagnostic(
            item.span,
            "structure entries need a link or a plain label",
        ));
        return None;
    };
    let collapsed = match label.attrs.get("collapsed").or_else(|| {
        label
            .children
            .iter()
            .find(|child| !blank(child))
            .and_then(|child| child.attrs.get("collapsed"))
    }) {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            errors.push(diagnostic(
                item.span,
                "structure entry `collapsed` must be a boolean",
            ));
            false
        }
    };
    let (label, target) = interpret_label(label, errors)?;
    let children = nested
        .map(|list| chapter_list(list, errors))
        .unwrap_or_default();
    Some(Entry {
        kind: match target.as_deref() {
            None => NavigationKind::Chapter,
            Some("") => NavigationKind::Draft,
            Some(_) => NavigationKind::Chapter,
        },
        numbered: true,
        label,
        target,
        collapsed,
        span: item.span,
        children,
    })
}

/// Interpret the analyzed document as a structure declaration.
pub(crate) fn interpret(document: &Item) -> Result<Vec<Entry>, Vec<Diagnostic>> {
    let mut blocks = vec![];
    for child in &document.children {
        flatten(child, &mut blocks);
    }
    let mut errors = vec![];
    let mut entries = vec![];
    let mut numbered = false;
    let mut suffix = false;
    for (index, block) in blocks.iter().enumerate() {
        match block.ctor {
            Ctor::Heading => {
                if matches!(block.fields.get("level"), Some(Value::Int(1))) && index != 0 {
                    if suffix {
                        errors.push(diagnostic(
                            block.span,
                            "part titles cannot follow suffix chapters",
                        ));
                    }
                    let label = text_of(block);
                    if label.is_empty() {
                        errors.push(diagnostic(block.span, "part titles must not be empty"));
                    }
                    entries.push(Entry {
                        kind: NavigationKind::Part,
                        numbered: false,
                        label: Some(label),
                        target: None,
                        collapsed: false,
                        span: block.span,
                        children: vec![],
                    });
                }
            }
            Ctor::Divider => entries.push(Entry {
                kind: NavigationKind::Separator,
                numbered: false,
                label: None,
                target: None,
                collapsed: false,
                span: block.span,
                children: vec![],
            }),
            Ctor::List => {
                if suffix {
                    errors.push(diagnostic(
                        block.span,
                        "numbered chapters cannot follow suffix chapters",
                    ));
                }
                numbered = true;
                entries.extend(chapter_list(block, &mut errors));
            }
            Ctor::Paragraph => {
                let links: Vec<_> = block
                    .children
                    .iter()
                    .filter(|child| !blank(child))
                    .collect();
                if links.is_empty() || links.iter().any(|link| link.ctor != Ctor::Link) {
                    errors.push(diagnostic(
                        block.span,
                        "prefix and suffix chapters must contain only standalone links",
                    ));
                    continue;
                }
                suffix |= numbered;
                for link in links {
                    let target = match link.fields.get("target") {
                        Some(Value::Str(target)) => target.clone(),
                        _ => String::new(),
                    };
                    if target.ends_with('/') {
                        errors.push(diagnostic(link.span, "prefix and suffix chapters must reference a single document, not a directory"));
                    }
                    let label = text_of(link);
                    entries.push(Entry {
                        kind: if target.is_empty() {
                            NavigationKind::Draft
                        } else {
                            NavigationKind::Chapter
                        },
                        numbered: false,
                        label: (!label.is_empty()).then_some(label),
                        target: Some(target),
                        collapsed: false,
                        span: link.span,
                        children: vec![],
                    });
                }
            }
            _ => errors.push(diagnostic(
                block.span,
                "structure allows only headings, chapter links, chapter lists and separators",
            )),
        }
    }
    if entries
        .iter()
        .all(|entry| matches!(entry.kind, NavigationKind::Part | NavigationKind::Separator))
        && errors.is_empty()
    {
        errors.push(diagnostic(
            document.span,
            "site structure declares no entries",
        ));
    }
    if errors.is_empty() {
        Ok(entries)
    } else {
        Err(errors)
    }
}

/// Resolve entry targets against eligible sources, rejecting external
/// URLs, queries, fragments, ineligible documents and duplicates. Resolve all
/// explicit claims before discovery so their position never affects priority.
pub(crate) fn resolve<R: Resources>(
    entries: &[Entry],
    summary: &Path,
    sources: &BTreeSet<PathBuf>,
    resources: &R,
) -> Result<Plan, Vec<Diagnostic>> {
    fn resolve_entry<R: Resources>(
        entry: &Entry,
        summary: &Path,
        sources: &BTreeSet<PathBuf>,
        resources: &R,
        claims: &mut Claims,
        errors: &mut Vec<Diagnostic>,
    ) -> Resolved {
        let target = match entry.target.as_deref() {
            None | Some("") => Target::Label,
            Some(value) => match routes::target(summary, value) {
                Ok(routes::Target::External) => {
                    errors.push(diagnostic(
                        entry.span,
                        format!("structure entry `{value}` must reference a local document"),
                    ));
                    Target::Label
                }
                Ok(routes::Target::Local {
                    path,
                    query,
                    fragment,
                }) => {
                    if !query.is_empty() || fragment.is_some() {
                        errors.push(diagnostic(
                            entry.span,
                            format!("structure entry `{value}` must not carry a query or fragment"),
                        ));
                        Target::Label
                    } else if value.ends_with('/') {
                        if !path.starts_with(resources.root()) {
                            errors.push(diagnostic(
                                entry.span,
                                "structure directory is outside Vault root",
                            ));
                        } else {
                            match resources.kind(&path) {
                                Ok(Some(ResourceKind::Directory)) => {
                                    if !claims.directories.insert(path.clone()) {
                                        errors.push(diagnostic(
                                            entry.span,
                                            format!("duplicate structure directory `{value}`"),
                                        ));
                                    }
                                }
                                Ok(_) => errors.push(diagnostic(
                                    entry.span,
                                    format!("structure entry `{value}` is not a directory"),
                                )),
                                Err(error) => {
                                    errors.push(diagnostic(entry.span, error.to_string()))
                                }
                            }
                        }
                        Target::Directory(path)
                    } else if !sources.contains(&path) {
                        errors.push(diagnostic(
                            entry.span,
                            format!("structure entry `{value}` is not a published page"),
                        ));
                        Target::Label
                    } else {
                        if !claims.pages.insert(path.clone()) {
                            errors.push(diagnostic(
                                entry.span,
                                format!("duplicate structure entry `{value}`"),
                            ));
                        }
                        Target::Page(path)
                    }
                }
                Err(message) => {
                    errors.push(diagnostic(entry.span, message));
                    Target::Label
                }
            },
        };
        Resolved {
            kind: entry.kind,
            numbered: entry.numbered,
            label: entry.label.clone(),
            target,
            collapsed: entry.collapsed,
            children: entry
                .children
                .iter()
                .map(|child| resolve_entry(child, summary, sources, resources, claims, errors))
                .collect(),
        }
    }
    let mut errors = vec![];
    let mut claims = Claims::default();
    let entries = entries
        .iter()
        .map(|entry| resolve_entry(entry, summary, sources, resources, &mut claims, &mut errors))
        .collect();
    if errors.is_empty() {
        Ok(Plan { entries, claims })
    } else {
        Err(errors)
    }
}

impl Plan {
    /// Expand directory declarations using only the selected publication routes.
    pub fn expand(self, root: &Path, routes: &Routes) -> Vec<structure::Entry> {
        fn expand(
            entry: Resolved,
            root: &Path,
            routes: &Routes,
            claims: &Claims,
        ) -> structure::Entry {
            let mut children: Vec<_> = entry
                .children
                .into_iter()
                .map(|child| expand(child, root, routes, claims))
                .collect();
            match entry.target {
                Target::Directory(path) => {
                    let selected = routes.iter().filter(|route| {
                        route.source.starts_with(&path)
                            && !claims.pages.contains(&route.source)
                            // The most specific directory owns overlapping pages.
                            && !claims.directories.iter().any(|other| {
                                other != &path
                                    && other.starts_with(&path)
                                    && route.source.starts_with(other)
                            })
                    });
                    let mut discovered =
                        structure::directory(path.strip_prefix(root).unwrap(), selected);
                    discovered.collapsed = entry.collapsed;
                    discovered.label = entry.label.or_else(|| {
                        discovered.source.is_none().then(|| {
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned()
                        })
                    });
                    children.append(&mut discovered.children);
                    discovered.children = children;
                    discovered
                }
                target => structure::Entry {
                    kind: entry.kind,
                    numbered: entry.numbered,
                    label: entry.label,
                    collapsed: entry.collapsed,
                    source: match target {
                        Target::Page(path) => Some(path),
                        _ => None,
                    },
                    children,
                },
            }
        }
        self.entries
            .into_iter()
            .map(|entry| expand(entry, root, routes, &self.claims))
            .collect()
    }
}
