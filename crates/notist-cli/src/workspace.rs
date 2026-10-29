//! Explicit projects and publication targets. Workspace membership does not grant
//! publication access or merge document configuration across project boundaries.
use crate::{publication::Files, serve::Kind};
use notist::resources::normalize;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    workspace: Option<Workspace>,
    project: Option<Project>,
    #[serde(default)]
    book: Vec<toml::Table>,
    #[serde(default)]
    site: Vec<toml::Table>,
    #[serde(default)]
    extensions: Vec<toml::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Workspace {
    members: Vec<PathBuf>,
    #[serde(default = "default_dist")]
    dist: PathBuf,
}
fn default_dist() -> PathBuf {
    "dist".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Project {
    name: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Target {
    manifest: PathBuf,
    workspace: Option<PathBuf>,
    pub project: String,
    pub name: String,
    pub root: PathBuf,
    pub output: PathBuf,
    pub kind: Kind,
    output_override: Option<PathBuf>,
}

pub(crate) struct Prepared {
    pub files: Files,
    pub config: PathBuf,
    pub options: Options,
}

pub(crate) enum Options {
    Book(notist_book::BookConfig),
    Site(notist_ssg::SiteConfig),
}

fn read(path: &Path) -> Result<Manifest, String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: toml::Value =
        toml::from_str(&source).map_err(|e| format!("{}: {e}", path.display()))?;
    if value.get("ssg").is_some() || value.get("book").is_some_and(toml::Value::is_table) {
        return Err(format!(
            "{}: legacy [book]/[ssg] configuration is no longer supported; add [project] and migrate to [[book]]/[[site]], then use notist build/serve",
            path.display()
        ));
    }
    toml::from_str(&source).map_err(|e| format!("{}: {e}", path.display()))
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn name(name: &str) -> Result<(), String> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(format!(
            "invalid project or target name {name:?}: use ASCII letters, digits, '-' or '_'"
        ))
    }
}
fn string(table: &mut toml::Table, key: &str, default: &str) -> Result<String, String> {
    match table.remove(key) {
        None => Ok(default.into()),
        Some(toml::Value::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(format!("target {key} must be a nonempty string")),
    }
}

fn targets(path: &Path, workspace: Option<&Path>, dist: &Path) -> Result<Vec<Target>, String> {
    let manifest = read(path)?;
    let project = manifest
        .project
        .ok_or_else(|| format!("{}: missing [project]", path.display()))?
        .name;
    name(&project)?;
    let mut names = BTreeSet::new();
    let mut result = Vec::new();
    for (kind, tables) in [(Kind::Book, manifest.book), (Kind::Site, manifest.site)] {
        for mut table in tables {
            let target_name = string(&mut table, "name", &project)?;
            name(&target_name)?;
            if !names.insert(target_name.clone()) {
                return Err(format!(
                    "duplicate target {target_name:?} in project {project}"
                ));
            }
            let root = path
                .parent()
                .unwrap()
                .join(string(&mut table, "root", ".")?);
            let root =
                std::fs::canonicalize(&root).map_err(|e| format!("{}: {e}", root.display()))?;
            if !root.is_dir() {
                return Err(format!(
                    "target root is not a directory: {}",
                    root.display()
                ));
            }
            let output = match table.get("output") {
                None => dist.join(&project).join(&target_name),
                Some(toml::Value::String(p)) if !p.is_empty() => normalize(&root.join(p)),
                _ => return Err("target output must be a nonempty string".into()),
            };
            result.push(Target {
                manifest: path.into(),
                workspace: workspace.map(Into::into),
                project: project.clone(),
                name: target_name,
                root,
                output,
                kind,
                output_override: None,
            });
        }
    }
    if result.is_empty() {
        return Err(format!(
            "{}: project has no [[book]] or [[site]] targets",
            path.display()
        ));
    }
    Ok(result)
}

fn nearest(start: &Path) -> Result<PathBuf, String> {
    start
        .ancestors()
        .map(|p| p.join("Notist.toml"))
        .find(|p| p.is_file())
        .ok_or_else(|| "no Notist.toml found in this directory or its ancestors".into())
}

/// Load every member before selection so names and output ownership are checked
/// independently of which target happens to be built today.
fn discover(config: Option<&Path>) -> Result<(Vec<Target>, Option<String>), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let current = match config {
        Some(p) => std::fs::canonicalize(p).map_err(|e| e.to_string())?,
        None => nearest(&cwd)?,
    };
    let local = read(&current)?;
    let current_project = local.project.as_ref().map(|p| p.name.clone());
    let mut workspace = local.workspace.as_ref().map(|_| current.clone());
    if workspace.is_none() {
        for parent in current.parent().unwrap().ancestors().skip(1) {
            let candidate = parent.join("Notist.toml");
            if candidate.is_file() {
                // Ancestor document configuration is not inherited.
                let value: toml::Value = std::fs::read_to_string(&candidate)
                    .map_err(|e| e.to_string())?
                    .parse()
                    .map_err(|e: toml::de::Error| e.to_string())?;
                if value.get("workspace").is_some() {
                    workspace = Some(candidate);
                    break;
                }
            }
        }
    }
    let mut result = Vec::new();
    if let Some(ws_path) = &workspace {
        let ws_manifest = read(ws_path)?;
        let ws = ws_manifest
            .workspace
            .ok_or("workspace configuration changed while reading; retry")?;
        if ws.dist.as_os_str().is_empty() {
            return Err("workspace dist must not be empty".into());
        }
        let base = ws_path.parent().unwrap();
        let dist = normalize(&base.join(ws.dist));
        let mut members = BTreeSet::new();
        for member in ws.members {
            let member = std::fs::canonicalize(base.join(member).join("Notist.toml"))
                .map_err(|e| format!("invalid workspace member: {e}"))?;
            if !members.insert(member.clone()) {
                return Err(format!("duplicate workspace member: {}", member.display()));
            }
            if member != *ws_path && read(&member)?.workspace.is_some() {
                return Err("nested workspaces are not supported".into());
            }
        }
        if ws_manifest.project.is_some() {
            members.insert(ws_path.clone());
        }
        if !members.contains(&current) && current != *ws_path {
            return Err(format!(
                "{} is not a member of {}",
                current.display(),
                ws_path.display()
            ));
        }
        for member in members {
            result.extend(targets(&member, Some(ws_path), &dist)?);
        }
    } else {
        result = targets(&current, None, &current.parent().unwrap().join("dist"))?;
    }
    if result.is_empty() {
        return Err("workspace has no projects".into());
    }
    let mut projects = std::collections::BTreeMap::new();
    for target in &result {
        if projects
            .insert(&target.project, &target.manifest)
            .is_some_and(|previous| previous != &target.manifest)
        {
            return Err(format!("duplicate project name: {}", target.project));
        }
    }
    validate_outputs(&result)?;
    Ok((result, current_project))
}

