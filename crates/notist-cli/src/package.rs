//! Package identity, workspace membership and isolated publication directories.
use crate::publication::Files;
use notist::resources::normalize;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::ExitCode,
};

const OUTPUT: &str = "notist-output";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    package: Option<Metadata>,
    workspace: Option<Workspace>,
    site: Option<toml::Table>,
    #[serde(default)]
    extensions: Vec<toml::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Workspace {
    members: Vec<PathBuf>,
    #[serde(default = "default_output")]
    output: PathBuf,
}
fn default_output() -> PathBuf {
    OUTPUT.into()
}

#[derive(Clone, Debug)]
pub(crate) struct Package {
    manifest: PathBuf,
    workspace: Option<PathBuf>,
    pub name: String,
    pub root: PathBuf,
    output: Option<PathBuf>,
    output_override: Option<PathBuf>,
}

pub(crate) struct Prepared {
    pub files: Files,
    pub config: PathBuf,
    pub options: notist_ssg::SiteConfig,
}

struct Scope {
    workspace: Option<PathBuf>,
    output_root: PathBuf,
    packages: Vec<Package>,
}

fn read(path: &Path) -> Result<Manifest, String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: toml::Value =
        toml::from_str(&source).map_err(|e| format!("{}: {e}", path.display()))?;
    if ["project", "book", "ssg"]
        .iter()
        .any(|key| value.get(key).is_some())
        || value.get("site").is_some_and(toml::Value::is_array)
    {
        return Err(format!(
            "{}: use [package] and optional [site]; project and publication target arrays are not supported",
            path.display()
        ));
    }
    let manifest: Manifest =
        toml::from_str(&source).map_err(|e| format!("{}: {e}", path.display()))?;
    if manifest.package.is_none() && (manifest.site.is_some() || !manifest.extensions.is_empty()) {
        return Err("[site] and document extensions require [package]".into());
    }
    if manifest.package.is_none() && manifest.workspace.is_none() {
        return Err("manifest requires [package] or [workspace]".into());
    }
    if let Some(package) = &manifest.package {
        if package.name.is_empty()
            || !package
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("invalid package name: use ASCII letters, digits, '-' or '_'".into());
        }
    }
    if manifest
        .workspace
        .as_ref()
        .is_some_and(|w| w.output.as_os_str().is_empty())
    {
        return Err("workspace output must not be empty".into());
    }
    Ok(manifest)
}

fn locate(config: Option<&Path>) -> Result<PathBuf, String> {
    let path = match config {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir()
            .map_err(|e| e.to_string())?
            .ancestors()
            .map(|p| p.join("Notist.toml"))
            .find(|p| p.is_file())
            .ok_or("no Notist.toml found in this directory or its ancestors")?,
    };
    std::fs::canonicalize(&path).map_err(|e| format!("{}: {e}", path.display()))
}

// Only an explicit workspace owns members. Unrelated parent packages do not
// supply settings, and their malformed manifests do not invalidate this package.
fn workspace_for(manifest: &Path, local: &Manifest) -> Option<PathBuf> {
    if local.workspace.is_some() {
        return Some(manifest.into());
    }
    manifest
        .parent()?
        .ancestors()
        .skip(1)
        .find_map(|directory| {
            let path = directory.join("Notist.toml");
            let text = std::fs::read_to_string(&path).ok()?;
            let value: toml::Value = toml::from_str(&text).ok()?;
            value.get("workspace").map(|_| path)
        })
}

