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

mod config;
mod loader;
pub use config::{Configuration, Dependency, PackageMetadata, parse_config};

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
        loader::load(resources, config_path.as_ref())
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