fn validate_outputs(result: &[Target]) -> Result<(), String> {
    for target in result {
        for other in result {
            if other.root.starts_with(&target.output) || other.manifest.starts_with(&target.output)
            {
                return Err(format!(
                    "output would replace project inputs: {}",
                    target.output.display()
                ));
            }
        }
    }
    for (i, target) in result.iter().enumerate() {
        for other in &result[..i] {
            if target.output.starts_with(&other.output) || other.output.starts_with(&target.output)
            {
                return Err(format!(
                    "overlapping target outputs: {} and {}",
                    target.output.display(),
                    other.output.display()
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn select(
    config: Option<&Path>,
    all: bool,
    projects: &[String],
    target: Option<&str>,
) -> Result<Vec<Target>, String> {
    let (mut targets, current) = discover(config)?;
    for project in projects {
        if !targets.iter().any(|t| &t.project == project) {
            return Err(format!("unknown project: {project}"));
        }
    }
    targets.retain(|t| {
        (all || if projects.is_empty() {
            current.as_ref().is_none_or(|p| p == &t.project)
        } else {
            projects.contains(&t.project)
        }) && target.is_none_or(|name| name == t.name)
    });
    if targets.is_empty() {
        return Err("no matching targets".into());
    }
    Ok(targets)
}

impl Target {
    pub(crate) fn inputs(&self) -> BTreeSet<PathBuf> {
        let mut inputs = BTreeSet::from([self.manifest.clone()]);
        if let Some(ws) = &self.workspace {
            inputs.insert(ws.clone());
            if let Ok(manifest) = read(ws)
                && let Some(workspace) = manifest.workspace
            {
                for member in workspace.members {
                    let path = normalize(&ws.parent().unwrap().join(member).join("Notist.toml"));
                    inputs.insert(std::fs::canonicalize(&path).unwrap_or(path));
                }
            }
        }
        inputs
    }
    /// Re-read manifests on every preview rebuild. Root/type changes require a
    /// restart, but content options, extensions and workspace dist reload live.
    pub(crate) fn prepare(&self) -> Result<Prepared, String> {
        let (mut all, _) = discover(Some(self.workspace.as_deref().unwrap_or(&self.manifest)))?;
        if let Some(output) = &self.output_override {
            for target in &mut all {
                if target.project == self.project && target.name == self.name {
                    target.output = output.clone();
                }
            }
            validate_outputs(&all)?;
        }
        let fresh = all
            .into_iter()
            .find(|t| t.project == self.project && t.name == self.name)
            .ok_or_else(|| "selected target was removed; restart serve".to_string())?;
        if fresh.root != self.root || fresh.kind != self.kind {
            return Err("target root or kind changed; restart serve".into());
        }
        let manifest = read(&self.manifest)?;
        let tables = match self.kind {
            Kind::Book => manifest.book,
            Kind::Site => manifest.site,
        };
        let mut options = tables
            .into_iter()
            .find(|t| {
                t.get("name")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(&self.project)
                    == self.name
            })
            .ok_or("target configuration changed while reading; retry")?;
        options.remove("name");
        options.remove("root");
        options.insert(
            "output".into(),
            fresh.output.to_string_lossy().into_owned().into(),
        );
        let mut extensions = manifest.extensions;
        if self.kind == Kind::Book
            && let Some(value) = options.remove("extensions")
        {
            extensions.extend(
                value
                    .as_array()
                    .ok_or("book extensions must be an array")?
                    .iter()
                    .cloned(),
            );
        }
        let options = match self.kind {
            Kind::Book => Options::Book(
                toml::Value::Table(options)
                    .try_into()
                    .map_err(|e: toml::de::Error| e.to_string())?,
            ),
            Kind::Site => {
                let options: notist_ssg::SiteConfig = toml::Value::Table(options)
                    .try_into()
                    .map_err(|e: toml::de::Error| e.to_string())?;
                options.validate().map_err(|e| e.to_string())?;
                Options::Site(options)
            }
        };
        let mut config = toml::Table::new();
        config.insert("extensions".into(), extensions.into());
        let source = toml::to_string(&config).map_err(|e| e.to_string())?;
        // A distinct virtual identity makes diagnostics show the exact selected
        // configuration instead of attributing generated spans to the manifest.
        // Document extension imports and command CWD remain relative to the
        // declaring project manifest, just like single-document commands.
        let path = self
            .manifest
            .parent()
            .unwrap()
            .join(format!(".notist-target-{}.toml", self.name));
        let files =
            Files::new(&self.root).with_configuration(path.clone(), source.clone(), self.inputs());
        Ok(Prepared {
            files,
            config: path,
            options,
        })
    }
}

pub(crate) fn build(
    config: Option<&Path>,
    all: bool,
    projects: &[String],
    target: Option<&str>,
    output: Option<&Path>,
) -> ExitCode {
    let mut targets = match select(config, all, projects, target) {
        Ok(targets) => targets,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(output) = output {
        if targets.len() != 1 || output.as_os_str().is_empty() {
            eprintln!("--out-dir requires exactly one target and a nonempty output path");
            return ExitCode::FAILURE;
        }
        let cwd = match std::env::current_dir() {
            Ok(cwd) => cwd,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        targets[0].output_override = Some(normalize(&cwd.join(output)));
    }
    // Preflight all selected configurations before replacing any output.
    for target in &targets {
        let check = target.prepare();
        if let Err(e) = check {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }
    for target in &targets {
        println!("Building {} / {}", target.project, target.name);
        let code = match target.kind {
            Kind::Book => crate::book::build_target(target),
            Kind::Site => crate::ssg::build_target(target),
        };
        if code != ExitCode::SUCCESS {
            return code;
        }
    }
    ExitCode::SUCCESS
}
