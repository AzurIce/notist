//! SUMMARY.not or SUMMARY.md: an optional declarative book structure at the Vault root.
//!
//! The file is interpreted as data, never evaluated or rendered: an optional
//! document title, prefix/suffix links, part headings, chapter lists
//! and separators follow mdBook's SUMMARY structure. Directory discovery,
//! unlinked chapters and collapsed branches extend the chapter-list syntax.
use notc::{AttrValue, Block, BlockKind, Inline, InlineKind};
use notist::{Diagnostic, Phase, Resources, TextRange, resources::ResourceKind};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::{
    routes::{self, Routes},
    structure::{self, NavigationKind},
};
use crate::mounts::Mounts;

pub(crate) const FILE_NAMES: [&str; 2] = ["SUMMARY.not", "SUMMARY.md"];

/// A declared entry before target resolution. A `None` target is an unlinked
/// chapter; an empty target is a draft entry.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Entry {
    pub kind: NavigationKind,
    pub label: Option<String>,
    pub target: Option<String>,
    pub collapsed: bool,
    #[serde(skip)]
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
    pub fn includes(&self, source: &Path, mounts: &Mounts) -> bool {
        self.claims.pages.contains(source)
            || self.claims.directories.iter().any(|directory| {
                mounts
                    .path(source)
                    .is_some_and(|path| path.starts_with(directory))
            })
    }
}

fn to_range(span: notc::Span) -> TextRange {
    TextRange::new(span.start.into(), span.end.into())
}

fn diagnostic(span: TextRange, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Phase::Semantic, span, message)
}

fn is_blank_inline(inline: &Inline) -> bool {
    matches!(&inline.node().kind, InlineKind::Text(t) if t.trim().is_empty())
}

fn inline_children(inline: &Inline) -> Vec<&Inline> {
    match &inline.node().kind {
        InlineKind::Fragment(children) => children.iter().collect(),
        _ => vec![inline],
    }
}

fn collect_inline_links<'a>(inline: &'a Inline, out: &mut Vec<&'a Inline>, has_image: &mut Option<TextRange>) {
    match &inline.node().kind {
        InlineKind::Fragment(children) => {
            for child in children.iter() {
                collect_inline_links(child, out, has_image);
            }
        }
        InlineKind::Strong(inner)
        | InlineKind::Emph(inner)
        | InlineKind::Strike(inner)
        | InlineKind::Underline(inner) => {
            collect_inline_links(inner, out, has_image);
        }
        InlineKind::Link { .. } => out.push(inline),
        InlineKind::Image { .. } => {
            if has_image.is_none() {
                *has_image = Some(to_range(inline.node().span));
            }
        }
        InlineKind::Text(_) | InlineKind::Raw(_) | InlineKind::Math(_) | InlineKind::Custom(_) => {}
    }
}

/// Transparent containers carry no structure of their own.
fn flatten<'a>(block: &'a Block, blocks: &mut Vec<&'a Block>) {
    match &block.node().kind {
        BlockKind::Fragment(children) => {
            for child in children.iter() {
                flatten(child, blocks);
            }
        }
        BlockKind::Section { heading, body } => {
            flatten(heading, blocks);
            flatten(body, blocks);
        }
        _ => blocks.push(block),
    }
}

fn interpret_label(
    paragraph: &Inline,
    para_span: TextRange,
    errors: &mut Vec<Diagnostic>,
) -> Option<(Option<String>, Option<String>)> {
    let mut links = Vec::new();
    let mut image_span = None;
    collect_inline_links(paragraph, &mut links, &mut image_span);
    if let Some(span) = image_span {
        errors.push(diagnostic(span, "structure entries do not embed resources"));
        return None;
    }
    match links.as_slice() {
        [] => {
            let label = paragraph.plain_text().trim().to_owned();
            if label.is_empty() {
                errors.push(diagnostic(
                    para_span,
                    "structure entries need a link or a plain label",
                ));
                None
            } else {
                Some((Some(label), None))
            }
        }
        [link] => {
            let meaningful: Vec<_> = inline_children(paragraph)
                .into_iter()
                .filter(|child| !is_blank_inline(child))
                .collect();
            if meaningful.len() != 1 || !matches!(meaningful[0].node().kind, InlineKind::Link { .. })
            {
                errors.push(diagnostic(
                    para_span,
                    "a structure link must be the whole label",
                ));
                return None;
            }
            let InlineKind::Link { target, label } = &link.node().kind else {
                unreachable!()
            };
            let label_text = label.plain_text().trim().to_owned();
            Some((
                (!label_text.is_empty()).then_some(label_text),
                Some(target.to_string()),
            ))
        }
        _ => {
            errors.push(diagnostic(
                para_span,
                "a structure entry has at most one link",
            ));
            None
        }
    }
}

