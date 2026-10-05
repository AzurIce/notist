//! Browser adapter. Native hosts can use PreparedInputs and Vault directly.
use crate::{MemoryResources, PreparedInputs, RenderOptions, Vault, VaultError};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewPackage {
    pub source: String,
    #[serde(default)]
    pub components: BTreeMap<String, String>,
    /// The selected relative entry from project_description's candidates.
    #[serde(default)]
    pub entries: BTreeMap<String, PathBuf>,
}

pub fn configuration_json(config: &str) -> String {
    match crate::environment::parse_config(config) {
        Ok(config) => json!({"dependencies": config.dependencies.iter().map(|dependency| json!({"name":dependency.name,"path":dependency.path})).collect::<Vec<_>>(), "transforms":config.transforms.iter().map(|rule|json!({"kind":"replace","from":rule.from.to_string(),"to":rule.to.to_string()})).collect::<Vec<_>>()}).to_string(),
        Err(errors) => json!({"diagnostics": errors.iter().map(|error| diagnostic_json("Notist.toml", "environment", error)).collect::<Vec<_>>()}).to_string(),
    }
}

/// Prepare browser-fetched declaration sources and published module URLs using
/// the same configuration and entry rules as filesystem hosts.
pub fn prepare(
    config: &str,
    packages: &BTreeMap<String, PreviewPackage>,
) -> Result<Vault<MemoryResources>, Value> {
    let configuration = crate::environment::parse_config(config).map_err(|errors| json!({"diagnostics":errors.iter().map(|error| diagnostic_json("Notist.toml", "environment", error)).collect::<Vec<_>>()}))?;
    let mut inputs = PreparedInputs {
        root: "/preview".into(),
        config: Some("Notist.toml".into()),
        files: BTreeMap::new(),
        module_urls: BTreeMap::new(),
    };
    inputs
        .files
        .insert("Notist.toml".into(), config.as_bytes().to_vec());
    for dependency in configuration.dependencies {
        let package = packages.get(&dependency.name).ok_or_else(
            || json!({"error":format!("missing declaration source for `{}`", dependency.name)}),
        )?;
        inputs.files.insert(
            dependency.path.join("lib.notc"),
            package.source.as_bytes().to_vec(),
        );
        for (name, url) in &package.components {
            let candidates = notist_html::components::component_entries(name);
            let entry = package.entries.get(name).unwrap_or(&candidates[0]);
            if !candidates.contains(entry) {
                return Err(
                    json!({"error":format!("invalid component entry for `{}::{name}`", dependency.name)}),
                );
            }
            let path = dependency.path.join(entry);
            inputs.files.insert(path.clone(), Vec::new());
            inputs.module_urls.insert(path, url.clone());
        }
    }
    let mut vault = inputs.into_vault();
    vault.environment_for("document.not").map_err(error_json)?;
    Ok(vault)
}
fn parse_packages(packages: &str) -> Result<BTreeMap<String, PreviewPackage>, Value> {
    serde_json::from_str(packages).map_err(|error| json!({"error":error.to_string()}))
}

pub fn project_description(config: &str, packages_json: &str) -> String {
    let result = parse_packages(packages_json).and_then(|packages| prepare(config, &packages));
    match result {
        Ok(mut vault) => json!({"functions": vault.environment_for("document.not").unwrap().registry().functions().filter(|definition| definition.id.package != "notist").map(|definition| json!({"package":definition.id.package,"name":definition.id.name,"entries":notist_html::components::component_entries(&definition.id.name)})).collect::<Vec<_>>(), "diagnostics":[]}).to_string(),
        Err(error) => error.to_string(),
    }
}

/// Ordinary rendering adapter: no CST, AST or intermediate IR collection.
pub fn render_preview(path: &str, src: &str, config: &str, packages_json: &str) -> String {
    let result = parse_packages(packages_json).and_then(|packages| prepare(config, &packages));
    match result {
        Ok(mut vault) => match vault.render_html(path, src, RenderOptions::default()) {
            Ok(output) => {
                let mut data = json!({"diagnostics":output.analysis.diagnostics().iter().map(|error|diagnostic_json(path,"analysis",error)).collect::<Vec<_>>()});
                append_transforms(&mut data, path, &output.transformed);
                append_render(&mut data, path, output.rendered);
                data.to_string()
            }
            Err(error) => error_json(error).to_string(),
        },
        Err(error) => error.to_string(),
    }
}

