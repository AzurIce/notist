//! Source import graph loading through the host's immutable resource view.
use super::*;
use crate::{
    Import, Pipeline,
    extensions::{DocumentExtension, Replace, Replacement},
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GraphKey {
    directory: PathBuf,
    imports: Vec<String>,
    config: Option<PathBuf>,
}
#[derive(Clone)]
struct LoadedGraph {
    modules: Arc<BTreeMap<String, ModuleSource>>,
    context: Context,
}
/// Batch-local successful graph/config reuse, never document lexical scopes.
#[derive(Default)]
pub(crate) struct EnvironmentCache {
    graphs: BTreeMap<GraphKey, LoadedGraph>,
    configs: BTreeMap<PathBuf, Result<Arc<Configuration>, Vec<Diagnostic>>>,
}

struct Graph<'a, R: Resources + ?Sized> {
    resources: &'a R,
    pipeline: &'a Pipeline,
    visiting: BTreeSet<PathBuf>,
    verified: BTreeSet<String>,
    modules: Arc<BTreeMap<String, ModuleSource>>,
    context: Context,
    errors: Vec<SourceDiagnostic>,
}
impl<'a, R: Resources + ?Sized> Graph<'a, R> {
    fn new(resources: &'a R, pipeline: &'a Pipeline) -> Self {
        Self {
            resources,
            pipeline,
            visiting: Default::default(),
            verified: Default::default(),
            modules: Default::default(),
            context: Context::default().with_resource_root(resources.root().to_string_lossy()),
            errors: Vec::new(),
        }
    }
    fn resolve_imports<'b>(
        &mut self,
        path: &Path,
        source: &str,
        imports: &'b [Import],
    ) -> Vec<(&'b Import, String)> {
        let mut bindings = Vec::new();
        for import in imports {
            if Path::new(&import.path).is_absolute()
                || import.path.starts_with('@')
                || import
                    .path
                    .split('/')
                    .next()
                    .is_some_and(|s| s.contains(':'))
            {
                self.errors.push(issue(
                    path,
                    source,
                    import.span,
                    "import requires a relative local module path".into(),
                ));
                continue;
            }
            let target = self
                .resources
                .resolve(&path.parent().unwrap().join(&import.path));
            if !target.extension().is_some_and(|ext| ext == "notc") {
                self.errors.push(issue(
                    path,
                    source,
                    import.span,
                    "import target must be a .notc source file".into(),
                ));
                continue;
            }
            if self.visiting.contains(&target) {
                self.errors.push(issue(
                    path,
                    source,
                    import.span,
                    format!("circular module import: {}", target.display()),
                ));
                continue;
            }
            let before = self.errors.len();
            self.visit(&target);
            if self.errors.len() > before {
                self.errors.push(issue(
                    path,
                    source,
                    import.span,
                    format!("cannot import `{}`", import.path),
                ));
                continue;
            }
            bindings.push((import, target.to_string_lossy().into_owned()));
        }
        bindings
    }
    fn imports(
        &mut self,
        path: &Path,
        source: &str,
        imports: &[Import],
    ) -> BTreeMap<String, crate::ImportBinding> {
        let bindings = self.resolve_imports(path, source, imports);
        match self.context.import_bindings(bindings, None) {
            Ok(bindings) => bindings,
            Err(errors) => {
                self.errors.extend(
                    errors
                        .into_iter()
                        .map(|diagnostic| SourceDiagnostic::locate(path, source, diagnostic)),
                );
                BTreeMap::new()
            }
        }
    }
    fn visit(&mut self, path: &Path) {
        let id = path.to_string_lossy().into_owned();
        if self.verified.contains(&id) {
            return;
        }
        if !path.extension().is_some_and(|ext| ext == "notc") {
            self.errors.push(issue(
                path,
                "",
                TextRange::empty(0.into()),
                "module entry must be a .notc source file".into(),
            ));
            return;
        }
        if self.visiting.len() >= 128 {
            self.errors.push(issue(
                path,
                "",
                TextRange::empty(0.into()),
                "module import depth limit exceeded".into(),
            ));
            return;
        }
        self.visiting.insert(path.into());
        let source = match self.resources.source_shared(path) {
            Ok(source) => source,
            Err(e) => {
                self.errors
                    .push(issue(path, "", TextRange::empty(0.into()), e.to_string()));
                self.visiting.remove(path);
                return;
            }
        };
        // Reuse assembly, not authorization or request accounting. Visit the
        // same source dependency graph even when its immutable Context is shared.
        if let Some(linked) = self.context.program().module(&id).cloned() {
            let before = self.errors.len();
            self.resolve_imports(path, &source, &linked.code().imports);
            if self.errors.len() == before {
                self.verified.insert(id);
            }
            self.visiting.remove(path);
            return;
        }
        let module = match self.pipeline.compile_module_with_root(
            "content.not",
            &id,
            &id,
            &source,
            &self.resources.source_root(path).to_string_lossy(),
        ) {
            Ok(module) => module,
            Err(errors) => {
                self.errors.extend(
                    errors
                        .into_iter()
                        .map(|diagnostic| SourceDiagnostic::locate(path, &source, diagnostic)),
                );
                self.visiting.remove(path);
                return;
            }
        };
        let before = self.errors.len();
        let targets = self.resolve_imports(path, &source, &module.imports);
        if self.errors.len() != before {
            self.visiting.remove(path);
            return;
        }
        let targets: Vec<_> = targets.into_iter().map(|(_, target)| target).collect();
        let text = module.source().clone();
        let linked = match self.context.link_module(module, targets) {
            Ok(linked) => linked,
            Err(errors) => {
                self.errors.extend(
                    errors
                        .into_iter()
                        .map(|diagnostic| SourceDiagnostic::locate(path, &source, diagnostic)),
                );
                self.visiting.remove(path);
                return;
            }
        };
        match std::mem::take(&mut self.context).with_linked_module(linked) {
            Ok(context) => {
                self.context = context;
                self.verified.insert(id.clone());
                Arc::make_mut(&mut self.modules).insert(
                    id,
                    ModuleSource {
                        path: path.into(),
                        source: text,
                    },
                );
            }
            Err(errors) => self.errors.extend(
                errors
                    .into_iter()
                    .map(|diagnostic| SourceDiagnostic::locate(path, &source, diagnostic)),
            ),
        }
        self.visiting.remove(path);
    }
    fn finish(self, config_path: Option<PathBuf>) -> Result<Environment, Vec<SourceDiagnostic>> {
        if self.errors.is_empty() {
            Ok(Environment {
                config_path,
                modules: self.modules,
                context: self.context,
            })
        } else {
            Err(self.errors)
        }
    }
}

