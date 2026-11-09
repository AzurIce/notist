use crate::resources::normalize;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use rowan::TextRange;

use crate::diag::{Diagnostic, Phase};
use crate::index::Index;
use crate::item::{Ctor, Item, Value};

/// A derived document index and link graph, with root-relative paths.
pub struct VaultIndex {
    root: PathBuf,
    docs: HashMap<PathBuf, Doc>,
    directories: BTreeSet<PathBuf>,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// `https:`-style schemes never resolve.
    External,
    /// `[[#item]]`: an id in the current document.
    SameDoc { item: String },
    /// A normalized identity resolved relative to the linking document.
    Path { path: PathBuf, item: Option<String> },
}

use LinkTarget as Target;

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

/// Classify a source link using the same rules as workspace link checks.
/// `path` is the owning document's absolute logical path, not an output URL.
pub fn link_target(path: &Path, node: &Item) -> Option<LinkTarget> {
    if node.ctor() != Ctor::Link {
        return None;
    }
    match node.field("target") {
        Some(Value::Path(target)) => Some(Target::Path {
            path: PathBuf::from(target.logical_path()),
            item: crate::embed::string(&node.fields(), "fragment").map(str::to_owned),
        }),
        Some(Value::Str(target)) => {
            Some(parse_target(target, path.parent().unwrap_or(Path::new(""))))
        }
        _ => None,
    }
}

/// Resolve the innermost link containing a source byte offset, including its
/// label's Text leaves. This performs no IO or document evaluation.
pub fn link_at(path: &Path, item: &Item, offset: usize) -> Option<LinkTarget> {
    if !item.span.contains((offset as u32).into()) {
        return None;
    }
    item.children()
        .iter()
        .find_map(|child| link_at(path, child, offset))
        .or_else(|| link_target(path, item))
}

fn collect_links(item: &Item, path: &Path) -> Vec<Link> {
    item.descendants()
        .filter(|item| item.ctor() == Ctor::Link)
        .filter_map(|item| {
            Some(Link {
                span: item.span,
                target: link_target(path, item)?,
            })
        })
        .collect()
}

/// Document paths needed by the host to complete a link graph. This performs
/// no IO; schemes and same-document anchors do not request another resource.
pub(crate) fn linked_paths(path: &Path, item: &Item) -> Vec<PathBuf> {
    collect_links(item, path)
        .into_iter()
        .filter_map(|link| match link.target {
            Target::Path { path, .. } => Some(path),
            _ => None,
        })
        .collect()
}

/// Check bound embed identities against the Vault root, without resource reads.
pub(crate) fn check_embed_paths(
    root: &Path,
    path: &Path,
    item: &Item,
    sources: &crate::Sources,
) -> Vec<Diagnostic> {
    let source = normalize(&root.join(path)).to_string_lossy().into_owned();
    crate::embed::references(item)
        .filter_map(|node| {
            let Some(Value::Path(target)) = node.field("target") else {
                return None;
            };
            if Path::new(&target.logical_path()).starts_with(root) {
                return None;
            }
            let mut diagnostic = Diagnostic::new(
                Phase::Semantic,
                node.span,
                format!("local resource is outside Vault root `{}`", root.display()),
            );
            diagnostic.code = Some(crate::DiagnosticCode::OutsideVault);
            if let Some(origin) = &node.origin {
                diagnostic = diagnostic.at_source(sources, origin.source);
            } else {
                match sources.by_name(&source) {
                    Ok(source) => diagnostic.source = Some(source.clone()),
                    Err(error) => {
                        diagnostic.message.push_str(&format!(" ({error})"));
                        diagnostic.span = Default::default();
                    }
                }
            }
            Some(diagnostic)
        })
        .collect()
}

