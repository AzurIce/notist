//! Manifest graph assembly and configuration scope. IO stays in the host.
use super::{Configuration, Dependency, Environment, Package, issue, parse_config};
use crate::resources::Resources;
use crate::{Diagnostic, FunctionId, Registry, SourceDiagnostic, TextRange};
use notist_pipeline::transforms::{Replace, TransformPlan};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone)]
struct Manifest {
    path: PathBuf,
    source: String,
    config: Configuration,
}
#[derive(Clone)]
struct Rule {
    value: Replace,
    path: PathBuf,
    source: String,
}
impl Manifest {
    fn rules(&self, values: &[Replace]) -> Vec<Rule> {
        values
            .iter()
            .cloned()
            .map(|value| Rule {
                value,
                path: self.path.clone(),
                source: self.source.clone(),
            })
            .collect()
    }
    fn issue(&self, span: TextRange, message: impl Into<String>) -> SourceDiagnostic {
        issue(&self.path, &self.source, span, message.into())
    }
}
fn read(
    resources: &(impl Resources + ?Sized),
    path: &Path,
) -> Result<Manifest, Vec<SourceDiagnostic>> {
    let source = resources.source(path).map_err(|error| {
        vec![issue(
            path,
            "",
            TextRange::empty(0.into()),
            format!("cannot load `{}`: {error}", path.display()),
        )]
    })?;
    let config = parse_config(&source).map_err(|errors| {
        errors
            .into_iter()
            .map(|diagnostic| SourceDiagnostic {
                path: path.into(),
                source: source.clone(),
                diagnostic,
            })
            .collect::<Vec<_>>()
    })?;
    Ok(Manifest {
        path: path.into(),
        source,
        config,
    })
}

struct Graph<'a, R: Resources + ?Sized> {
    resources: &'a R,
    manifests: BTreeMap<PathBuf, Manifest>,
    names: BTreeMap<String, PathBuf>,
    visiting: Vec<PathBuf>,
    packages: Vec<Package>,
    defaults: Vec<Rule>,
    active_transforms: BTreeSet<PathBuf>,
    errors: Vec<SourceDiagnostic>,
}
impl<R: Resources + ?Sized> Graph<'_, R> {
    fn dependency(&mut self, owner: &Manifest, dependency: &Dependency) {
        let root = self
            .resources
            .resolve(&owner.path.parent().unwrap().join(&dependency.path));
        let path = root.join("Notist.toml");
        if self.visiting.contains(&path) {
            self.errors.push(owner.issue(
                dependency.span,
                format!("package dependency cycle through `{}`", path.display()),
            ));
            return;
        }
        let manifest = if let Some(manifest) = self.manifests.get(&path) {
            manifest.clone()
        } else {
            match read(self.resources, &path) {
                Ok(manifest) => manifest,
                Err(errors) => {
                    // Missing manifests point at the dependency that requested them.
                    if errors.iter().all(|error| error.source.is_empty()) {
                        self.errors.push(
                            owner.issue(dependency.span, errors[0].diagnostic.message.clone()),
                        );
                    } else {
                        self.errors.extend(errors);
                    }
                    return;
                }
            }
        };
        let Some(metadata) = &manifest.config.package else {
            self.errors.push(manifest.issue(
                TextRange::empty(0.into()),
                "dependency package requires [package].name",
            ));
            return;
        };
        if metadata.name != dependency.name {
            self.errors.push(owner.issue(
                dependency.span,
                format!(
                    "dependency key `{}` does not match package name `{}` in `{}`",
                    dependency.name,
                    metadata.name,
                    path.display()
                ),
            ));
            return;
        }
        if let Some(previous) = self.names.get(&metadata.name) {
            if previous != &path {
                self.errors.push(owner.issue(
                    dependency.span,
                    format!(
                        "package name `{}` is provided by both `{}` and `{}`",
                        metadata.name,
                        previous.display(),
                        path.display()
                    ),
                ));
                return;
            }
        }
        if dependency.transforms {
            self.active_transforms.insert(path.clone());
        }
        if !self.manifests.contains_key(&path) {
            self.visit(manifest, false);
        }
    }
    fn visit(&mut self, manifest: Manifest, root: bool) {
        self.visiting.push(manifest.path.clone());
        self.manifests
            .insert(manifest.path.clone(), manifest.clone());
        if let Some(metadata) = &manifest.config.package {
            self.names
                .insert(metadata.name.clone(), manifest.path.clone());
            let directory = manifest.path.parent().unwrap();
            match self.resources.source(&directory.join("lib.notc")) {
                Ok(source) => self.packages.push(Package {
                    name: metadata.name.clone(),
                    root: directory.into(),
                    source,
                }),
                Err(error) => self.errors.push(manifest.issue(
                    metadata.span,
                    format!(
                        "cannot load `{}`: {error}",
                        directory.join("lib.notc").display()
                    ),
                )),
            }
        }
        for dependency in &manifest.config.dependencies {
            self.dependency(&manifest, dependency);
        }
        if root {
            for dependency in &manifest.config.dev_dependencies {
                self.dependency(&manifest, dependency);
            }
        } else {
            self.defaults
                .extend(manifest.rules(&manifest.config.transforms));
        }
        self.visiting.pop();
    }
}