fn discover(current: &Path) -> Result<Scope, String> {
    let local = read(current)?;
    let workspace = workspace_for(current, &local);
    let mut manifests = BTreeSet::new();
    let output_root = if let Some(path) = &workspace {
        let root = path.parent().unwrap();
        let root_manifest = read(path)?;
        let settings = root_manifest
            .workspace
            .ok_or("workspace configuration changed; retry")?;
        for member in settings.members {
            let candidate = root.join(member).join("Notist.toml");
            let member = std::fs::canonicalize(&candidate)
                .map_err(|e| format!("invalid workspace member {}: {e}", candidate.display()))?;
            if !member.starts_with(root) {
                return Err("workspace members must be inside the workspace root".into());
            }
            if !manifests.insert(member.clone()) {
                return Err(format!("duplicate workspace member: {}", member.display()));
            }
            let member_manifest = read(&member)?;
            if member != *path && member_manifest.workspace.is_some() {
                return Err("nested workspaces are not supported".into());
            }
            if member_manifest.package.is_none() {
                return Err(format!(
                    "workspace member {} requires [package]",
                    member.display()
                ));
            }
        }
        if root_manifest.package.is_some() {
            manifests.insert(path.clone());
        }
        if current != path && !manifests.contains(current) {
            return Err(format!(
                "{} is not a member of workspace {}",
                current.display(),
                path.display()
            ));
        }
        normalize(&root.join(settings.output))
    } else {
        manifests.insert(current.into());
        current.parent().unwrap().join(OUTPUT)
    };
    let mut names = BTreeSet::new();
    let mut packages = vec![];
    for path in manifests {
        let manifest = read(&path)?;
        let name = manifest
            .package
            .ok_or("workspace member requires [package]")?
            .name;
        if !names.insert(name.clone()) {
            return Err(format!("duplicate package name in workspace: {name}"));
        }
        let root = path.parent().unwrap().to_path_buf();
        let output = manifest
            .site
            .map(|site| match site.get("output") {
                None => Ok(output_root.join(&name)),
                Some(toml::Value::String(path)) if !path.is_empty() => {
                    Ok(normalize(&root.join(path)))
                }
                _ => Err("site output must be a nonempty path".to_owned()),
            })
            .transpose()?;
        packages.push(Package {
            manifest: path,
            workspace: workspace.clone(),
            name,
            root,
            output,
            output_override: None,
        });
    }
    let scope = Scope {
        workspace,
        output_root,
        packages,
    };
    scope.validate()?;
    Ok(scope)
}

impl Scope {
    fn validate(&self) -> Result<(), String> {
        if self.workspace.is_some()
            && self
                .packages
                .iter()
                .any(|p| p.root.starts_with(&self.output_root))
        {
            return Err("workspace output collection must not contain package inputs".into());
        }
        let mut outputs = BTreeMap::<&Path, &str>::new();
        for package in &self.packages {
            let Some(output) = package.output.as_deref() else {
                continue;
            };
            if self.packages.iter().any(|p| p.root.starts_with(output))
                || self
                    .workspace
                    .as_ref()
                    .is_some_and(|p| p.starts_with(output))
            {
                return Err(format!(
                    "output would replace package inputs: {}",
                    output.display()
                ));
            }
            for (previous, name) in &outputs {
                if output.starts_with(previous) || previous.starts_with(output) {
                    return Err(format!(
                        "overlapping package outputs: {name} and {}",
                        package.name
                    ));
                }
            }
            outputs.insert(output, &package.name);
        }
        Ok(())
    }
}

fn override_output(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("--out-dir requires a nonempty output path".into());
    }
    Ok(normalize(
        &std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path),
    ))
}

impl Package {
    pub(crate) fn load(
        config: Option<&Path>,
        output: Option<&Path>,
        name: Option<&str>,
    ) -> Result<Self, String> {
        let current = locate(config)?;
        let mut scope = discover(&current)?;
        let index = scope
            .packages
            .iter()
            .position(|p| name.map_or(p.manifest == current, |name| p.name == name))
            .ok_or(
                "package not found or workspace root has no package site; use -p <package> for serve",
            )?;
        if scope.packages[index].output.is_none() {
            return Err(format!(
                "package `{}` has no [site] to build or serve",
                scope.packages[index].name
            ));
        }
        if let Some(path) = output {
            let output = override_output(path)?;
            scope.packages[index].output = Some(output.clone());
            scope.packages[index].output_override = Some(output);
            scope.validate()?;
        }
        Ok(scope.packages.remove(index))
    }

    pub(crate) fn inputs(&self) -> BTreeSet<PathBuf> {
        let mut inputs = BTreeSet::from([self.manifest.clone()]);
        if let Some(path) = &self.workspace {
            inputs.insert(path.clone());
            if let Ok(manifest) = read(path) {
                if let Some(workspace) = manifest.workspace {
                    for member in workspace.members {
                        let path =
                            normalize(&path.parent().unwrap().join(member).join("Notist.toml"));
                        inputs.insert(std::fs::canonicalize(&path).unwrap_or(path));
                    }
                }
            }
        }
        inputs
    }

