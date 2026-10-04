//! Pure browser host APIs: sources and module URLs are supplied explicitly.
use crate::project::{Package, Project};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewPackage {
    pub source: String,
    #[serde(default)]
    pub components: BTreeMap<String, String>,
}

pub fn configuration_json(config: &str) -> String {
    match crate::project::parse_config(config) {
        Ok(dependencies) => json!({"dependencies": dependencies.iter().map(|dependency| json!({"name":dependency.name,"path":dependency.path})).collect::<Vec<_>>()}).to_string(),
        Err(errors) => json!({"diagnostics": errors.iter().map(|error| diagnostic_json("Notist.toml", error)).collect::<Vec<_>>()}).to_string(),
    }
}

fn prepare(
    config: &str,
    packages_json: &str,
) -> Result<(Project, BTreeMap<String, PreviewPackage>), Value> {
    let dependencies = crate::project::parse_config(config).map_err(|errors| json!({"diagnostics":errors.iter().map(|error| diagnostic_json("Notist.toml", error)).collect::<Vec<_>>()}))?;
    let packages: BTreeMap<String, PreviewPackage> =
        serde_json::from_str(packages_json).map_err(|error| json!({"error": error.to_string()}))?;
    let mut sources = Vec::new();
    for dependency in dependencies {
        let source = packages.get(&dependency.name).ok_or_else(
            || json!({"error":format!("missing declaration source for `{}`", dependency.name)}),
        )?;
        sources.push(Package {
            name: dependency.name,
            root: dependency.path,
            source: source.source.clone(),
        });
    }
    let project = Project::from_packages(sources).map_err(|errors| json!({"diagnostics":errors.iter().map(|error| diagnostic_json(&error.path.to_string_lossy(), &error.diagnostic)).collect::<Vec<_>>()}))?;
    Ok((project, packages))
}

pub fn project_description(config: &str, packages_json: &str) -> String {
    match prepare(config, packages_json) {
        Ok((project, _)) => json!({"functions": project.registry().functions().filter(|definition| definition.id.package != "notist").map(|definition| json!({"package":definition.id.package,"name":definition.id.name})).collect::<Vec<_>>(), "diagnostics":[]}).to_string(),
        Err(error) => error.to_string(),
    }
}

pub fn analyze_preview(path: &str, src: &str, config: &str, packages_json: &str) -> String {
    if path.ends_with(".notc") {
        return crate::cst_json::analyze_module_json("package", src);
    }
    let (project, packages) = match prepare(config, packages_json) {
        Ok(environment) => environment,
        Err(error) => return error.to_string(),
    };
    let mut registry = notist_html::HtmlRegistry::default();
    for definition in project
        .registry()
        .functions()
        .filter(|definition| definition.id.package != "notist")
    {
        if let Some(module) = packages[&definition.id.package]
            .components
            .get(&definition.id.name)
            && let Err(error) = registry.bind_component(definition, module)
        {
            return json!({"error":error}).to_string();
        }
    }
    let mut data: Value = serde_json::from_str(&crate::cst_json::analyze_document_json(
        std::path::Path::new(path),
        src,
        project.registry(),
    ))
    .unwrap();
    let Ok(analysis) = project.analyzer().analyze(path, src) else {
        return data.to_string();
    };
    let result = notist_html::Renderer::new()
        .with_registry(registry)
        .with_source_map()
        .render_with_diagnostics(analysis.root());
    data["html"] = json!(result.html);
    data["used_components"] = json!(result.used_components.iter().map(|component| json!({"package":component.id.package,"name":component.id.name,"tag":component.tag,"module":component.module})).collect::<Vec<_>>());
    data["diagnostics"].as_array_mut().unwrap().extend(
        result
            .diagnostics
            .iter()
            .map(|error| diagnostic_json(path, error)),
    );
    data.to_string()
}
fn diagnostic_json(path: &str, error: &crate::Diagnostic) -> Value {
    json!({"path":path,"start":u32::from(error.span.start()),"end":u32::from(error.span.end()),"phase":error.phase.to_string(),"message":error.message})
}
