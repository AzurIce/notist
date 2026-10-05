//! Declaration environment assembly through host-provided resources.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::resources::{ResourceKind, Resources};
use crate::{Diagnostic, Phase, Registry, TextRange, analyze_module, builtins};
use notist_pipeline::transforms::{Replace, TransformPlan};

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
struct Config {
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Spanned<LocalDependency>>,
    #[serde(default)]
    transforms: Vec<toml::Spanned<TransformConfig>>,
}
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum TransformConfig {
    Replace { from: String, to: String },
}
#[derive(serde::Deserialize)]
struct LocalDependency {
    path: String,
}

#[derive(Debug, Clone)]
pub struct Configuration {
    pub dependencies: Vec<Dependency>,
    pub transforms: Vec<Replace>,
}

/// Parse configuration without reading packages or resolving function identities.
/// Unknown fields are ignored; known fields and transform kinds are validated.
pub fn parse_config(source: &str) -> Result<Configuration, Vec<Diagnostic>> {
    let config: Config = toml::from_str(source).map_err(|error| {
        let span = error.span().unwrap_or(0..0);
        vec![Diagnostic::new(
            Phase::Semantic,
            range(span),
            error.message(),
        )]
    })?;
    let mut dependencies = Vec::new();
    let mut transforms = Vec::new();
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
    for entry in config.transforms {
        let span = range(entry.span());
        let TransformConfig::Replace { from, to } = entry.into_inner();
        let mut identity = |name: &str| {
            let parts: Vec<_> = name.split("::").collect();
            if parts.len() == 2
                && parts
                    .iter()
                    .all(|part| notist_core::definitions::valid_name(part))
            {
                Some(crate::FunctionId::new(parts[0], parts[1]))
            } else {
                diagnostics.push(Diagnostic::new(
                    Phase::Semantic,
                    span,
                    format!("transform identity must be `package::function`: `{name}`"),
                ));
                None
            }
        };
        let from = identity(&from);
        let to = identity(&to);
        if let (Some(from), Some(to)) = (from, to) {
            transforms.push(Replace { from, to, span });
        }
    }
    if diagnostics.is_empty() {
        Ok(Configuration {
            dependencies,
            transforms,
        })
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

/// A fully installed environment. Assembly errors never return a partial environment.
#[derive(Debug, Clone)]
pub struct Environment {
    config_path: Option<PathBuf>,
    packages: BTreeMap<String, Package>,
    registry: Registry,
    transforms: TransformPlan,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            config_path: None,
            packages: BTreeMap::new(),
            registry: builtins::registry().clone(),
            transforms: TransformPlan::default(),
        }
    }
}

impl Environment {
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }
    pub fn packages(&self) -> &BTreeMap<String, Package> {
        &self.packages
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    pub fn transforms(&self) -> &TransformPlan {
        &self.transforms
    }
    /// Compile an explicit plan against the installed declarations, without IO.
    pub fn with_transforms(mut self, rules: &[Replace]) -> Result<Self, Vec<Diagnostic>> {
        self.transforms = TransformPlan::compile(rules, &self.registry)?;
        Ok(self)
    }
    /// Install explicitly supplied declaration sources; performs no IO.
    pub fn from_packages(
        packages: impl IntoIterator<Item = Package>,
    ) -> Result<Self, Vec<SourceDiagnostic>> {
        let mut environment = Self::default();
        let mut diagnostics = Vec::new();
        for package in packages {
            let result = analyze_module(&package.name, &package.source)
                .and_then(|module| environment.registry.register(module));
            match result {
                Ok(()) => {
                    environment.packages.insert(package.name.clone(), package);
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
            Ok(environment)
        } else {
            Err(diagnostics)
        }
    }

    /// Assemble declarations through a host-provided read-only resource snapshot.
    pub fn load_from(
        resources: &(impl Resources + ?Sized),
        config_path: impl AsRef<Path>,
    ) -> Result<Self, Vec<SourceDiagnostic>> {
        let config_path = resources.resolve(config_path.as_ref());
        let config_source = resources.source(&config_path).map_err(|message| {
            vec![issue(
                &config_path,
                "",
                TextRange::empty(0.into()),
                message.to_string(),
            )]
        })?;
        let config = parse_config(&config_source).map_err(|errors| {
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
        for dependency in config.dependencies {
            let root =
                crate::resources::normalize(&config_path.parent().unwrap().join(&dependency.path));
            let entry = root.join("lib.notc");
            match resources.source(&entry) {
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
            Ok(environment) => match environment.with_transforms(&config.transforms) {
                Ok(mut environment) if diagnostics.is_empty() => {
                    environment.config_path = Some(config_path);
                    Ok(environment)
                }
                Ok(_) => Err(diagnostics),
                Err(errors) => {
                    diagnostics.extend(errors.into_iter().map(|diagnostic| SourceDiagnostic {
                        path: config_path.clone(),
                        source: config_source.clone(),
                        diagnostic,
                    }));
                    Err(diagnostics)
                }
            },
            Err(mut errors) => {
                diagnostics.append(&mut errors);
                Err(diagnostics)
            }
        }
    }
}

/// Discover one nearest configuration. Access failures are never treated as absence.
pub fn discover_config_in(
    resources: &(impl Resources + ?Sized),
    document: &Path,
) -> Result<Option<PathBuf>, crate::resources::ResourceError> {
    let document = resources.resolve(document);
    let directory = if resources.kind(&document)? == Some(ResourceKind::Directory) {
        document.as_path()
    } else {
        document.parent().unwrap_or(resources.root())
    };
    for directory in directory.ancestors() {
        let config = directory.join("Notist.toml");
        if resources.kind(&config)? == Some(ResourceKind::File) {
            return Ok(Some(config));
        }
    }
    Ok(None)
}
fn range(span: std::ops::Range<usize>) -> TextRange {
    TextRange::new((span.start as u32).into(), (span.end as u32).into())
}
pub(crate) fn issue(
    path: &Path,
    source: &str,
    span: TextRange,
    message: String,
) -> SourceDiagnostic {
    SourceDiagnostic {
        path: path.into(),
        source: source.into(),
        diagnostic: Diagnostic::new(Phase::Semantic, span, message),
    }
}
