//! Browser resource assembly. The fragment renderer itself performs no IO.
use crate::{
    Item,
    project::{Project, SourceDiagnostic},
};
use notist_html::{Component, HtmlRegistry, Renderer};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Bind convention-based modules. Missing modules are diagnosed only if called;
/// conflicting entries and protocol collisions invalidate the target environment.
pub fn html_registry(project: &Project) -> Result<HtmlRegistry, Vec<SourceDiagnostic>> {
    let mut registry = HtmlRegistry::default();
    let mut errors = Vec::new();
    for definition in project
        .registry()
        .functions()
        .filter(|definition| definition.id.package != "notist")
    {
        let package = &project.packages()[&definition.id.package];
        let file = package
            .root
            .join("components")
            .join(format!("{}.js", definition.id.name));
        let directory = package
            .root
            .join("components")
            .join(&definition.id.name)
            .join("index.js");
        let error = if file.is_file() && directory.is_file() {
            Some(format!(
                "conflicting component entries: `{}` and `{}`",
                file.display(),
                directory.display()
            ))
        } else if file.is_file() || directory.is_file() {
            let entry = if file.is_file() { file } else { directory };
            registry
                .bind_component(definition, entry.to_string_lossy())
                .err()
        } else {
            None
        };
        if let Some(message) = error {
            errors.push(SourceDiagnostic {
                path: package.root.join("lib.notc"),
                source: package.source.clone(),
                diagnostic: crate::Diagnostic::new(
                    crate::Phase::Semantic,
                    definition.span,
                    message,
                ),
            });
        }
    }
    if errors.is_empty() {
        Ok(registry)
    } else {
        Err(errors)
    }
}

/// Generate registrations for the exact modules recorded by the renderer.
/// Calling this repeatedly is safe; conflicting constructors raise an error.
pub fn registration_script(
    components: &[Component],
    urls: &BTreeMap<crate::FunctionId, String>,
) -> Result<String, String> {
    let mut seen = BTreeMap::new();
    let mut output = String::new();
    let mut index = 0;
    for component in components {
        let url = urls
            .get(&component.id)
            .ok_or_else(|| format!("no published URL for `{}`", component.id))?;
        if let Some((id, previous_url)) = seen.get(&component.tag) {
            if id != &component.id || previous_url != url {
                return Err(format!(
                    "conflicting component registration `{}`",
                    component.tag
                ));
            }
            continue;
        }
        seen.insert(component.tag.clone(), (component.id.clone(), url.clone()));
        let tag = serde_json::to_string(&component.tag).unwrap();
        let url = serde_json::to_string(url).unwrap();
        output.push_str(&format!("import Component{index} from {url};\n{{\n  const tag = {tag};\n  const existing = customElements.get(tag);\n  if (existing && existing !== Component{index}) throw new Error(`Conflicting Notist component: ${{tag}}`);\n  if (!existing) customElements.define(tag, Component{index});\n}}\n"));
        index += 1;
    }
    Ok(output)
}

#[derive(Debug)]
pub struct BuildResult {
    pub page: PathBuf,
    pub used_components: Vec<Component>,
}

/// Build a complete page and copy only its used component directories.
pub fn build_page(item: &Item, project: &Project, output: &Path) -> Result<BuildResult, String> {
    let registry = html_registry(project).map_err(|errors| {
        errors
            .iter()
            .map(|error| format!("{}: {}", error.path.display(), error.diagnostic.message))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    let rendered = Renderer::new()
        .with_registry(registry)
        .render_with_diagnostics(item);
    if !rendered.diagnostics.is_empty() {
        return Err(rendered
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n"));
    }
    // Assemble and validate all locators before writing any output.
    let mut urls = BTreeMap::new();
    let mut copies = Vec::new();
    let mut destinations = BTreeSet::new();
    for component in &rendered.used_components {
        let package = &project.packages()[&component.id.package];
        let source = Path::new(&component.module);
        let relative = source
            .strip_prefix(&package.root)
            .map_err(|error| error.to_string())?;
        let destination = PathBuf::from("packages")
            .join(notist_html::components::encode_name(&component.id.package))
            .join(relative);
        if !destinations.insert(destination.clone()) {
            return Err("component resource destination collision".into());
        }
        urls.insert(
            component.id.clone(),
            format!("./{}", url_path(&destination)),
        );
        if source.file_name().is_some_and(|name| name == "index.js") {
            copies.push((
                source.parent().unwrap().to_path_buf(),
                output.join(destination.parent().unwrap()),
            ));
        } else {
            copies.push((source.to_path_buf(), output.join(&destination)));
        }
    }
    let registrations = registration_script(&rendered.used_components, &urls)?;
    // Reject source/destination overlap before recursive copying starts.
    for (source, destination) in &copies {
        let source = std::fs::canonicalize(source).map_err(|error| error.to_string())?;
        let destination = crate::vault::normalize(
            &std::env::current_dir()
                .map_err(|error| error.to_string())?
                .join(destination),
        );
        if destination.starts_with(&source) || source.starts_with(&destination) {
            return Err("component resource source and destination overlap".into());
        }
    }
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
    for (source, target) in copies {
        copy_resource(&source, &target)?;
    }
    std::fs::write(output.join("components.js"), registrations)
        .map_err(|error| error.to_string())?;
    let html = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Notist</title></head><body>\n{}\n<script type=\"module\" src=\"./components.js\"></script>\n</body></html>\n",
        rendered.html
    );
    let page = output.join("index.html");
    std::fs::write(&page, html).map_err(|error| error.to_string())?;
    Ok(BuildResult {
        page,
        used_components: rendered.used_components,
    })
}
fn copy_resource(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "component resources must be regular files or directories: {}",
            source.display()
        ));
    }
    if metadata.is_dir() {
        std::fs::create_dir_all(destination).map_err(|error| error.to_string())?;
        for entry in std::fs::read_dir(source).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            copy_resource(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        if !metadata.is_file() {
            return Err(format!(
                "component resource is not a regular file: {}",
                source.display()
            ));
        }
        std::fs::create_dir_all(destination.parent().unwrap())
            .map_err(|error| error.to_string())?;
        std::fs::copy(source, destination).map_err(|error| error.to_string())?;
    }
    Ok(())
}
fn url_path(path: &Path) -> String {
    path.components()
        .map(|part| {
            part.as_os_str()
                .to_string_lossy()
                .bytes()
                .map(|byte| {
                    if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                        (byte as char).to_string()
                    } else {
                        format!("%{byte:02X}")
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}