pub(super) fn load(
    resources: &(impl Resources + ?Sized),
    input: &Path,
) -> Result<Environment, Vec<SourceDiagnostic>> {
    let pipeline = Pipeline::default();
    let mut graph = Graph::new(resources, &pipeline);
    let input = resources.resolve(input);
    graph.visit(&input);
    graph.context = graph
        .context
        .with_root(input.to_string_lossy().into_owned());
    graph.finish(None)
}

pub(super) fn document(
    resources: &(impl Resources + ?Sized),
    pipeline: &Pipeline,
    document: DocumentSource<'_>,
    config: Option<&Path>,
    extensions: &[Arc<dyn DocumentExtension>],
    cache: &mut EnvironmentCache,
) -> Result<Environment, Vec<SourceDiagnostic>> {
    let DocumentSource {
        path,
        text: source,
        imports,
    } = document;
    let path = resources.resolve(path);
    let mut graph = Graph::new(resources, pipeline);
    let key = pipeline
        .module_cache_key(Path::new("content.not"))
        .map(|_| GraphKey {
            directory: path.parent().unwrap().into(),
            imports: imports.iter().map(|import| import.path.clone()).collect(),
            config: config.map(Path::to_path_buf),
        });
    if let Some(loaded) = key.as_ref().and_then(|key| cache.graphs.get(key)) {
        graph.modules = loaded.modules.clone();
        graph.context = loaded.context.clone();
    }
    let bindings = graph.imports(&path, source, imports);
    let mut configured: Vec<(Arc<dyn DocumentExtension>, TextRange)> = Vec::new();
    let mut config_source = None;
    if let Some(config) = config {
        let source = resources
            .source_shared(config)
            .map_err(|e| vec![issue(config, "", TextRange::empty(0.into()), e.to_string())])?;
        let parsed = match cache.configs.get(config) {
            Some(parsed) => parsed.clone(),
            None => {
                let parsed = parse_config(&source).map(Arc::new);
                if cache.configs.len() < 128 {
                    cache.configs.insert(config.into(), parsed.clone());
                }
                parsed
            }
        }
        .map_err(|errors| {
            errors
                .into_iter()
                .map(|diagnostic| SourceDiagnostic::locate(config, &source, diagnostic))
                .collect::<Vec<_>>()
        })?;
        for binding in parsed.extensions.iter().cloned() {
            match binding {
                ExtensionBinding::Replace {
                    imports,
                    from,
                    to,
                    span,
                } => {
                    let before = graph.errors.len();
                    let imports = graph
                        .imports(config, &source, &imports)
                        .into_iter()
                        .collect();
                    if graph.errors.len() != before {
                        continue;
                    }
                    match (
                        graph.context.resolve_imports(&imports, &from),
                        graph.context.resolve_imports(&imports, &to),
                    ) {
                        (Some(from), Some(to)) => configured.push((
                            Arc::new(Replace::new([Replacement {
                                from: from.id.clone(),
                                to: to.id.clone(),
                                span,
                            }])),
                            span,
                        )),
                        _ => graph.errors.push(issue(
                            config,
                            &source,
                            span,
                            "unknown function in extension binding".into(),
                        )),
                    }
                }
                ExtensionBinding::Command {
                    command,
                    stages,
                    span,
                } => {
                    #[cfg(all(not(target_arch = "wasm32"), feature = "command-extensions"))]
                    configured.push((
                        Arc::new(crate::command_extension::CommandExtension::new(
                            command,
                            stages,
                            config.parent().unwrap().to_path_buf(),
                        )),
                        span,
                    ));
                    #[cfg(not(all(not(target_arch = "wasm32"), feature = "command-extensions")))]
                    {
                        let _ = (command, stages);
                        graph.errors.push(issue(
                            config,
                            &source,
                            span,
                            "command extensions are unavailable in this host".into(),
                        ));
                    }
                }
            }
        }
        config_source = Some(source);
    }
    // Validate only after all document/config imports have been loaded.
    if !graph.errors.is_empty() {
        return Err(graph.errors);
    }
    if let Some(key) = key
        && !cache.graphs.contains_key(&key)
        && cache.graphs.len() < 128
    {
        cache.graphs.insert(
            key,
            LoadedGraph {
                modules: graph.modules.clone(),
                context: graph.context.clone(),
            },
        );
    }
    for extension in extensions {
        match std::mem::take(&mut graph.context).with_shared_extension(extension.clone()) {
            Ok(context) => graph.context = context,
            Err(errors) => {
                return Err(errors
                    .into_iter()
                    .map(|diagnostic| SourceDiagnostic::locate(&path, source, diagnostic))
                    .collect());
            }
        }
    }
    if let (Some(config), Some(source)) = (config, config_source) {
        for (extension, span) in configured {
            match std::mem::take(&mut graph.context).with_shared_extension(extension) {
                Ok(context) => graph.context = context,
                Err(errors) => {
                    return Err(errors
                        .into_iter()
                        .map(|mut diagnostic| {
                            if diagnostic.source.is_none() && diagnostic.span.is_empty() {
                                diagnostic.span = span;
                            }
                            SourceDiagnostic::locate(config, &source, diagnostic)
                        })
                        .collect());
                }
            }
        }
    }
    graph.context = graph
        .context
        .with_scope(path.to_string_lossy().into_owned(), bindings);
    graph.finish(config.map(Path::to_path_buf))
}