pub(super) fn load(
    resources: &(impl Resources + ?Sized),
    path: &Path,
) -> Result<Environment, Vec<SourceDiagnostic>> {
    let root = read(resources, &resources.resolve(path))?;
    let mut graph = Graph {
        resources,
        manifests: BTreeMap::new(),
        names: BTreeMap::new(),
        visiting: Vec::new(),
        packages: Vec::new(),
        defaults: Vec::new(),
        active_transforms: BTreeSet::new(),
        errors: Vec::new(),
    };
    graph.visit(root.clone(), true);
    let environment = match Environment::from_packages(graph.packages) {
        Ok(environment) => environment,
        Err(errors) => {
            graph.errors.extend(errors);
            return Err(graph.errors);
        }
    };
    let explicit = root.rules(&root.config.transforms);
    let development = root.rules(&root.config.dev_transforms);
    // Activation belongs to dependency edges; a shared package contributes
    // defaults when any importing edge enables them. Filter after the whole
    // graph is loaded so the first visit does not determine the final scope.
    let defaults = graph
        .defaults
        .into_iter()
        .filter(|rule| graph.active_transforms.contains(&rule.path))
        .collect();
    let rules = compose(&environment.registry, defaults, explicit, development);
    match rules {
        Ok(rules) if graph.errors.is_empty() => {
            let mut environment = environment
                .with_transforms(&rules)
                .expect("rules were validated against this immutable registry");
            environment.config_path = Some(root.path);
            Ok(environment)
        }
        Ok(_) => Err(graph.errors),
        Err(errors) => {
            graph.errors.extend(errors);
            Err(graph.errors)
        }
    }
}

fn compose(
    registry: &Registry,
    defaults: Vec<Rule>,
    explicit: Vec<Rule>,
    development: Vec<Rule>,
) -> Result<Vec<Replace>, Vec<SourceDiagnostic>> {
    let mut errors = Vec::new();
    // Validate all active declarations at their own source, even when a root
    // override selects another default. Inactive dependency dev rules are absent.
    for rule in defaults.iter().chain(&explicit).chain(&development) {
        if let Err(diagnostics) =
            TransformPlan::compile(std::slice::from_ref(&rule.value), registry)
        {
            errors.extend(
                diagnostics
                    .into_iter()
                    .map(|diagnostic: Diagnostic| SourceDiagnostic {
                        path: rule.path.clone(),
                        source: rule.source.clone(),
                        diagnostic,
                    }),
            );
        }
    }
    let overridden = |from: &FunctionId| {
        explicit
            .iter()
            .chain(&development)
            .any(|rule| &rule.value.from == from)
    };
    let mut selected: BTreeMap<FunctionId, Rule> = BTreeMap::new();
    let mut rules = Vec::new();
    for rule in defaults {
        if overridden(&rule.value.from) {
            continue;
        }
        match selected.get(&rule.value.from) {
            None => {
                rules.push(rule.value.clone());
                selected.insert(rule.value.from.clone(), rule);
            }
            Some(previous) if previous.value.to == rule.value.to => {}
            Some(previous) => errors.push(issue(
                &rule.path,
                &rule.source,
                rule.value.span,
                format!(
                    "conflicting default transforms for `{}`: `{}` from `{}` and `{}` from `{}`; select a target in the root configuration",
                    rule.value.from,
                    previous.value.to,
                    previous.path.display(),
                    rule.value.to,
                    rule.path.display(),
                ),
            )),
        }
    }
    // Preserve root declaration order. Development overrides suppress public
    // root rules with the same source identity, then run after other root rules.
    rules.extend(
        explicit
            .into_iter()
            .filter(|rule| {
                !development
                    .iter()
                    .any(|dev| dev.value.from == rule.value.from)
            })
            .map(|rule| rule.value),
    );
    rules.extend(development.into_iter().map(|rule| rule.value));
    if errors.is_empty() {
        Ok(rules)
    } else {
        Err(errors)
    }
}
