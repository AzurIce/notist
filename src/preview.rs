//! Browser adapter. Native hosts can use PreparedInputs and Vault directly.
use crate::{MemoryResources, PreparedInputs, RenderOptions, Vault, VaultError};
use serde_json::{Value, json};

pub fn configuration_json(source: &str) -> String {
    match crate::environment::parse_config(source) {
        Ok(config) => {
            let dependencies = |values: &[crate::environment::Dependency]| values.iter().map(|dependency| json!({"name":dependency.name,"path":dependency.path})).collect::<Vec<_>>();
            let transforms = |values: &[crate::transforms::Replace]| values.iter().map(|rule|json!({"kind":"replace","from":rule.from.to_string(),"to":rule.to.to_string()})).collect::<Vec<_>>();
            json!({"package":config.package.map(|package|json!({"name":package.name})), "dependencies":dependencies(&config.dependencies), "dev_dependencies":dependencies(&config.dev_dependencies), "transforms":transforms(&config.transforms), "dev_transforms":transforms(&config.dev_transforms)}).to_string()
        }
        Err(errors) => json!({"diagnostics": errors.iter().map(|error| diagnostic_json("Notist.toml", "environment", error)).collect::<Vec<_>>()}).to_string(),
    }
}
fn prepare(inputs: &str) -> Result<Vault<MemoryResources>, Value> {
    serde_json::from_str::<PreparedInputs>(inputs)
        .map(PreparedInputs::into_vault)
        .map_err(|error| json!({"error":error.to_string()}))
}

/// Describe registered functions and resource roots after manifest assembly.
/// Component candidates need not have been fetched yet.
pub fn describe_prepared(inputs: &str) -> String {
    let mut vault = match prepare(inputs) {
        Ok(vault) => vault,
        Err(error) => return error.to_string(),
    };
    match vault.environment_for("document.not") {
        Ok(environment) => json!({"functions": environment.registry().functions().filter(|definition| definition.id.package != "notist").map(|definition| json!({"package":definition.id.package,"root":environment.packages()[&definition.id.package].root,"name":definition.id.name,"entries":notist_html::components::component_entries(&definition.id.name)})).collect::<Vec<_>>(), "diagnostics":[]}).to_string(),
        Err(error) => error_json(error).to_string(),
    }
}

/// Worker transport adapter for a host-prepared logical resource view.
pub fn render_prepared(path: &str, src: &str, inputs_json: &str) -> String {
    let mut vault = match prepare(inputs_json) {
        Ok(vault) => vault,
        Err(error) => return error.to_string(),
    };
    match vault.render_html(path, src, RenderOptions::default()) {
        Ok(output) => {
            let mut data = json!({"diagnostics":output.analysis.diagnostics().iter().map(|error|diagnostic_json(path,"analysis",error)).collect::<Vec<_>>()});
            append_transforms(&mut data, path, &output.transformed);
            append_render(&mut data, path, output.rendered);
            data.to_string()
        }
        Err(error) => error_json(error).to_string(),
    }
}

pub fn analyze_prepared(path: &str, src: &str, inputs: &str) -> String {
    if path.ends_with(".notc") {
        let mut data: Value =
            serde_json::from_str(&crate::cst_json::analyze_module_json("package", src)).unwrap();
        annotate_analysis(&mut data, path);
        return data.to_string();
    }
    let mut vault = match prepare(inputs) {
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