/// Worker transport adapter for a host-prepared logical resource view.
pub fn render_prepared(path: &str, src: &str, inputs_json: &str) -> String {
    let inputs: PreparedInputs = match serde_json::from_str(inputs_json) {
        Ok(inputs) => inputs,
        Err(error) => return json!({"error":error.to_string()}).to_string(),
    };
    match inputs
        .into_vault()
        .render_html(path, src, RenderOptions::default())
    {
        Ok(output) => {
            let mut data = json!({"diagnostics":output.analysis.diagnostics().iter().map(|error|diagnostic_json(path,"analysis",error)).collect::<Vec<_>>()});
            append_transforms(&mut data, path, &output.transformed);
            append_render(&mut data, path, output.rendered);
            data.to_string()
        }
        Err(error) => error_json(error).to_string(),
    }
}

pub fn analyze_preview(path: &str, src: &str, config: &str, packages_json: &str) -> String {
    if path.ends_with(".notc") {
        let mut data: Value =
            serde_json::from_str(&crate::cst_json::analyze_module_json("package", src)).unwrap();
        annotate_analysis(&mut data, path);
        return data.to_string();
    }
    let result = parse_packages(packages_json).and_then(|packages| prepare(config, &packages));
    let mut vault = match result {
        Ok(vault) => vault,
        Err(error) => return error.to_string(),
    };
    let (analysis, inspection) = match vault.inspect(path, src) {
        Ok(result) => result,
        Err(error) => return error_json(error).to_string(),
    };
    let mut data: Value = serde_json::from_str(&crate::cst_json::inspection_json(
        &analysis,
        Some(&inspection),
    ))
    .unwrap();
    annotate_analysis(&mut data, path);
    match vault.render_output(path, analysis.root(), RenderOptions::default()) {
        Ok(output) => {
            append_transforms(&mut data, path, &output.transformed);
            // Debug consumers can compare the original and output identities.
            data["transformed"] = crate::json::item(&output.transformed.root);
            append_render(&mut data, path, output.rendered);
        }
        Err(error) => return error_json(error).to_string(),
    }
    data.to_string()
}
fn annotate_analysis(data: &mut Value, path: &str) {
    for diagnostic in data["diagnostics"].as_array_mut().unwrap() {
        diagnostic["path"] = json!(path);
        diagnostic["origin"] = json!("analysis");
    }
}
fn append_transforms(data: &mut Value, path: &str, output: &crate::transforms::TransformOutput) {
    data["diagnostics"].as_array_mut().unwrap().extend(
        output
            .diagnostics
            .iter()
            .map(|error| diagnostic_json(path, "transform", error)),
    );
}
fn append_render(data: &mut Value, path: &str, result: notist_html::RenderResult) {
    data["html"] = json!(result.html);
    data["source_map"] = json!(result.source_map.iter().map(|mapping|json!({"node_id":mapping.node_id,"start":mapping.range.start,"end":mapping.range.end,"kind":format!("{:?}",mapping.kind)})).collect::<Vec<_>>());
    data["used_components"] = json!(result.used_components.iter().map(|component| json!({"package":component.id.package,"name":component.id.name,"tag":component.tag,"module":component.module.url(),"resource":component.module.resource()})).collect::<Vec<_>>());
    data["diagnostics"].as_array_mut().unwrap().extend(
        result
            .diagnostics
            .iter()
            .map(|error| diagnostic_json(path, "render", error)),
    );
}
pub(crate) fn error_json(error: VaultError) -> Value {
    match error {
        VaultError::Environment(errors) => {
            json!({"diagnostics":errors.iter().map(|error|diagnostic_json(&error.path.to_string_lossy(),"environment",&error.diagnostic)).collect::<Vec<_>>()})
        }
        error => json!({"error":error.to_string()}),
    }
}
fn diagnostic_json(path: &str, origin: &str, error: &crate::Diagnostic) -> Value {
    json!({"path":path,"origin":origin,"start":u32::from(error.span.start()),"end":u32::from(error.span.end()),"phase":error.phase.to_string(),"message":error.message})
}