fn chapter_list(list: &Block, errors: &mut Vec<Diagnostic>) -> Vec<Entry> {
    let BlockKind::List { ordered, items, .. } = &list.node().kind else {
        return Vec::new();
    };
    if *ordered {
        errors.push(diagnostic(
            to_range(list.node().span),
            "structure chapter lists use unordered markers; write chapter numbers in the label",
        ));
    }
    items
        .iter()
        .filter_map(|item| entry(item, errors))
        .collect()
}

fn entry(item: &Block, errors: &mut Vec<Diagnostic>) -> Option<Entry> {
    let item_span = to_range(item.node().span);
    let mut parts: Vec<&Block> = Vec::new();
    flatten(item, &mut parts);

    let mut label_block: Option<&Block> = None;
    let mut nested: Option<&Block> = None;
    for child in parts {
        match &child.node().kind {
            BlockKind::Paragraph(_) if label_block.is_none() => label_block = Some(child),
            BlockKind::List { .. } if nested.is_none() => nested = Some(child),
            _ => errors.push(diagnostic(
                to_range(child.node().span),
                "structure entries contain a label and at most one nested list",
            )),
        }
    }
    let Some(label_node) = label_block else {
        errors.push(diagnostic(
            item_span,
            "structure entries need a link or a plain label",
        ));
        return None;
    };
    let BlockKind::Paragraph(label_inline) = &label_node.node().kind else {
        unreachable!()
    };

    let collapsed_attr = label_node
        .node()
        .attrs
        .get("collapsed")
        .or_else(|| {
            inline_children(label_inline)
                .into_iter()
                .find(|child| !is_blank_inline(child))
                .and_then(|child| child.node().attrs.get("collapsed"))
        });
    let collapsed = match collapsed_attr {
        None => false,
        Some(AttrValue::Bool(value)) => *value,
        Some(_) => {
            errors.push(diagnostic(
                item_span,
                "structure entry `collapsed` must be a boolean",
            ));
            false
        }
    };

    let (label, target) = interpret_label(label_inline, to_range(label_node.node().span), errors)?;
    let children = nested
        .map(|list| chapter_list(list, errors))
        .unwrap_or_default();
    Some(Entry {
        kind: match target.as_deref() {
            None => NavigationKind::Chapter,
            Some("") => NavigationKind::Draft,
            Some(_) => NavigationKind::Chapter,
        },
        label,
        target,
        collapsed,
        span: item_span,
        children,
    })
}

