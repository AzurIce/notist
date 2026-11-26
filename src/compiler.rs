//! Orchestrated Notist compiler built on `notist-core`, `Resources`, and `Executor`.
//!
//! ```text
//! Compiler ──► SourceSet ──► Analyzed ──► Evaluated (path → Block)
//! ```
//!
//! The compiler stops at evaluated Content trees (`Block`); HTML rendering,
//! templates, Sass, Web Components, and site publication live in downstream
//! crates (`notist-html`, `notist-ssg`, `notist-cli`).

use crate::{
    Executor, Resources,
    resources::{ResourceError, ResourceKind},
};
use notist_core::{
    Block, CheckedDocument, CheckedProject, DataHost, JsonValue, Module, ProjectDiagnostic,
    ProjectDiagnosticKind, ReplacementSpec, parse_document, parse_file,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Component, Path},
    sync::Arc,
};

#[derive(Debug, Clone)]
pub struct CompilerError {
    message: String,
}

impl CompilerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CompilerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CompilerError {}

impl From<ResourceError> for CompilerError {
    fn from(error: ResourceError) -> Self {
        Self::new(error.to_string())
    }
}

/// Orchestrated Notist compiler holding a read-only `Resources` view, a shared
/// `Executor` worker pool, and a statically checked shared `.not` module graph.
#[derive(Clone)]
pub struct Compiler {
    resources: Arc<dyn Resources + Send + Sync>,
    executor: Arc<Executor>,
    project: Arc<CheckedProject>,
}

impl fmt::Debug for Compiler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Compiler")
            .field("root", &self.resources.root())
            .field("jobs", &self.executor.jobs())
            .finish()
    }
}

impl Compiler {
    /// Discover and statically check every `.not` module reachable under
    /// `resources`.
    pub fn new(
        resources: Arc<dyn Resources + Send + Sync>,
        executor: Arc<Executor>,
    ) -> Result<Self, CompilerError> {
        let project = check_shared_project(resources.as_ref())?;
        Ok(Self {
            resources,
            executor,
            project: Arc::new(project),
        })
    }

