use crate::resources::normalize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rowan::TextRange;

use crate::diag::{Diagnostic, Phase};
use crate::index::Index;
use crate::item::{Ctor, Item, Value};

/// A derived document index and link graph, with root-relative paths.
pub struct VaultIndex {
    docs: HashMap<PathBuf, Doc>,
}

struct Doc {
    index: Index,
    links: Vec<Link>,
    diagnostics: Vec<Diagnostic>,
}

struct Link {
    span: TextRange,
    target: Target,
}

/// A link target after splitting the `#item` suffix.
enum Target {
    /// `https:`-style schemes never resolve.
    External,
    /// `[[#item]]`: an id in the current document.
    SameDoc { item: String },
    /// A path relative to the linking file, normalized root-relative.
    Path { path: PathBuf, item: Option<String> },
}

fn has_scheme(target: &str) -> bool {
    target.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

fn parse_target(target: &str, file_dir: &Path) -> Target {
    if let Some(item) = target.strip_prefix('#') {
        return Target::SameDoc {
            item: item.to_string(),
        };
    }
    if has_scheme(target) {
        return Target::External;
    }
    let (path, item) = match target.split_once('#') {
        Some((path, item)) => (path, Some(item.to_string())),
        None => (target, None),
    };
    Target::Path {
        path: normalize(&file_dir.join(path)),
        item,
    }
}

fn collect_links(item: &Item, file_dir: &Path) -> Vec<Link> {
    item.descendants()
        .filter(|item| item.ctor == Ctor::Link)
        .filter_map(|item| match item.fields.get("target") {
            Some(Value::Str(target)) => Some(Link {
                span: item.span,
                target: parse_target(target, file_dir),
            }),
            _ => None,
        })
        .collect()
}

/// Same-document link checks, shared by single-file checks and the vault.
pub fn check_doc_links(item: &Item, index: &Index, diags: &mut Vec<Diagnostic>) {
    for node in item.descendants() {
        if node.ctor != Ctor::Link {
            continue;
        }
        let Some(Value::Str(target)) = node.fields.get("target") else {
            continue;
        };
        let Some(item) = target.strip_prefix('#') else {
            continue;
        };
        if index.by_id(item).is_none() {
            diags.push(Diagnostic::new(
                Phase::Semantic,
                node.span,
                format!("missing item `#{item}` in this document"),
            ));
        }
    }
}

impl VaultIndex {
    /// Build the derived index from already processed, root-relative documents.
    pub fn from_documents(documents: impl IntoIterator<Item = (PathBuf, crate::Analysis)>) -> Self {
        let mut docs = HashMap::new();
        for (path, analysis) in documents {
            let (item, mut diagnostics) = analysis.into_parts();
            let index = Index::build(&item, &mut diagnostics);
            let links = collect_links(&item, path.parent().unwrap_or(Path::new("")));
            docs.insert(
                normalize(&path),
                Doc {
                    index,
                    links,
                    diagnostics,
                },
            );
        }
        Self { docs }
    }

    /// Pipeline diagnostics per document, then link-resolution diagnostics.
    pub fn check(&self) -> Vec<(PathBuf, Diagnostic)> {
        let mut out = Vec::new();
        for (path, doc) in &self.docs {
            out.extend(doc.diagnostics.iter().map(|d| (path.clone(), d.clone())));
            for link in &doc.links {
                match &link.target {
                    Target::External => {}
                    Target::SameDoc { item } => {
                        if doc.index.by_id(item).is_none() {
                            out.push((
                                path.clone(),
                                Diagnostic::new(
                                    Phase::Semantic,
                                    link.span,
                                    format!("missing item `#{item}` in this document"),
                                ),
                            ));
                        }
                    }
                    Target::Path { path: target, item } => match self.docs.get(target) {
                        None => out.push((
                            path.clone(),
                            Diagnostic::new(
                                Phase::Semantic,
                                link.span,
                                format!("unresolved link target `{}`", target.display()),
                            ),
                        )),
                        Some(doc2) => {
                            if let Some(item) = item {
                                if doc2.index.by_id(item).is_none() {
                                    out.push((
                                        path.clone(),
                                        Diagnostic::new(
                                            Phase::Semantic,
                                            link.span,
                                            format!(
                                                "missing item `#{item}` in `{}`",
                                                target.display()
                                            ),
                                        ),
                                    ));
                                }
                            }
                        }
                    },
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.span.start().cmp(&b.1.span.start())));
        out
    }

    /// All links pointing at `path` (normalized, vault-root-relative).
    pub fn backlinks(&self, path: &Path) -> Vec<(PathBuf, TextRange)> {
        let path = normalize(path);
        let mut out = Vec::new();
        for (source, doc) in &self.docs {
            for link in &doc.links {
                if let Target::Path { path: p, .. } = &link.target {
                    if *p == path {
                        out.push((source.clone(), link.span));
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(u32::from(a.1.start()).cmp(&u32::from(b.1.start())))
        });
        out
    }
}
