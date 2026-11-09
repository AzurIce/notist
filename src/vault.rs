//! Vault orchestration: resources, declaration environments and typed outputs.
use crate::environment::{discover_config_in, issue};
use crate::extensions::DocumentExtension;
use crate::resources::{FsResources, ResourceError, ResourceKind, Resources};
use crate::{Analysis, Environment, Pipeline, SourceDiagnostic, UnsupportedFormat};
use notist_html::{HtmlRegistry, ModuleLocator, RenderResult, Renderer};
use notist_pipeline::Evaluation;
use std::collections::{BTreeMap, BTreeSet};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

mod code;
mod parallel;
pub use code::CodeAnalysis;

#[derive(Debug)]
pub enum VaultError {
    Environment(Vec<SourceDiagnostic>),
    Unsupported(UnsupportedFormat),
    Resource(ResourceError),
    Source(crate::SourceError),
    /// Incomplete prepared hosts must load these identities and retry. This is
    /// not a recoverable content error or a publishable partial evaluation.
    NeedResources(Vec<crate::ResourcePath>),
}
impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Environment(errors) => {
                for (i, error) in errors.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}: {}", error.path.display(), error.diagnostic.message)?;
                }
                Ok(())
            }
            Self::Unsupported(error) => error.fmt(f),
            Self::Resource(error) => error.fmt(f),
            Self::Source(error) => error.fmt(f),
            Self::NeedResources(paths) => {
                write!(f, "{} resource inputs need preparation", paths.len())
            }
        }
    }
}
impl std::error::Error for VaultError {}
impl From<ResourceError> for VaultError {
    fn from(error: ResourceError) -> Self {
        Self::Resource(error)
    }
}
impl From<UnsupportedFormat> for VaultError {
    fn from(error: UnsupportedFormat) -> Self {
        Self::Unsupported(error)
    }
}
impl From<crate::SourceError> for VaultError {
    fn from(error: crate::SourceError) -> Self {
        Self::Source(error)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub source_map: bool,
}
impl Default for RenderOptions {
    fn default() -> Self {
        Self { source_map: true }
    }
}

/// Diagnostics carry their actual source, including evaluated Code definitions.
/// Module/config loading failures return source-bearing VaultError::Environment entries.
#[derive(Debug)]
pub struct HtmlOutput {
    pub path: PathBuf,
    pub analysis: Analysis,
    pub evaluation: Evaluation,
    pub rendered: RenderResult,
}

/// Rendering of an existing analysis tree, with the evaluated representation tree.
#[derive(Debug)]
pub struct ItemOutput {
    pub evaluation: Evaluation,
    pub rendered: RenderResult,
}

/// Exact document identity retained across host stages, bound to one Vault
/// configuration. The pure Analysis remains independent of host resources.
#[derive(Clone)]
pub struct DocumentHandle {
    path: PathBuf,
    source: crate::Source,
    environment: Arc<Environment>,
    owner: Arc<()>,
}

/// An immutable source analysis and its exact environment for editor queries.
/// Navigation reuses lowering; the index includes single-file check diagnostics.
#[derive(Debug)]
pub struct DocumentTooling {
    pub analysis: Analysis,
    pub environment: Arc<Environment>,
    pub navigation: crate::SourceNavigation,
    pub index: crate::index::Index,
}

/// One input batch, potentially containing multiple nearest configs. Loaded
/// bytes, errors and document inputs remain fixed; create a new Vault for the
/// next build or edit. This is a lazy view, not an atomic filesystem snapshot.
pub struct Vault<R = FsResources> {
    resources: crate::ResourceSession<R>,
    pipeline: Pipeline,
    extensions: Vec<Arc<dyn DocumentExtension>>,
    environments: BTreeMap<PathBuf, BTreeMap<Arc<str>, Arc<Environment>>>,
    environment_cache: crate::environment::EnvironmentCache,
    identity: Arc<()>,
    config: Option<PathBuf>,
    module_urls: BTreeMap<PathBuf, String>,
    evaluation_limits: crate::Limits,
}
impl<R: Resources> Vault<R> {
    pub fn new(resources: R) -> Self {
        let root = resources.root().to_path_buf();
        Self {
            // The host's project-root read grant is explicit and independent of
            // lexical Path roots and of embed/publication mounts.
            resources: crate::ResourceSession::new(resources).with_read_root(root),
            pipeline: Pipeline::default(),
            extensions: Vec::new(),
            environments: BTreeMap::new(),
            environment_cache: Default::default(),
            identity: Arc::new(()),
            config: None,
            module_urls: BTreeMap::new(),
            evaluation_limits: Default::default(),
        }
    }
    pub fn resources(&self) -> &R {
        self.resources.resources()
    }
    pub fn input_session(&self) -> &crate::ResourceSession<R> {
        &self.resources
    }
    /// Replace data grants; importing or publishing a directory never adds one.
    pub fn with_read_roots(mut self, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        self.resources = self.resources.with_read_roots(roots);
        self
    }
    pub fn with_input_limits(mut self, limits: crate::InputLimits) -> Self {
        self.resources = self.resources.with_limits(limits);
        self
    }
    pub fn with_evaluation_limits(mut self, limits: crate::Limits) -> Self {
        self.evaluation_limits = limits;
        self
    }
    pub fn with_data_cache_limits(mut self, bytes: usize, entries: usize) -> Self {
        self.resources = self.resources.with_data_cache_limits(bytes, entries);
        self
    }
    /// Load source bytes from the same input view used by `read_text`.
    pub fn source(&self, path: impl AsRef<Path>) -> Result<String, ResourceError> {
        self.resources.source(path.as_ref())
    }
    /// Whether the configured document pipeline handles this file format.
    pub fn supports(&self, path: impl AsRef<Path>) -> bool {
        self.pipeline.supports(path.as_ref())
    }
    pub fn with_pipeline(mut self, pipeline: Pipeline) -> Self {
        self.pipeline = pipeline;
        self.environments.clear();
        self.environment_cache = Default::default();
        self.identity = Arc::new(());
        self
    }
    /// Register a document extension, validated after the document's imports load.
    /// Native extensions run before extensions configured in Notist.toml.
    pub fn with_extension(mut self, extension: impl DocumentExtension + 'static) -> Self {
        self.extensions.push(Arc::new(extension));
        self.environments.clear();
        self.environment_cache = Default::default();
        self.identity = Arc::new(());
        self
    }
    pub fn with_config(mut self, path: impl AsRef<Path>) -> Self {
        self.config = Some(self.resources.resolve(path.as_ref()));
        self.environments.clear();
        self.environment_cache = Default::default();
        self.identity = Arc::new(());
        self
    }
    /// Host publication mapping. Resource paths remain distinct from browser URLs.
    pub fn with_module_urls(mut self, urls: BTreeMap<PathBuf, String>) -> Self {
        self.module_urls = urls
            .into_iter()
            .map(|(path, url)| (self.resources.resolve(&path), url))
            .collect();
        self
    }
    /// Keep each document source snapshot bound to its own import environment.
    /// Callers pass an already resolved path and retain the returned handles,
    /// rather than looking up the same path/source again for each operation.
    fn ensure_environment(
        &mut self,
        path: &Path,
        source: &str,
        imports: &[crate::Import],
    ) -> Result<(Arc<str>, Arc<Environment>), VaultError> {
        if let Some((text, environment)) = self
            .environments
            .get(path)
            .and_then(|versions| versions.get_key_value(source))
        {
            return Ok((text.clone(), environment.clone()));
        }
        let config = match &self.config {
            Some(config) => Some(config.clone()),
            None => discover_config_in(&self.resources, path)?,
        };
        let environment = Arc::new(
            Environment::for_document(
                &self.resources,
                &self.pipeline,
                crate::environment::DocumentSource {
                    path,
                    text: source,
                    imports,
                },
                config.as_deref(),
                &self.extensions,
                &mut self.environment_cache,
            )
            .map_err(VaultError::Environment)?,
        );
        let text: Arc<str> = Arc::from(source);
        self.resources.register_source(path, text.clone());
        self.environments
            .entry(path.to_path_buf())
            .or_default()
            .insert(text.clone(), environment.clone());
        Ok((text, environment))
    }
    fn environment_snapshot(
        &mut self,
        path: &Path,
        source: &str,
    ) -> Result<(Arc<str>, Arc<Environment>), VaultError> {
        if let Some((text, environment)) = self
            .environments
            .get(path)
            .and_then(|versions| versions.get_key_value(source))
        {
            return Ok((text.clone(), environment.clone()));
        }
        let lowered = self
            .pipeline
            .lower(path, source, crate::FrontendOptions::default())?;
        self.ensure_environment(path, source, &lowered.imports)
    }
    pub fn environment_for(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<Arc<Environment>, VaultError> {
        let path = self.resources.resolve(path.as_ref());
        self.environment_snapshot(&path, source)
            .map(|(_, environment)| environment)
    }
    pub fn analyze(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<Analysis, VaultError> {
        self.compile(path.as_ref(), source, false, None)
            .map(|(analysis, _)| analysis)
    }

    /// Analyze once for diagnostics, outlines, hover and source navigation.
    /// Runs analysis hooks, never evaluation or output/SSG extensions.
    pub fn analyze_for_tooling(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<DocumentTooling, VaultError> {
        let mut navigation = crate::SourceNavigation::default();
        let (mut analysis, _, handle) =
            self.compile_bound(path.as_ref(), source, false, None, Some(&mut navigation))?;
        let index = crate::vault_index::check_document(path.as_ref(), &mut analysis);
        Ok(DocumentTooling {
            analysis,
            environment: handle.environment,
            navigation,
            index,
        })
    }
    /// Analyze a document under a host-selected logical resource root, without
    /// changing module roots, import resolution or the Vault's access boundary.
    /// Site hosts use their content directory for root-relative embed references.
    pub fn analyze_with_resource_root(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
        resource_root: impl AsRef<Path>,
    ) -> Result<Analysis, VaultError> {
        let root = self.resources.resolve(resource_root.as_ref());
        self.compile(path.as_ref(), source, false, Some(&root))
            .map(|(analysis, _)| analysis)
    }
    pub fn inspect(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<(Analysis, notist_pipeline::Inspection), VaultError> {
        self.compile(path.as_ref(), source, true, None)
    }
    /// Analyze and keep the host identity for subsequent evaluation and binding.
    pub fn analyze_document_with_resource_root(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
        resource_root: impl AsRef<Path>,
    ) -> Result<(Analysis, DocumentHandle), VaultError> {
        let root = self.resources.resolve(resource_root.as_ref());
        let (analysis, _, handle) =
            self.compile_bound(path.as_ref(), source, false, Some(&root), None)?;
        Ok((analysis, handle))
    }
    fn compile(
        &mut self,
        path: &Path,
        source: &str,
        capture: bool,
        resource_root: Option<&Path>,
    ) -> Result<(Analysis, notist_pipeline::Inspection), VaultError> {
        self.compile_bound(path, source, capture, resource_root, None)
            .map(|(analysis, inspection, _)| (analysis, inspection))
    }
    fn compile_bound(
        &mut self,
        path: &Path,
        source: &str,
        capture: bool,
        resource_root: Option<&Path>,
        navigation: Option<&mut crate::SourceNavigation>,
    ) -> Result<(Analysis, notist_pipeline::Inspection, DocumentHandle), VaultError> {
        let compilation = crate::progress::Span::enter(|| {
            format!(
                "Analyze {}",
                path.strip_prefix(self.resources.root())
                    .unwrap_or(path)
                    .display()
            )
        });
        let path = self.resources.resolve(path);
        let output = crate::progress::run(
            || "Parse and lower".into(),
            || {
                self.pipeline.lower(
                    &path,
                    source,
                    crate::FrontendOptions {
                        capture_syntax: capture,
                    },
                )
            },
        )?;
        let (text, environment) = crate::progress::run(
            || "Load modules and environment".into(),
            || self.ensure_environment(&path, source, &output.imports),
        )?;
        let context = resource_root.map(|root| {
            environment
                .context()
                .clone()
                .with_resource_root(root.to_string_lossy())
        });
        if let Some(navigation) = navigation {
            *navigation =
                crate::SourceNavigation::from_lowered(&output, source, environment.context());
        }
        let checking = crate::progress::Span::enter(|| "Check and materialize".into());
        let (mut analysis, inspection) = self.pipeline.analyze_lowered(
            &path,
            text.clone(),
            output,
            context.as_ref().unwrap_or(environment.context()),
            capture,
        );
        analysis.extend_diagnostics(crate::vault_index::check_content_paths(
            self.resources.root(),
            &path,
            analysis.root(),
            analysis.sources(),
        ));
        let handle = DocumentHandle {
            path,
            source: analysis.source().clone(),
            environment,
            owner: self.identity.clone(),
        };
        checking.finish();
        compilation.finish();
        Ok((analysis, inspection, handle))
    }
    pub fn analyze_resource(&mut self, path: impl AsRef<Path>) -> Result<Analysis, VaultError> {
        let source = self.source(path.as_ref())?;
        self.analyze(path, &source)
    }
    /// Bind only native targets visited by HTML rendering of this evaluated tree.
    pub fn html_registry(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
        root: &crate::Item,
    ) -> Result<HtmlRegistry, VaultError> {
        self.html_registry_and_environment(path, source, root)
            .map(|(registry, _)| registry)
    }

    /// Bind HTML targets and retain the exact document environment used for
    /// binding, so publication can use its module resources without a second
    /// path/source lookup. No environments or lexical scopes are merged.
    pub fn html_registry_and_environment(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
        root: &crate::Item,
    ) -> Result<(HtmlRegistry, Arc<Environment>), VaultError> {
        let environment = self.environment_for(path, source)?;
        let registry = bind_html(&environment, &self.resources, &self.module_urls, root)
            .map_err(VaultError::Environment)?;
        Ok((registry, environment))
    }
    pub fn render_html(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
        options: RenderOptions,
    ) -> Result<HtmlOutput, VaultError> {
        let path = path.as_ref();
        let analysis = self.analyze(path, source)?;
        let ItemOutput {
            evaluation,
            rendered,
        } = self.render_output(path, &analysis, options)?;
        Ok(HtmlOutput {
            path: self.resources.resolve(path),
            analysis,
            evaluation,
            rendered,
        })
    }
    /// Evaluate with this document's immutable Context. Transform mappings
    /// select implementations while retaining the original semantic identity.
    pub fn evaluate(
        &mut self,
        path: impl AsRef<Path>,
        analysis: &Analysis,
    ) -> Result<Evaluation, VaultError> {
        let path = self.resources.resolve(path.as_ref());
        let source = analysis
            .sources()
            .get(path.to_string_lossy().as_ref())
            .ok_or_else(|| ResourceError::NotFound(path.clone()))?;
        let (_, environment) = self.environment_snapshot(&path, source)?;
        self.evaluate_in(analysis, environment.context())
    }
    fn check_handle(&self, handle: &DocumentHandle) -> Result<(), VaultError> {
        if Arc::ptr_eq(&self.identity, &handle.owner) {
            Ok(())
        } else {
            Err(ResourceError::Access {
                path: handle.path.clone(),
                message: "document handle belongs to another Vault or configuration".into(),
            }
            .into())
        }
    }
    pub fn evaluate_document(
        &self,
        handle: &DocumentHandle,
        analysis: &Analysis,
    ) -> Result<Evaluation, VaultError> {
        self.check_handle(handle)?;
        if analysis.source().id() != handle.source.id()
            || !analysis
                .source()
                .text()
                .zip(handle.source.text())
                .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
        {
            return Err(ResourceError::Access {
                path: handle.path.clone(),
                message: "analysis does not belong to this document handle".into(),
            }
            .into());
        }
        self.evaluate_in(analysis, handle.environment.context())
    }
    pub fn html_registry_for_document(
        &self,
        handle: &DocumentHandle,
        root: &crate::Item,
    ) -> Result<(HtmlRegistry, Arc<Environment>), VaultError> {
        self.check_handle(handle)?;
        let registry = bind_html(
            &handle.environment,
            &self.resources,
            &self.module_urls,
            root,
        )
        .map_err(VaultError::Environment)?;
        Ok((registry, handle.environment.clone()))
    }
    fn evaluate_in(
        &self,
        analysis: &Analysis,
        context: &crate::Context,
    ) -> Result<Evaluation, VaultError> {
        let evaluation = crate::progress::Span::enter(|| "Evaluate Code and extensions".into());
        let result = self.pipeline.evaluate_with_inputs_and_limits(
            analysis,
            context,
            &self.resources,
            self.evaluation_limits,
        );
        match result {
            crate::EvaluationOutcome::Complete(output) => {
                evaluation.finish();
                Ok(*output)
            }
            crate::EvaluationOutcome::NeedResources(paths) => Err(VaultError::NeedResources(paths)),
        }
    }

    /// Evaluate once, then bind and render only the targets present in the result.
    pub fn render_output(
        &mut self,
        path: impl AsRef<Path>,
        analysis: &Analysis,
        options: RenderOptions,
    ) -> Result<ItemOutput, VaultError> {
        let path = self.resources.resolve(path.as_ref());
        let source = analysis
            .sources()
            .get(path.to_string_lossy().as_ref())
            .ok_or_else(|| ResourceError::NotFound(path.clone()))?;
        let (_, environment) = self.environment_snapshot(&path, source)?;
        let mut evaluation = self.evaluate_in(analysis, environment.context())?;
        let outside = crate::vault_index::check_embed_paths(
            self.resources.root(),
            &path,
            &evaluation.root,
            &evaluation.sources,
        );
        for diagnostic in &outside {
            if !analysis.diagnostics().contains(diagnostic)
                && !evaluation.diagnostics.contains(diagnostic)
            {
                evaluation.diagnostics.push(diagnostic.clone());
            }
        }
        let registry = bind_html(
            &environment,
            &self.resources,
            &self.module_urls,
            &evaluation.root,
        )
        .map_err(VaultError::Environment)?;
        let mut renderer = Renderer::from_registry(registry)
            .with_sources(&evaluation.sources)
            .with_resource_resolver(|_, target| {
                let target = PathBuf::from(target.logical_path());
                target
                    .starts_with(self.resources.root())
                    .then(|| crate::publication::url::relative_url(&path, &target, false))
            });
        if options.source_map {
            renderer = renderer.with_source_map();
        }
        let mut rendered = renderer.render_with_diagnostics(&evaluation.root);
        // The host's boundary diagnostic is more precise than the renderer's
        // missing binding. Keep one error for an unauthorized embed reference.
        rendered.diagnostics.retain(|diagnostic| {
            diagnostic.code != Some(crate::DiagnosticCode::UnpublishedResource)
                || !outside
                    .iter()
                    .any(|error| error.span == diagnostic.span && error.source == diagnostic.source)
        });
        Ok(ItemOutput {
            evaluation,
            rendered,
        })
    }
    /// Scan a directory and reachable documents within the Vault's resource
    /// root. Returned paths are Vault-root-relative, including linked documents
    /// outside the initial scan directory. Out-of-Vault references are diagnosed.
    pub fn index(&mut self, root: impl AsRef<Path>) -> Result<crate::VaultIndex, VaultError> {
        let root = self.resources.resolve(root.as_ref());
        if !root.starts_with(self.resources.root()) {
            return Err(ResourceError::Access {
                path: root,
                message: "scan directory is outside Vault root".into(),
            }
            .into());
        }
        let mut directories = vec![root.clone()];
        let mut pending = Vec::new();
        while let Some(directory) = directories.pop() {
            for path in self.resources.entries(&directory)? {
                match self.resources.kind(&path)? {
                    Some(ResourceKind::Directory) => directories.push(path),
                    Some(ResourceKind::File) if self.pipeline.supports(&path) => {
                        pending.push(path);
                    }
                    _ => {}
                }
            }
        }
        let mut visited = BTreeSet::new();
        let mut documents = Vec::new();
        let mut linked_directories = BTreeSet::new();
        while let Some(path) = pending.pop() {
            if !visited.insert(path.clone()) {
                continue;
            }
            let source = self.resources.source(&path)?;
            let analysis = self.analyze(&path, &source)?;
            for target in crate::vault_index::linked_paths(&path, analysis.root()) {
                let resource = self.resources.resolve(&target);
                if resource.starts_with(self.resources.root()) && !visited.contains(&resource) {
                    match self.resources.kind(&resource)? {
                        Some(ResourceKind::Directory) => {
                            linked_directories.insert(resource);
                        }
                        Some(ResourceKind::File) if self.pipeline.supports(&resource) => {
                            pending.push(resource)
                        }
                        _ => {}
                    }
                }
            }
            documents.push((path, analysis));
        }
        Ok(
            crate::VaultIndex::from_documents(self.resources.root(), documents)
                .with_directories(linked_directories),
        )
    }
}
impl Vault<FsResources> {
    pub fn open(root: impl AsRef<Path>) -> Self {
        Self::new(FsResources::new(root))
    }
}

/// Shared host orchestration; entry rules and target validation belong to HTML.
pub(crate) fn bind_html(
    environment: &Environment,
    resources: &(impl Resources + ?Sized),
    urls: &BTreeMap<PathBuf, String>,
    root: &crate::Item,
) -> Result<HtmlRegistry, Vec<SourceDiagnostic>> {
    let mut bindings = BTreeMap::new();
    let mut errors = Vec::new();
    for definition in
        notist_html::components::component_definitions(root, environment.context().registry())
            .into_iter()
            .filter(|definition| {
                definition.id.module != "notist"
                    && !definition.implementation.is_code()
                    && environment
                        .context()
                        .registry()
                        .signature(definition.id.def_id())
                        .expect("registered declarations have resolved signatures")
                        .may_return_content()
            })
    {
        let module = &environment.modules()[&definition.id.module_id()];
        let result = (|| -> Result<Option<ModuleLocator>, String> {
            let entries = notist_html::components::component_entries(&definition.id.name)
                .map(|entry| resources.resolve(&module.path.parent().unwrap().join(entry)));
            let available = [
                resources.kind(&entries[0]).map_err(|e| e.to_string())? == Some(ResourceKind::File),
                resources.kind(&entries[1]).map_err(|e| e.to_string())? == Some(ResourceKind::File),
            ];
            let entry =
                notist_html::components::select_component_entry(&definition.id.name, available)?;
            Ok(entry.map(|entry| {
                let path = resources.resolve(&module.path.parent().unwrap().join(entry));
                match urls.get(&path) {
                    Some(url) => ModuleLocator::Url(url.clone()),
                    None => ModuleLocator::Resource(path),
                }
            }))
        })();
        match result {
            Ok(Some(module)) => {
                bindings.insert(definition.id.def_id(), module);
            }
            Ok(None) => {}
            Err(message) => errors.push(issue(
                &module.path,
                &module.source,
                definition.span,
                message,
            )),
        }
    }
    match HtmlRegistry::from_bindings(environment.context().registry(), &bindings) {
        Ok(registry) if errors.is_empty() => Ok(registry),
        Ok(_) => Err(errors),
        Err(binding_errors) => {
            for error in binding_errors {
                let definition = environment
                    .context()
                    .registry()
                    .definition(error.id)
                    .expect("bindings originate in this registry");
                let module = &environment.modules()[&definition.id.module_id()];
                let span = environment
                    .context()
                    .registry()
                    .definition(error.id)
                    .unwrap()
                    .span;
                errors.push(issue(&module.path, &module.source, span, error.message));
            }
            Err(errors)
        }
    }
}