    /// Convenience constructor rooted at a filesystem directory.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_dir(
        root: impl Into<std::path::PathBuf>,
        executor: Arc<Executor>,
    ) -> Result<Self, CompilerError> {
        let resources = Arc::new(crate::FsResources::new(root.into()));
        Self::new(resources, executor)
    }

    pub fn resources(&self) -> &Arc<dyn Resources + Send + Sync> {
        &self.resources
    }

    pub fn executor(&self) -> &Arc<Executor> {
        &self.executor
    }

    pub fn project(&self) -> &Arc<CheckedProject> {
        &self.project
    }

    /// Derive a new `Compiler` sharing the same checked module ASTs while
    /// rebinding declarations according to `specs`.
    pub fn with_replacements(&self, specs: &[ReplacementSpec]) -> Result<Self, CompilerError> {
        if specs.is_empty() {
            return Ok(self.clone());
        }
        let project = self.project.with_replacements(specs).map_err(|errs| {
            CompilerError::new(
                errs.iter()
                    .map(notist_core::format_replacement_error)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        })?;
        Ok(Self {
            resources: Arc::clone(&self.resources),
            executor: Arc::clone(&self.executor),
            project: Arc::new(project),
        })
    }

    /// Match relative resource paths against a glob pattern, sorted and
    /// deduplicated.
    pub fn glob(&self, pattern: &str) -> Result<Vec<String>, CompilerError> {
        glob_resources(self.resources.as_ref(), pattern)
    }

    /// Read a UTF-8 text resource relative to the compiler's root.
    pub fn read_text(&self, path: &str) -> Result<String, CompilerError> {
        let target = self.resources.root().join(path);
        self.resources.source(&target).map_err(CompilerError::from)
    }

    /// Create an empty immutable [`SourceSet`].
    pub fn sources(&self) -> SourceSet {
        SourceSet::default()
    }

    /// Parse and statically check all documents in `sources` in parallel using
    /// the shared `Executor`.
    pub fn analyze(&self, sources: &SourceSet) -> Result<Analyzed, CompilerError> {
        let docs = &sources.docs;
        if docs.is_empty() {
            return Ok(Analyzed {
                resources: Arc::clone(&self.resources),
                executor: Arc::clone(&self.executor),
                project: Arc::clone(&self.project),
                pages: Vec::new(),
            });
        }

        let resources = &self.resources;
        let project = &self.project;
        let results: Vec<Result<(String, CheckedDocument), String>> = self
            .executor
            .map_chunks(docs, 1, |_, chunk| {
                let path = &chunk[0];
                let target = resources.root().join(path);
                let source = resources
                    .source(&target)
                    .map_err(|err| format!("{path}: {err}"))?;
                let parsed = parse_document(&source);
                if !parsed.diagnostics.is_empty() {
                    return Err(parsed
                        .diagnostics
                        .iter()
                        .map(|diag| {
                            format!(
                                "{path}:{}..{}: {}",
                                diag.span.start, diag.span.end, diag.message
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"));
                }
                let checked = project
                    .check_standalone_document(path, &parsed.program.root)
                    .map_err(|errs| format_project_diagnostics(&errs))?;
                Ok((path.clone(), checked))
            });

        let mut pages = Vec::with_capacity(results.len());
        let mut errors = Vec::new();
        for res in results {
            match res {
                Ok(page) => pages.push(page),
                Err(err) => errors.push(err),
            }
        }
        if !errors.is_empty() {
            return Err(CompilerError::new(errors.join("\n")));
        }

        Ok(Analyzed {
            resources: Arc::clone(&self.resources),
            executor: Arc::clone(&self.executor),
            project: Arc::clone(&self.project),
            pages,
        })
    }
}

/// Immutable set of document paths selected for analysis.
#[derive(Debug, Clone, Default)]
pub struct SourceSet {
    docs: Vec<String>,
}

impl SourceSet {
    pub fn docs(&self) -> &[String] {
        &self.docs
    }

    pub fn add(&self, path: impl Into<String>) -> Self {
        let path = path.into();
        let mut docs = self.docs.clone();
        if !docs.iter().any(|existing| existing == &path) {
            docs.push(path);
        }
        Self { docs }
    }

    pub fn add_all<I, S>(&self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut docs = self.docs.clone();
        for path in paths {
            let path = path.into();
            if !docs.iter().any(|existing| existing == &path) {
                docs.push(path);
            }
        }
        Self { docs }
    }
}

/// Statically checked documents paired with their shared `CheckedProject`.
pub struct Analyzed {
    resources: Arc<dyn Resources + Send + Sync>,
    executor: Arc<Executor>,
    project: Arc<CheckedProject>,
    pages: Vec<(String, CheckedDocument)>,
}

impl Analyzed {
    pub fn documents(&self) -> Vec<String> {
        self.pages.iter().map(|(path, _)| path.clone()).collect()
    }

    pub fn project(&self) -> &Arc<CheckedProject> {
        &self.project
    }

    /// Evaluate every analyzed document's `__doc__` entry in parallel using the
    /// shared `Executor`.
    pub fn eval(&self) -> Result<Evaluated, CompilerError> {
        if self.pages.is_empty() {
            return Ok(Evaluated {
                pages: BTreeMap::new(),
            });
        }

        let host = ResourcesDataHost::new(Arc::clone(&self.resources));
        let project = &self.project;
        let results: Vec<Result<(String, Block), String>> = self
            .executor
            .map_chunks(&self.pages, 1, |_, chunk| {
                let (path, checked) = &chunk[0];
                let block = project
                    .eval_checked_document(checked, &host)
                    .map_err(|err| format!("{path}: {err:?}"))?;
                Ok((path.clone(), block))
            });

        let mut pages = BTreeMap::new();
        let mut errors = Vec::new();
        for res in results {
            match res {
                Ok((path, block)) => {
                    pages.insert(path, block);
                }
                Err(err) => errors.push(err),
            }
        }
        if !errors.is_empty() {
            return Err(CompilerError::new(errors.join("\n")));
        }

        Ok(Evaluated { pages })
    }
}

/// Read-only evaluated document content trees keyed by normalized document path.
#[derive(Debug, Clone)]
pub struct Evaluated {
    pages: BTreeMap<String, Block>,
}

impl Evaluated {
    pub fn pages(&self) -> &BTreeMap<String, Block> {
        &self.pages
    }

    pub fn page(&self, path: &str) -> Option<&Block> {
        self.pages.get(path)
    }
}

struct ResourcesDataHost {
    resources: Arc<dyn Resources + Send + Sync>,
}

impl ResourcesDataHost {
    fn new(resources: Arc<dyn Resources + Send + Sync>) -> Self {
        Self { resources }
    }

    fn resolve_target(&self, from_doc: &str, rel_path: &str) -> Result<std::path::PathBuf, String> {
        let resolved = notist_core::resolve_import_path(from_doc, rel_path)
            .map_err(|err| notist_core::format_type_error(&err))?;
        Ok(self.resources.root().join(resolved))
    }
}

impl DataHost for ResourcesDataHost {
    fn read_json(&self, from_doc: &str, rel_path: &str) -> Result<JsonValue, String> {
        let target = self.resolve_target(from_doc, rel_path)?;
        let text = self
            .resources
            .source(&target)
            .map_err(|err| err.to_string())?;
        JsonValue::parse(&text)
    }

    fn read_text(&self, from_doc: &str, rel_path: &str) -> Result<String, String> {
        let target = self.resolve_target(from_doc, rel_path)?;
        self.resources
            .source(&target)
            .map_err(|err| err.to_string())
    }
}

fn check_shared_project(resources: &dyn Resources) -> Result<CheckedProject, CompilerError> {
    let mut diagnostics = Vec::new();
    let mut modules: Vec<(String, Module)> = Vec::new();
    for (relative, source) in collect_not_sources(resources)? {
        let parsed = parse_file(&relative, &source);
        for diagnostic in parsed.diagnostics {
            diagnostics.push(format!(
                "{relative}:{}..{}: {}",
                diagnostic.span.start, diagnostic.span.end, diagnostic.message
            ));
        }
        modules.push((relative, parsed.program.root));
    }
    if !diagnostics.is_empty() {
        return Err(CompilerError::new(diagnostics.join("\n")));
    }
    let borrowed: Vec<(&str, &Module)> = modules
        .iter()
        .map(|(path, module)| (path.as_str(), module))
        .collect();
    notist_core::check_parsed_modules(&borrowed)
        .map_err(|errs| CompilerError::new(format_project_diagnostics(&errs)))
}

fn format_project_diagnostics(diagnostics: &[ProjectDiagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| match &diagnostic.kind {
            ProjectDiagnosticKind::Parse(parse) => format!(
                "{}:{}..{}: {}",
                diagnostic.module, parse.span.start, parse.span.end, parse.message
            ),
            ProjectDiagnosticKind::Type { span, error, .. } => format!(
                "{}:{}..{}: {}",
                diagnostic.module,
                span.start,
                span.end,
                notist_core::format_type_error(error)
            ),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn should_skip_dir_name(name: &str) -> bool {
    name.starts_with('.') || name == "notist-output" || name == "target"
}

fn relative_posix(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn walk_resource_files(
    resources: &dyn Resources,
    dir: &Path,
    out: &mut BTreeSet<String>,
) -> Result<(), CompilerError> {
    let entries = match resources.entries_with_kind(dir) {
        Ok(entries) => entries,
        Err(ResourceError::NotFound(_)) => return Ok(()),
        Err(err) => return Err(CompilerError::from(err)),
    };
    for entry in entries {
        let Some(name) = entry.path.file_name().map(|n| n.to_string_lossy()) else {
            continue;
        };
        if should_skip_dir_name(&name) {
            continue;
        }
        let kind = match entry.kind_hint {
            Some(kind) => Some(kind),
            None => resources.kind(&entry.path).ok().flatten(),
        };
        match kind {
            Some(ResourceKind::Directory) => walk_resource_files(resources, &entry.path, out)?,
            Some(ResourceKind::File) => {
                out.insert(relative_posix(resources.root(), &entry.path));
            }
            None => {}
        }
    }
    Ok(())
}

fn collect_not_sources(resources: &dyn Resources) -> Result<Vec<(String, String)>, CompilerError> {
    let mut files = BTreeSet::new();
    walk_resource_files(resources, resources.root(), &mut files)?;
    let mut out = Vec::new();
    for relative in files {
        if relative.ends_with(".not") {
            let target = resources.root().join(&relative);
            let source = resources.source(&target)?;
            out.push((relative, source));
        }
    }
    Ok(out)
}

fn glob_resources(resources: &dyn Resources, pattern: &str) -> Result<Vec<String>, CompilerError> {
    let matcher = globset::GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .and_then(|glob| {
            let mut set = globset::GlobSetBuilder::new();
            set.add(glob);
            set.build()
        })
        .map_err(|err| CompilerError::new(format!("invalid glob `{pattern}`: {err}")))?;

    let mut files = BTreeSet::new();
    walk_resource_files(resources, resources.root(), &mut files)?;
    Ok(files
        .into_iter()
        .filter(|rel| matcher.is_match(rel))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryResources;
    use std::path::PathBuf;

    #[test]
    fn compiler_analyzes_evaluates_and_replaces_in_memory() {
        let mut mem = MemoryResources::new(PathBuf::from("/virtual"));
        mem.insert(
            "custom/math.not",
            "fn math<C: Type = Inline>(text: String) -> C where C == Inline || C == Block;\n",
        );
        mem.insert("content/a.md", "# Hello\n\nInline $x + 1$.\n");
        mem.insert("content/b.md", "# World\n\nPlain text.\n");

        let executor = Arc::new(Executor::auto().unwrap());
        let compiler = Compiler::new(Arc::new(mem), executor)
            .unwrap()
            .with_replacements(&[ReplacementSpec {
                from_module: "notist".into(),
                from_name: "math".into(),
                to_module: "custom/math.not".into(),
                to_name: "math".into(),
            }])
            .unwrap();

        let docs = compiler.glob("content/**/*.md").unwrap();
        assert_eq!(docs, vec!["content/a.md", "content/b.md"]);

        let analyzed = compiler.analyze(&compiler.sources().add_all(docs)).unwrap();
        let evaluated = analyzed.eval().unwrap();
        assert_eq!(evaluated.pages().len(), 2);
        assert!(
            evaluated
                .page("content/a.md")
                .unwrap()
                .plain_text()
                .contains("Hello")
        );
    }
}
