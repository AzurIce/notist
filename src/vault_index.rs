use crate::resources::normalize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rowan::TextRange;

use crate::diag::{Diagnostic, Phase};
use crate::index::Index;
use crate::item::{Ctor, Item, Value};

/// A derived document index and link graph, with root-relative paths.
pub struct VaultIndex {
    root: PathBuf,
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
    /// A normalized identity resolved relative to the linking document.
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
    if has_scheme(target) || target.starts_with("//") {
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

/// Document paths needed by the host to complete a link graph. This performs
/// no IO; schemes and same-document anchors do not request another resource.
pub(crate) fn linked_paths(path: &Path, item: &Item) -> Vec<PathBuf> {
    collect_links(item, path.parent().unwrap_or(Path::new("")))
        .into_iter()
        .filter_map(|link| match link.target {
            Target::Path { path, .. } => Some(path),
            _ => None,
        })
        .collect()
}

/// Check local document and embed references against the logical Vault root.
/// This requires no resource reads; package dependencies are not content references.
pub(crate) fn check_content_paths(root: &Path, path: &Path, item: &Item) -> Vec<Diagnostic> {
    let path = normalize(&root.join(path));
    let file_dir = path.parent().unwrap_or(root);
    item.descendants()
        .filter(|node| matches!(node.ctor, Ctor::Link | Ctor::Embed))
        .filter_map(|node| {
            let Some(Value::Str(target)) = node.fields.get("target") else {
                return None;
            };
            match parse_target(target, file_dir) {
                Target::Path { path, .. } if !path.starts_with(root) => Some(Diagnostic::new(
                    Phase::Semantic,
                    node.span,
                    format!(
                        "local content reference `{target}` is outside Vault root `{}`",
                        root.display()
                    ),
                )),
                _ => None,
            }
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
    /// Build from processed documents using an absolute logical root. Relative
    /// document paths resolve against that root; link identities are absolute
    /// internally, while diagnostics and backlinks use root-relative paths.
    pub fn from_documents(
        root: impl AsRef<Path>,
        documents: impl IntoIterator<Item = (PathBuf, crate::Analysis)>,
    ) -> Self {
        let root = normalize(root.as_ref());
        let mut docs = HashMap::new();
        for (path, analysis) in documents {
            let path = normalize(&root.join(path));
            let (item, mut diagnostics) = analysis.into_parts();
            // Pure callers may supply Pipeline results; Vault callers already
            // attached the same checks. Keep one diagnostic per reference.
            for diagnostic in check_content_paths(&root, &path, &item) {
                if !diagnostics.contains(&diagnostic) {
                    diagnostics.push(diagnostic);
                }
            }
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
        Self { root, docs }
    }

    /// Pipeline diagnostics per document, then link-resolution diagnostics.
    pub fn check(&self) -> Vec<(PathBuf, Diagnostic)> {
        let mut out = Vec::new();
        for (path, doc) in &self.docs {
            let path = self.relative(path);
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
                    Target::Path { path: target, .. } if !target.starts_with(&self.root) => {}
                    Target::Path { path: target, item } => match self.docs.get(target) {
                        None => out.push((
                            path.clone(),
                            Diagnostic::new(
                                Phase::Semantic,
                                link.span,
                                format!(
                                    "unresolved link target `{}`",
                                    self.relative(target).display()
                                ),
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
                                                self.relative(target).display()
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
        let path = normalize(&self.root.join(path));
        if !path.starts_with(&self.root) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (source, doc) in &self.docs {
            for link in &doc.links {
                if let Target::Path { path: p, .. } = &link.target {
                    if *p == path {
                        out.push((self.relative(source), link.span));
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

    fn relative(&self, path: &Path) -> PathBuf {
        let base: Vec<_> = self.root.components().collect();
        let target: Vec<_> = path.components().collect();
        let shared = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
        if shared == 0 {
            return path.to_path_buf();
        }
        let mut relative = PathBuf::new();
        for _ in &base[shared..] {
            relative.push("..");
        }
        for component in &target[shared..] {
            relative.push(component.as_os_str());
        }
        relative
    }
}
