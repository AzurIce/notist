//! Configuration syntax and source spans; performs no resource loading.
use super::range;
use crate::{Diagnostic, Phase, TextRange};
use notist_pipeline::transforms::Replace;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub path: PathBuf,
    pub span: TextRange,
}

#[derive(serde::Deserialize)]
struct Config {
    package: Option<toml::Spanned<Metadata>>,
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Spanned<LocalDependency>>,
    #[serde(default)]
    transforms: Vec<toml::Spanned<TransformConfig>>,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: BTreeMap<String, toml::Spanned<LocalDependency>>,
    #[serde(default, rename = "dev-transforms")]
    dev_transforms: Vec<toml::Spanned<TransformConfig>>,
}
#[derive(serde::Deserialize)]
struct Metadata {
    name: toml::Spanned<String>,
}

#[derive(Debug, Clone)]
pub struct PackageMetadata {
    pub name: String,
    pub span: TextRange,
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
    pub package: Option<PackageMetadata>,
    pub dependencies: Vec<Dependency>,
    pub transforms: Vec<Replace>,
    pub dev_dependencies: Vec<Dependency>,
    pub dev_transforms: Vec<Replace>,
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
    let mut diagnostics = Vec::new();
    let package = config.package.map(|entry| {
        let name = entry.into_inner().name;
        let span = range(name.span());
        if name.get_ref() == "notist" || !notist_core::definitions::valid_name(name.get_ref()) {
            diagnostics.push(Diagnostic::new(
                Phase::Semantic,
                span,
                format!("invalid or reserved package name `{}`", name.get_ref()),
            ));
        }
        PackageMetadata {
            name: name.into_inner(),
            span,
        }
    });
    let dependencies = parse_dependencies(config.dependencies, &mut diagnostics);
    let dev_dependencies = parse_dependencies(config.dev_dependencies, &mut diagnostics);
    let transforms = parse_transforms(config.transforms, &mut diagnostics);
    let dev_transforms = parse_transforms(config.dev_transforms, &mut diagnostics);
    if diagnostics.is_empty() {
        Ok(Configuration {
            package,
            dependencies,
            transforms,
            dev_dependencies,
            dev_transforms,
        })
    } else {
        Err(diagnostics)
    }
}

fn parse_dependencies(
    entries: BTreeMap<String, toml::Spanned<LocalDependency>>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Dependency> {
    let mut dependencies = Vec::new();
    for (name, entry) in entries {
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
    dependencies
}
fn parse_transforms(
    entries: Vec<toml::Spanned<TransformConfig>>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Replace> {
    let mut transforms = Vec::new();
    for entry in entries {
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
    transforms
}
