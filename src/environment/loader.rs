//! Source import graph loading through the host's immutable resource view.
use super::*;
use crate::{Import, Pipeline, transforms::Replace};
use std::collections::BTreeSet;

struct Graph<'a, R: Resources + ?Sized> {
    resources: &'a R,
    pipeline: &'a Pipeline,
    visiting: BTreeSet<PathBuf>,
    modules: BTreeMap<String, ModuleSource>,
    context: Context,
    errors: Vec<SourceDiagnostic>,
}
impl<'a, R: Resources + ?Sized> Graph<'a, R> {
    fn new(resources: &'a R, pipeline: &'a Pipeline) -> Self {
        Self {
            resources,
            pipeline,
            visiting: Default::default(),
            modules: Default::default(),
            context: Context::default(),
            errors: Vec::new(),
        }
    }
    fn imports(&mut self, path: &Path, source: &str, imports: &[Import]) -> Vec<(String, String)> {
        let mut bindings = Vec::new();
        let mut aliases = BTreeSet::new();
        for import in imports {
            if !aliases.insert(&import.alias) {
                self.errors.push(issue(
                    path,
                    source,
                    import.span,
                    "duplicate import alias".into(),
                ));
                continue;
            }
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
            }
            bindings.push((import.alias.clone(), target.to_string_lossy().into_owned()));
        }
        bindings
    }
    fn visit(&mut self, path: &Path) {
        let id = path.to_string_lossy().into_owned();
        if self.modules.contains_key(&id) {
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
        let source = match self.resources.source(path) {
            Ok(source) => source,
            Err(e) => {
                self.errors
                    .push(issue(path, "", TextRange::empty(0.into()), e.to_string()));
                self.visiting.remove(path);
                return;
            }
        };
        let mut module = match self
            .pipeline
            .compile_module("content.not", &id, &id, &source)
        {
            Ok(module) => module,
            Err(errors) => {
                self.errors
                    .extend(errors.into_iter().map(|diagnostic| SourceDiagnostic {
                        path: path.into(),
                        source: source.clone(),
                        diagnostic,
                    }));
                self.visiting.remove(path);
                return;
            }
        };
        let text = module.program.sources()[&id].clone();
        for (alias, target) in self.imports(path, &source, &module.imports) {
            module.bind_import(&alias, target);
        }
        match std::mem::take(&mut self.context).with_module(module) {
            Ok(context) => {
                self.context = context;
                self.modules.insert(
                    id,
                    ModuleSource {
                        path: path.into(),
                        source: text,
                    },
                );
            }
            Err(errors) => {
                self.errors
                    .extend(errors.into_iter().map(|diagnostic| SourceDiagnostic {
                        path: path.into(),
                        source: source.clone(),
                        diagnostic,
                    }))
            }
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
    path: &Path,
    source: &str,
    imports: &[Import],
    config: Option<&Path>,
) -> Result<Environment, Vec<SourceDiagnostic>> {
    let path = resources.resolve(path);
    let mut graph = Graph::new(resources, pipeline);
    let bindings = graph.imports(&path, source, imports);
    if let Some(config) = config {
        let source = resources
            .source(config)
            .map_err(|e| vec![issue(config, "", TextRange::empty(0.into()), e.to_string())])?;
        let parsed = parse_config(&source).map_err(|errors| {
            errors
                .into_iter()
                .map(|diagnostic| SourceDiagnostic {
                    path: config.into(),
                    source: source.clone(),
                    diagnostic,
                })
                .collect::<Vec<_>>()
        })?;
        let mut rules = Vec::new();
        for rule in parsed.transforms {
            let before = graph.errors.len();
            let imports = graph
                .imports(config, &source, &rule.imports)
                .into_iter()
                .collect();
            if graph.errors.len() != before {
                continue;
            }
            match (
                graph.context.resolve_imports(&imports, &rule.from),
                graph.context.resolve_imports(&imports, &rule.to),
            ) {
                (Some(from), Some(to)) => rules.push(Replace {
                    from: from.id.clone(),
                    to: to.id.clone(),
                    span: rule.span,
                }),
                _ => graph.errors.push(issue(
                    config,
                    &source,
                    rule.span,
                    "unknown function in transform binding".into(),
                )),
            }
        }
        match std::mem::take(&mut graph.context).with_transforms(&rules) {
            Ok(context) => graph.context = context,
            Err(errors) => {
                graph
                    .errors
                    .extend(errors.into_iter().map(|diagnostic| SourceDiagnostic {
                        path: config.into(),
                        source: source.clone(),
                        diagnostic,
                    }))
            }
        }
    }
    graph.context = graph
        .context
        .with_scope(path.to_string_lossy().into_owned(), bindings);
    graph.finish(config.map(Path::to_path_buf))
}
