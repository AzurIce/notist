//! Vault orchestration: resources, declaration environments and typed outputs.
use crate::environment::{discover_config_in, issue};
use crate::resources::{FsResources, ResourceError, ResourceKind, Resources};
use crate::{Analysis, Environment, Pipeline, SourceDiagnostic, UnsupportedFormat};
use notist_html::{HtmlRegistry, ModuleLocator, RenderResult, Renderer};
use notist_pipeline::transforms::TransformOutput;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum VaultError {
    Environment(Vec<SourceDiagnostic>),
    Unsupported(UnsupportedFormat),
    Resource(ResourceError),
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

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub source_map: bool,
}
impl Default for RenderOptions {
    fn default() -> Self {
        Self { source_map: true }
    }
}

/// Analysis, transform and render diagnostics refer to this document source. Package/config
/// failures instead return source-bearing VaultError::Environment entries.
#[derive(Debug)]
pub struct HtmlOutput {
    pub path: PathBuf,
    pub analysis: Analysis,
    pub transformed: TransformOutput,
    pub rendered: RenderResult,
}

/// Rendering of an existing analysis tree, with the transformed output tree.
#[derive(Debug)]
pub struct ItemOutput {
    pub transformed: TransformOutput,
    pub rendered: RenderResult,
}

/// One immutable resource view, potentially containing multiple nearest configs.
/// Environment assembly is retained within this view; replace the Vault when
/// inputs change. No mutable source cache or incremental compiler is implied.
pub struct Vault<R = FsResources> {
    resources: R,
    pipeline: Pipeline,
    environments: BTreeMap<Option<PathBuf>, Environment>,
    config: Option<PathBuf>,
    module_urls: BTreeMap<PathBuf, String>,
}
impl<R: Resources> Vault<R> {
    pub fn new(resources: R) -> Self {
        Self {
            resources,
            pipeline: Pipeline::default(),
            environments: BTreeMap::new(),
            config: None,
            module_urls: BTreeMap::new(),
        }
    }
    pub fn resources(&self) -> &R {
        &self.resources
    }
    pub fn with_pipeline(mut self, pipeline: Pipeline) -> Self {
        self.pipeline = pipeline;
        self
    }
    pub fn with_config(mut self, path: impl AsRef<Path>) -> Self {
        self.config = Some(self.resources.resolve(path.as_ref()));
        self
    }
    /// Supply a pure, already assembled default environment. Nearest or explicit
    /// configs still select their own environments.
    pub fn with_environment(mut self, environment: Environment) -> Self {
        let key = environment.config_path().map(Path::to_path_buf);
        if key.is_some() {
            self.config = key.clone();
        }
        self.environments.insert(key, environment);
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
    fn ensure_environment(&mut self, path: &Path) -> Result<Option<PathBuf>, VaultError> {
        let key = match &self.config {
            Some(config) => Some(config.clone()),
            None => discover_config_in(&self.resources, path)?,
        };
        if !self.environments.contains_key(&key) {
            let environment = match &key {
                Some(config) => Environment::load_from(&self.resources, config)
                    .map_err(VaultError::Environment)?,
                None => Environment::default(),
            };
            self.environments.insert(key.clone(), environment);
        }
        Ok(key)
    }
    pub fn environment_for(&mut self, path: impl AsRef<Path>) -> Result<&Environment, VaultError> {
        let key = self.ensure_environment(path.as_ref())?;
        Ok(&self.environments[&key])
    }
    pub fn analyze(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<Analysis, VaultError> {
        let path = path.as_ref();
        let key = self.ensure_environment(path)?;
        Ok(self
            .pipeline
            .analyze(path, source, self.environments[&key].registry())?)
    }
    pub fn inspect(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<(Analysis, notist_pipeline::Inspection), VaultError> {
        let path = path.as_ref();
        let key = self.ensure_environment(path)?;
        Ok(self
            .pipeline
            .inspect(path, source, self.environments[&key].registry())?)
    }
    pub fn analyze_resource(&mut self, path: impl AsRef<Path>) -> Result<Analysis, VaultError> {
        let source = self.resources.source(path.as_ref())?;
        self.analyze(path, &source)
    }
    pub fn html_registry(&mut self, path: impl AsRef<Path>) -> Result<HtmlRegistry, VaultError> {
        let key = self.ensure_environment(path.as_ref())?;
        bind_html(&self.environments[&key], &self.resources, &self.module_urls)
            .map_err(VaultError::Environment)
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
            transformed,
            rendered,
        } = self.render_output(path, analysis.root(), options)?;
        Ok(HtmlOutput {
            path: self.resources.resolve(path),
            analysis,
            transformed,
            rendered,
        })
    }
    /// Apply this document's configured plan without invoking a backend.
    pub fn transform(
        &mut self,
        path: impl AsRef<Path>,
        item: &crate::Item,
    ) -> Result<TransformOutput, VaultError> {
        Ok(self.environment_for(path)?.transforms().apply(item))
    }

    /// Transform an analysis tree once and render the result. Callers with an
    /// already transformed tree can pass it directly to notist_html::Renderer.
    pub fn render_output(
        &mut self,
        path: impl AsRef<Path>,
        item: &crate::Item,
        options: RenderOptions,
    ) -> Result<ItemOutput, VaultError> {
        let path = path.as_ref();
        let transformed = self.transform(path, item)?;
        let mut renderer = Renderer::new().with_registry(self.html_registry(path)?);
        if options.source_map {
            renderer = renderer.with_source_map();
        }
        let rendered = renderer.render_with_diagnostics(&transformed.root);
        Ok(ItemOutput {
            transformed,
            rendered,
        })
    }
    /// Index the directory and reachable linked documents, including sibling
    /// directories, through the same resources and nearest environments.
    pub fn index(&mut self, root: impl AsRef<Path>) -> Result<crate::VaultIndex, VaultError> {
        let root = self.resources.resolve(root.as_ref());
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
        while let Some(path) = pending.pop() {
            if !visited.insert(path.clone()) {
                continue;
            }
            let source = self.resources.source(&path)?;
            let analysis = self.analyze(&path, &source)?;
            for target in crate::vault_index::linked_paths(&path, analysis.root()) {
                let resource = self.resources.resolve(&target);
                if !visited.contains(&resource)
                    && self.pipeline.supports(&resource)
                    && self.resources.kind(&resource)? == Some(ResourceKind::File)
                {
                    pending.push(resource);
                }
            }
            documents.push((path, analysis));
        }
        Ok(crate::VaultIndex::from_documents(&root, documents))
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
) -> Result<HtmlRegistry, Vec<SourceDiagnostic>> {
    let mut bindings = BTreeMap::new();
    let mut errors = Vec::new();
    for definition in environment
        .registry()
        .functions()
        .filter(|definition| definition.id.package != "notist")
    {
        let package = &environment.packages()[&definition.id.package];
        let result = (|| -> Result<Option<ModuleLocator>, String> {
            let entries = notist_html::components::component_entries(&definition.id.name)
                .map(|entry| resources.resolve(&package.root.join(entry)));
            let available = [
                resources.kind(&entries[0]).map_err(|e| e.to_string())? == Some(ResourceKind::File),
                resources.kind(&entries[1]).map_err(|e| e.to_string())? == Some(ResourceKind::File),
            ];
            let entry =
                notist_html::components::select_component_entry(&definition.id.name, available)?;
            Ok(entry.map(|entry| {
                let path = resources.resolve(&package.root.join(entry));
                match urls.get(&path) {
                    Some(url) => ModuleLocator::Url(url.clone()),
                    None => ModuleLocator::Resource(path),
                }
            }))
        })();
        match result {
            Ok(Some(module)) => {
                bindings.insert(definition.id.clone(), module);
            }
            Ok(None) => {}
            Err(message) => errors.push(issue(
                &package.root.join("lib.notc"),
                &package.source,
                definition.span,
                message,
            )),
        }
    }
    match HtmlRegistry::from_bindings(environment.registry(), &bindings) {
        Ok(registry) if errors.is_empty() => Ok(registry),
        Ok(_) => Err(errors),
        Err(binding_errors) => {
            for error in binding_errors {
                let package = &environment.packages()[&error.id.package];
                let span = environment.registry().get(&error.id).unwrap().span;
                errors.push(issue(
                    &package.root.join("lib.notc"),
                    &package.source,
                    span,
                    error.message,
                ));
            }
            Err(errors)
        }
    }
}
