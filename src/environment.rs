//! Declaration environment assembly through host-provided resources.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::resources::{ResourceKind, Resources};
use crate::{Context, Diagnostic, Phase, TextRange};

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

/// A host-loaded Context and the configuration/package metadata used for resources.
/// Assembly errors never return a partial environment.
#[derive(Debug, Clone)]
pub struct Environment {
    config_path: Option<PathBuf>,
    packages: BTreeMap<String, Package>,
    context: Context,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            config_path: None,
            packages: BTreeMap::new(),
            context: Context::default(),
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
    /// Pure analysis and transform inputs, independent of host resource paths.
    pub fn context(&self) -> &Context {
        &self.context
    }
    /// Install explicitly supplied declaration sources; performs no IO.
    pub fn from_packages(
        packages: impl IntoIterator<Item = Package>,
    ) -> Result<Self, Vec<SourceDiagnostic>> {
        let mut context = Context::default();
        let mut installed = BTreeMap::new();
        let mut diagnostics = Vec::new();
        for package in packages {
            let result = crate::Pipeline::default()
                .compile_module(
                    "content.not",
                    &package.name,
                    &package.root.join("lib.notc").to_string_lossy(),
                    &package.source,
                )
                .and_then(|module| context.clone().with_module(module));
            match result {
                Ok(updated) => {
                    context = updated;
                    installed.insert(package.name.clone(), package);
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
            Ok(Self {
                config_path: None,
                packages: installed,
                context,
            })
        } else {
            Err(diagnostics)
        }
    }

    /// Add an explicitly prepared Code source to this host environment. The
    /// caller chooses the content dialect and performs any resource reads.
    pub fn with_code(
        mut self,
        package: &str,
        path: &Path,
        source: &str,
        dialect: &Path,
    ) -> Result<Self, Vec<SourceDiagnostic>> {
        let compiled = crate::Pipeline::default()
            .compile_module(dialect, package, &path.to_string_lossy(), source)
            .and_then(|module| self.context.clone().with_module(module));
        match compiled {
            Ok(context) => {
                self.context = context;
                self.packages.insert(
                    package.into(),
                    Package {
                        name: package.into(),
                        root: path.parent().unwrap_or(Path::new("")).into(),
                        source: source.into(),
                    },
                );
                Ok(self)
            }
            Err(errors) => Err(errors
                .into_iter()
                .map(|diagnostic| SourceDiagnostic {
                    path: path.into(),
                    source: source.into(),
                    diagnostic,
                })
                .collect()),
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