    /// Re-read membership, output ownership and package configuration on every build.
    pub(crate) fn prepare(&self) -> Result<Prepared, String> {
        let mut scope = discover(&self.manifest)?;
        let fresh = scope
            .packages
            .iter_mut()
            .find(|p| p.manifest == self.manifest)
            .ok_or("package was removed from workspace; restart serve")?;
        if fresh.name != self.name {
            return Err("package name changed; restart serve".into());
        }
        if let Some(output) = &self.output_override {
            fresh.output = Some(output.clone());
        }
        scope.validate()?;
        let fresh = scope
            .packages
            .iter()
            .find(|p| p.manifest == self.manifest)
            .unwrap();
        let manifest = read(&self.manifest)?;
        let mut site = manifest
            .site
            .ok_or("package has no [site] to build or serve")?;
        // The host owns the package-aware default; the SSG library has no package identity.
        let output = fresh
            .output
            .as_ref()
            .ok_or("site configuration changed; retry")?;
        site.insert(
            "output".into(),
            output.to_string_lossy().into_owned().into(),
        );
        let mut options: notist_ssg::SiteConfig = toml::Value::Table(site)
            .try_into()
            .map_err(|e: toml::de::Error| format!("{}: {e}", self.manifest.display()))?;
        options.validate().map_err(|e| e.to_string())?;
        let content = normalize(&self.root.join(&options.content));
        if let Ok(relative) = scope.output_root.strip_prefix(&content) {
            if !relative.as_os_str().is_empty() {
                options
                    .exclude
                    .push(relative.to_string_lossy().replace('\\', "/"));
                options.exclude.push(format!(
                    "{}/**",
                    relative.to_string_lossy().replace('\\', "/")
                ));
            }
        }
        let mut inputs = fresh.inputs();
        inputs.extend(scope.packages.iter().map(|p| p.manifest.clone()));
        Ok(Prepared {
            files: Files::new(&self.root).with_inputs(inputs),
            config: self.manifest.clone(),
            options,
        })
    }
}

pub(crate) fn build(
    config: Option<&Path>,
    output: Option<&Path>,
    all: bool,
    names: &[String],
    jobs: Option<std::num::NonZeroUsize>,
) -> ExitCode {
    let result = (|| {
        let current = locate(config)?;
        let mut scope = discover(&current)?;
        for name in names {
            let package = scope
                .packages
                .iter()
                .find(|p| p.name == *name)
                .ok_or_else(|| format!("unknown package: {name}"))?;
            if package.output.is_none() {
                return Err(format!("package `{name}` has no [site] to build"));
            }
        }
        let root_is_virtual = read(&current)?.package.is_none();
        let indices: Vec<_> = scope
            .packages
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                (p.output.is_some()
                    && if names.is_empty() {
                        all || root_is_virtual || p.manifest == current
                    } else {
                        names.contains(&p.name)
                    })
                .then_some(i)
            })
            .collect();
        if indices.is_empty() {
            return Err(
                match scope.packages.iter().find(|p| p.manifest == current) {
                    Some(package) => {
                        format!("package `{}` has no [site] to build or serve", package.name)
                    }
                    None => "workspace has no package with [site] to build".to_owned(),
                },
            );
        }
        if let Some(path) = output {
            if indices.len() != 1 {
                return Err("--out-dir requires exactly one package site".into());
            }
            let output = override_output(path)?;
            let package = &mut scope.packages[indices[0]];
            package.output = Some(output.clone());
            package.output_override = Some(output);
            scope.validate()?;
        }
        for &index in &indices {
            scope.packages[index].prepare()?;
        }
        let executor = crate::ssg::executor(jobs)?;
        for index in indices {
            let result = crate::ssg::build_package(&scope.packages[index], &executor);
            if result != ExitCode::SUCCESS {
                return Ok(result);
            }
        }
        Ok(ExitCode::SUCCESS)
    })();
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