/// Interpret the parsed `notc::Block` document as a book structure declaration.
pub(crate) fn interpret(document: &Block) -> Result<Vec<Entry>, Vec<Diagnostic>> {
    let mut blocks = Vec::new();
    flatten(document, &mut blocks);
    let mut errors = Vec::new();
    let mut entries = Vec::new();
    let mut chapters_started = false;
    let mut suffix = false;
    for (index, block) in blocks.iter().enumerate() {
        let block_span = to_range(block.node().span);
        match &block.node().kind {
            BlockKind::Heading { level, title } => {
                if *level == 1 && index != 0 {
                    if suffix {
                        errors.push(diagnostic(
                            block_span,
                            "part titles cannot follow suffix chapters",
                        ));
                    }
                    let label = title.plain_text().trim().to_owned();
                    if label.is_empty() {
                        errors.push(diagnostic(block_span, "part titles must not be empty"));
                    }
                    entries.push(Entry {
                        kind: NavigationKind::Part,
                        label: Some(label),
                        target: None,
                        collapsed: false,
                        span: block_span,
                        children: vec![],
                    });
                }
            }
            BlockKind::Divider => entries.push(Entry {
                kind: NavigationKind::Separator,
                label: None,
                target: None,
                collapsed: false,
                span: block_span,
                children: vec![],
            }),
            BlockKind::List { .. } => {
                if suffix {
                    errors.push(diagnostic(
                        block_span,
                        "chapter lists cannot follow suffix chapters",
                    ));
                }
                chapters_started = true;
                entries.extend(chapter_list(block, &mut errors));
            }
            BlockKind::Paragraph(inline) => {
                let links: Vec<_> = inline_children(inline)
                    .into_iter()
                    .filter(|child| !is_blank_inline(child))
                    .collect();
                if links.is_empty()
                    || links
                        .iter()
                        .any(|link| !matches!(link.node().kind, InlineKind::Link { .. }))
                {
                    errors.push(diagnostic(
                        block_span,
                        "prefix and suffix chapters must contain only standalone links",
                    ));
                    continue;
                }
                suffix |= chapters_started;
                for link in links {
                    let link_span = to_range(link.node().span);
                    let InlineKind::Link { target, label } = &link.node().kind else {
                        unreachable!()
                    };
                    if target.ends_with('/') {
                        errors.push(diagnostic(
                            link_span,
                            "prefix and suffix chapters must reference a single document, not a directory",
                        ));
                    }
                    let label_text = label.plain_text().trim().to_owned();
                    entries.push(Entry {
                        kind: if target.is_empty() {
                            NavigationKind::Draft
                        } else {
                            NavigationKind::Chapter
                        },
                        label: (!label_text.is_empty()).then_some(label_text),
                        target: Some(target.to_string()),
                        collapsed: false,
                        span: link_span,
                        children: vec![],
                    });
                }
            }
            _ => errors.push(diagnostic(
                block_span,
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
            to_range(document.node().span),
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
    mounts: &Mounts,
) -> Result<Plan, Vec<Diagnostic>> {
    fn resolve_entry<R: Resources>(
        entry: &Entry,
        summary: &Path,
        sources: &BTreeSet<PathBuf>,
        resources: &R,
        mounts: &Mounts,
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
                    let path = mounts.resolve(&path);
                    if !query.is_empty() || fragment.is_some() {
                        errors.push(diagnostic(
                            entry.span,
                            format!("structure entry `{value}` must not carry a query or fragment"),
                        ));
                        Target::Label
                    } else if mounts.path(&path).is_none() {
                        errors.push(diagnostic(
                            entry.span,
                            "structure target is outside Vault root and configured book mounts",
                        ));
                        Target::Label
                    } else if value.ends_with('/') {
                        let book_path = mounts.path(&path).unwrap();
                        let kind = if mounts.is_virtual_directory(&book_path) || sources.iter().any(|source| mounts.path(source).is_some_and(|p| p != book_path && p.starts_with(&book_path))) {
                            Ok(Some(ResourceKind::Directory))
                        } else {
                            resources.kind(&path)
                        };
                        match kind {
                            Ok(Some(ResourceKind::Directory)) => {
                                if !claims.directories.insert(book_path.clone()) {
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
                            Err(error) => errors.push(diagnostic(entry.span, error.to_string())),
                        }
                        Target::Directory(book_path)
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
            label: entry.label.clone(),
            target,
            collapsed: entry.collapsed,
            children: entry
                .children
                .iter()
                .map(|child| {
                    resolve_entry(child, summary, sources, resources, mounts, claims, errors)
                })
                .collect(),
        }
    }
    let mut errors = vec![];
    let mut claims = Claims::default();
    let entries = entries
        .iter()
        .map(|entry| {
            resolve_entry(
                entry,
                summary,
                sources,
                resources,
                mounts,
                &mut claims,
                &mut errors,
            )
        })
        .collect();
    if errors.is_empty() {
        Ok(Plan { entries, claims })
    } else {
        Err(errors)
    }
}

impl Plan {
    /// Expand directory declarations using only the selected publication routes.
    pub fn expand(self, routes: &Routes) -> Vec<structure::Entry> {
        fn expand(entry: Resolved, routes: &Routes, claims: &Claims) -> structure::Entry {
            let mut children: Vec<_> = entry
                .children
                .into_iter()
                .map(|child| expand(child, routes, claims))
                .collect();
            match entry.target {
                Target::Directory(path) => {
                    let selected = routes.iter().filter(|route| {
                        route.path.starts_with(&path)
                            && !claims.pages.contains(&route.source)
                            // The most specific directory owns overlapping pages.
                            && !claims.directories.iter().any(|other| {
                                other != &path
                                    && other.starts_with(&path)
                                    && route.path.starts_with(other)
                            })
                    });
                    let mut discovered = structure::directory(&path, selected);
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
            .map(|entry| expand(entry, routes, &self.claims))
            .collect()
    }
}
