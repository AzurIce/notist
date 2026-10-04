//! Explicit project assembly and local filesystem loading.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{Diagnostic, Phase, Registry, TextRange, analyze_module, builtins};

#[derive(Debug, Clone)]
pub struct SourceDiagnostic {
    pub path: PathBuf,
    pub source: String,
    pub diagnostic: Diagnostic,
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub path: PathBuf,
    pub span: TextRange,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Spanned<LocalDependency>>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalDependency {
    path: String,
}

pub fn parse_config(source: &str) -> Result<Vec<Dependency>, Vec<Diagnostic>> {
    let config: Config = toml::from_str(source).map_err(|error| {
        let span = error.span().unwrap_or(0..0);
        vec![Diagnostic::new(
            Phase::Semantic,
            range(span),
            error.message(),
        )]
    })?;
    let mut dependencies = Vec::new();
    let mut diagnostics = Vec::new();
    for (name, entry) in config.dependencies {
        let span = range(entry.span());
        let path = &entry.get_ref().path;
        if name == "notist" || !notist_core::definitions::valid_name(&name) {
            diagnostics.push(Diagnostic::new(
                Phase::Semantic,
                span,
                format!("invalid or reserved package name `{name}`"),
            ));
        } else if path.is_empty() {
            diagnostics.push(Diagnostic::new(
                Phase::Semantic,
                span,
                "dependency path cannot be empty",
            ));
        } else {
            dependencies.push(Dependency {
                name,
                path: path.into(),
                span,
            });
        }
    }
    if diagnostics.is_empty() {
        Ok(dependencies)
    } else {
        Err(diagnostics)
    }
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub root: PathBuf,
    pub source: String,
}

/// A fully installed environment. Assembly errors never return a partial project.
#[derive(Debug, Clone)]
pub struct Project {
    config_path: Option<PathBuf>,
    packages: BTreeMap<String, Package>,
    registry: Registry,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            config_path: None,
            packages: BTreeMap::new(),
            registry: builtins::registry().clone(),
        }
    }
}

impl Project {
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }
    pub fn packages(&self) -> &BTreeMap<String, Package> {
        &self.packages
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    pub fn analyzer(&self) -> crate::Notist {
        crate::Notist::default().with_registry(self.registry.clone())
    }

    /// Install explicitly supplied declaration sources; performs no IO.
    pub fn from_packages(
        packages: impl IntoIterator<Item = Package>,
    ) -> Result<Self, Vec<SourceDiagnostic>> {
        let mut project = Self::default();
        let mut diagnostics = Vec::new();
        for package in packages {
            let result = analyze_module(&package.name, &package.source)
                .and_then(|module| project.registry.register(module));
            match result {
                Ok(()) => {
                    project.packages.insert(package.name.clone(), package);
                }
                Err(errors) => {
                    diagnostics.extend(errors.into_iter().map(|diagnostic| SourceDiagnostic {
                        path: package.root.join("lib.notc"),
                        source: package.source.clone(),
                        diagnostic,
                    }))
                }
            }
        }
        if diagnostics.is_empty() {
            Ok(project)
        } else {
            Err(diagnostics)
        }
    }

    /// Read one project configuration and its direct local dependencies.
    pub fn load(config_path: impl AsRef<Path>) -> Result<Self, Vec<SourceDiagnostic>> {
        Self::load_with_sources(config_path, &BTreeMap::new())
    }

    /// Filesystem host with editor overlays, keyed by absolute source paths.
    pub fn load_with_sources(
        config_path: impl AsRef<Path>,
        overlays: &BTreeMap<PathBuf, String>,
    ) -> Result<Self, Vec<SourceDiagnostic>> {
        let config_path = absolute(config_path.as_ref());
        let config_source = read_source(&config_path, overlays).map_err(|message| {
            vec![issue(&config_path, "", TextRange::empty(0.into()), message)]
        })?;
        let dependencies = parse_config(&config_source).map_err(|errors| {
            errors
                .into_iter()
                .map(|diagnostic| SourceDiagnostic {
                    path: config_path.clone(),
                    source: config_source.clone(),
                    diagnostic,
                })
                .collect::<Vec<_>>()
        })?;
        let mut packages = Vec::new();
        let mut diagnostics = Vec::new();
        for dependency in dependencies {
            let root =
                crate::vault::normalize(&config_path.parent().unwrap().join(&dependency.path));
            let entry = root.join("lib.notc");
            match read_source(&entry, overlays) {
                Ok(source) => packages.push(Package {
                    name: dependency.name,
                    root,
                    source,
                }),
                Err(message) => diagnostics.push(issue(
                    &config_path,
                    &config_source,
                    dependency.span,
                    format!("cannot load `{}`: {message}", entry.display()),
                )),
            }
        }
        // Still analyze available packages to report every source error together.
        match Self::from_packages(packages) {
            Ok(mut project) if diagnostics.is_empty() => {
                project.config_path = Some(config_path);
                Ok(project)
            }
            Ok(_) => Err(diagnostics),
            Err(mut errors) => {
                diagnostics.append(&mut errors);
                Err(diagnostics)
            }
        }
    }

    pub fn for_document(
        document: impl AsRef<Path>,
        explicit_config: Option<&Path>,
    ) -> Result<Self, Vec<SourceDiagnostic>> {
        if let Some(config) = explicit_config {
            return Self::load(config);
        }
        match discover_config(document.as_ref()) {
            Some(config) => Self::load(config),
            None => Ok(Self::default()),
        }
    }
}

pub fn discover_config(document: &Path) -> Option<PathBuf> {
    discover_config_with_sources(document, &BTreeMap::new())
}

pub fn discover_config_with_sources(
    document: &Path,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<PathBuf> {
    let document = absolute(document);
    let directory = if document.is_dir() {
        document.as_path()
    } else {
        document.parent()?
    };
    directory
        .ancestors()
        .map(|directory| directory.join("Notist.toml"))
        .find(|path| path.is_file() || overlays.contains_key(path))
}
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        crate::vault::normalize(path)
    } else {
        crate::vault::normalize(&std::env::current_dir().unwrap_or_default().join(path))
    }
}
fn read_source(path: &Path, overlays: &BTreeMap<PathBuf, String>) -> Result<String, String> {
    overlays
        .get(path)
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| std::fs::read_to_string(path).map_err(|error| error.to_string()))
}
fn range(span: std::ops::Range<usize>) -> TextRange {
    TextRange::new((span.start as u32).into(), (span.end as u32).into())
}
fn issue(path: &Path, source: &str, span: TextRange, message: String) -> SourceDiagnostic {
    SourceDiagnostic {
        path: path.into(),
        source: source.into(),
        diagnostic: Diagnostic::new(Phase::Semantic, span, message),
    }
}
