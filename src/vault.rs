//! Vault orchestration: resources, declaration environments and typed outputs.
use crate::environment::{discover_config_in, issue};
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
pub use code::CodeAnalysis;

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

/// Analysis, evaluation and render diagnostics refer to this document source. Module/config
/// failures instead return source-bearing VaultError::Environment entries.
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

/// One immutable resource view, potentially containing multiple nearest configs.
/// Environment assembly is retained within this view; replace the Vault when
/// inputs change. No mutable source cache or incremental compiler is implied.
pub struct Vault<R = FsResources> {
    resources: R,
    pipeline: Pipeline,
    environments: BTreeMap<PathBuf, BTreeMap<Arc<str>, Arc<Environment>>>,
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
    /// Whether the configured document pipeline handles this file format.
    pub fn supports(&self, path: impl AsRef<Path>) -> bool {
        self.pipeline.supports(path.as_ref())
    }
    pub fn with_pipeline(mut self, pipeline: Pipeline) -> Self {
        self.pipeline = pipeline;
        self.environments.clear();
        self
    }
    pub fn with_config(mut self, path: impl AsRef<Path>) -> Self {
        self.config = Some(self.resources.resolve(path.as_ref()));
        self.environments.clear();
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
    fn ensure_environment(
        &mut self,
        path: &Path,
        source: &str,
        imports: &[crate::Import],
    ) -> Result<(PathBuf, Arc<str>), VaultError> {
        let path = self.resources.resolve(path);
        if !self
            .environments
            .get(&path)
            .is_some_and(|versions| versions.contains_key(source))
        {
            let config = match &self.config {
                Some(config) => Some(config.clone()),
                None => discover_config_in(&self.resources, &path)?,
            };
            let environment = Environment::for_document(
                &self.resources,
                &self.pipeline,
                &path,
                source,
                imports,
                config.as_deref(),
            )
            .map_err(VaultError::Environment)?;
            self.environments
                .entry(path.clone())
                .or_default()
                .insert(Arc::from(source), Arc::new(environment));
        }
        let text = self.environments[&path]
            .get_key_value(source)
            .unwrap()
            .0
            .clone();
        Ok((path, text))
    }
    fn environment_key(
        &mut self,
        path: &Path,
        source: &str,
    ) -> Result<(PathBuf, Arc<str>), VaultError> {
        let path = self.resources.resolve(path);
        if let Some((text, _)) = self
            .environments
            .get(&path)
            .and_then(|versions| versions.get_key_value(source))
        {
            return Ok((path, text.clone()));
        }
        let lowered = self
            .pipeline
            .lower(&path, source, crate::FrontendOptions::default())?;
        self.ensure_environment(&path, source, &lowered.imports)
    }
    pub fn environment_for(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<Arc<Environment>, VaultError> {
        let (path, text) = self.environment_key(path.as_ref(), source)?;
        Ok(self.environments[&path][text.as_ref()].clone())
    }
    pub fn analyze(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<Analysis, VaultError> {
        self.compile(path.as_ref(), source, false)
            .map(|(analysis, _)| analysis)
    }
    pub fn inspect(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<(Analysis, notist_pipeline::Inspection), VaultError> {
        self.compile(path.as_ref(), source, true)
    }
    fn compile(
        &mut self,
        path: &Path,
        source: &str,
        capture: bool,
    ) -> Result<(Analysis, notist_pipeline::Inspection), VaultError> {
        let path = self.resources.resolve(path);
        let output = self.pipeline.lower(
            &path,
            source,
            crate::FrontendOptions {
                capture_syntax: capture,
            },
        )?;
        let key = self.ensure_environment(&path, source, &output.imports)?;
        let (mut analysis, inspection) = self.pipeline.analyze_lowered(
            &path,
            key.1.clone(),
            output,
            self.environments[&key.0][key.1.as_ref()].context(),
            capture,
        );
        analysis.extend_diagnostics(crate::vault_index::check_content_paths(
            self.resources.root(),
            &path,
            analysis.root(),
        ));
        Ok((analysis, inspection))
    }
    pub fn analyze_resource(&mut self, path: impl AsRef<Path>) -> Result<Analysis, VaultError> {
        let source = self.resources.source(path.as_ref())?;
        self.analyze(path, &source)
    }
    /// Bind only native targets visited by HTML rendering of this evaluated tree.
    pub fn html_registry(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
        root: &crate::Item,
    ) -> Result<HtmlRegistry, VaultError> {
        let (path, text) = self.environment_key(path.as_ref(), source)?;
        bind_html(
            &self.environments[&path][text.as_ref()],
            &self.resources,
            &self.module_urls,
            root,
        )
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
        let (path, text) = self.environment_key(&path, source)?;
        Ok(self
            .pipeline
            .evaluate(analysis, self.environments[&path][text.as_ref()].context()))
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
        let (path, text) = self.environment_key(&path, source)?;
        let environment = &self.environments[&path][text.as_ref()];
        let evaluation = self.pipeline.evaluate(analysis, environment.context());
        let registry = bind_html(
            environment,
            &self.resources,
            &self.module_urls,
            &evaluation.root,
        )
        .map_err(VaultError::Environment)?;
        let mut renderer = Renderer::new().with_registry(registry);
        if options.source_map {
            renderer = renderer.with_source_map();
        }
        let rendered = renderer.render_with_diagnostics(&evaluation.root);
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
    for definition in notist_html::components::component_ids(root)
        .iter()
        .filter_map(|id| environment.context().registry().get(id))
        .filter(|definition| {
            definition.id.module != "notist"
                && !definition.code
                && (definition.returns.level().is_some()
                    || matches!(definition.returns, crate::Type::Parameter(_)))
        })
    {
        let module = &environment.modules()[&definition.id.module];
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
                bindings.insert(definition.id.clone(), module);
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
                let module = &environment.modules()[&error.id.module];
                let span = environment
                    .context()
                    .registry()
                    .get(&error.id)
                    .unwrap()
                    .span;
                errors.push(issue(&module.path, &module.source, span, error.message));
            }
            Err(errors)
        }
    }
}