/// Source-level link checks retain their existing String rules; embeds use their
/// bound identities. Module dependencies are not content references.
pub(crate) fn check_content_paths(
    root: &Path,
    path: &Path,
    item: &Item,
    sources: &crate::Sources,
) -> Vec<Diagnostic> {
    let path = normalize(&root.join(path));
    let file_dir = path.parent().unwrap_or(root);
    let source = path.to_string_lossy().into_owned();
    let mut diagnostics: Vec<_> = item
        .descendants()
        .filter(|node| node.ctor() == Ctor::Link)
        .filter_map(|node| {
            let Some(Value::Str(target)) = node.field("target") else {
                return None;
            };
            match parse_target(target, file_dir) {
                Target::Path { path, .. } if !path.starts_with(root) => {
                    let mut diagnostic = Diagnostic::new(
                        Phase::Semantic,
                        node.span,
                        format!(
                            "local content reference `{target}` is outside Vault root `{}`",
                            root.display()
                        ),
                    );
                    diagnostic.code = Some(crate::DiagnosticCode::OutsideVault);
                    if let Some(origin) = &node.origin {
                        diagnostic = diagnostic.at_source(sources, origin.source);
                    } else {
                        match sources.by_name(&source) {
                            Ok(source) => diagnostic.source = Some(source.clone()),
                            Err(error) => {
                                diagnostic.message.push_str(&format!(" ({error})"));
                                diagnostic.span = Default::default();
                            }
                        }
                    }
                    Some(diagnostic)
                }
                _ => None,
            }
        })
        .collect();
    diagnostics.extend(check_embed_paths(root, &path, item, sources));
    diagnostics
}

/// Same-document link checks, shared by single-file checks and the vault.
pub fn check_doc_links(
    item: &Item,
    sources: &crate::Sources,
    index: &Index,
    diags: &mut Vec<Diagnostic>,
) {
    for node in item.descendants() {
        if node.ctor() != Ctor::Link {
            continue;
        }
        let Some(Value::Str(target)) = node.field("target") else {
            continue;
        };
        let Some(item) = target.strip_prefix('#') else {
            continue;
        };
        if index.by_id(item).is_none() {
            let mut diagnostic = Diagnostic::new(
                Phase::Semantic,
                node.span,
                format!("missing item `#{item}` in this document"),
            );
            if let Some(origin) = &node.origin {
                diagnostic = diagnostic.at_source(sources, origin.source);
            }
            diags.push(diagnostic);
        }
    }
}

/// Build the source index shared by single-file checks and editor tooling.
/// The id/tag convention's diagnostics apply only to native Markup documents.
pub fn check_document(path: &Path, analysis: &mut crate::Analysis) -> Index {
    let mut diagnostics = Vec::new();
    let index = Index::build(analysis.root(), analysis.sources(), &mut diagnostics);
    if path.extension().is_some_and(|extension| extension == "not") {
        check_doc_links(
            analysis.root(),
            analysis.sources(),
            &index,
            &mut diagnostics,
        );
        analysis.extend_diagnostics(diagnostics);
    }
    index
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
        let mut directories = BTreeSet::from([root.clone()]);
        for (path, analysis) in documents {
            let path = normalize(&root.join(path));
            directories.extend(
                path.parent()
                    .into_iter()
                    .flat_map(Path::ancestors)
                    .take_while(|parent| parent.starts_with(&root))
                    .map(Path::to_path_buf),
            );
            let (item, mut diagnostics, sources) = analysis.into_parts();
            // Pure callers may supply Pipeline results; Vault callers already
            // attached the same checks. Keep one diagnostic per reference.
            for diagnostic in check_content_paths(&root, &path, &item, &sources) {
                if !diagnostics.contains(&diagnostic) {
                    diagnostics.push(diagnostic);
                }
            }
            let index = Index::build(&item, &sources, &mut diagnostics);
            let links = collect_links(&item, &path);
            docs.insert(
                normalize(&path),
                Doc {
                    index,
                    links,
                    diagnostics,
                },
            );
        }
        Self {
            root,
            docs,
            directories,
        }
    }

    /// The host also knows directories containing no indexed documents.
    pub(crate) fn with_directories(mut self, directories: BTreeSet<PathBuf>) -> Self {
        self.directories.extend(directories);
        self
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
                    Target::Path {
                        path: target,
                        item: None,
                    } if self.directories.contains(target) => {}
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
                            if let Some(item) = item
                                && doc2.index.by_id(item).is_none()
                            {
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
                if let Target::Path { path: p, .. } = &link.target
                    && *p == path
                {
                    out.push((self.relative(source), link.span));
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
