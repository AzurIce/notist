//! Browser adapter. Native hosts can use PreparedInputs and Vault directly.
use crate::{MemoryResources, PreparedInputs, RenderOptions, Vault, VaultError};
use serde_json::{Value, json};

/// Discover source imports or extension-local configuration imports without IO.
pub fn source_imports(path: &str, source: &str) -> String {
    let (imports, diagnostics) = if path.ends_with(".toml") {
        match crate::environment::parse_config(source) {
            Ok(config) => (
                config
                    .extensions
                    .into_iter()
                    .flat_map(|rule| rule.imports().to_vec())
                    .collect(),
                Vec::new(),
            ),
            Err(diagnostics) => (Vec::new(), diagnostics),
        }
    } else if path.ends_with(".notc") {
        let parsed =
            crate::syntax::code::parse_module(source, &crate::syntax::markup::MarkupContentParser);
        let ast = crate::syntax::code::ast::Module::cast(parsed.syntax()).unwrap();
        let mut errors: Vec<_> = parsed
            .diagnostics
            .iter()
            .map(|d| crate::Diagnostic::new(crate::Phase::Syntax, d.span, &d.message))
            .collect();
        let imports = notist_pipeline::lowering::imports::lower_imports(ast.imports(), &mut errors);
        (imports, errors)
    } else {
        match crate::Pipeline::default().lower(path, source, crate::FrontendOptions::default()) {
            Ok(output) => (output.imports, output.diagnostics),
            Err(error) => return json!({"error":error.to_string()}).to_string(),
        }
    };
    json!({"imports":imports.iter().map(import_json).collect::<Vec<_>>(),"diagnostics":diagnostics.iter().map(|d|diagnostic_json(path,"analysis",d)).collect::<Vec<_>>()}).to_string()
}

pub(crate) fn import_json(import: &crate::Import) -> Value {
    let mut data = json!({"path": import.path, "start": u32::from(import.span.start()), "end": u32::from(import.span.end())});
    match &import.kind {
        crate::ImportKind::Module { alias } => {
            data["kind"] = json!("module");
            data["alias"] = json!(alias);
        }
        crate::ImportKind::Members(members) => {
            data["kind"] = json!("members");
            data["members"] = json!(members.iter().map(|m| json!({"name": m.name, "alias": m.alias, "start": u32::from(m.span.start()), "end": u32::from(m.span.end())})).collect::<Vec<_>>());
        }
        crate::ImportKind::Wildcard => data["kind"] = json!("wildcard"),
    }
    data
}
fn prepare(inputs: &str) -> Result<Vault<MemoryResources>, Value> {
    serde_json::from_str::<PreparedInputs>(inputs)
        .map(PreparedInputs::into_vault)
        .map_err(|error| json!({"error":error.to_string()}))
}

/// Describe HTML components actually required by this document's evaluated tree.
/// This stage uses only source inputs; the host subsequently probes these entries.
pub fn describe_prepared(path: &str, inputs: &str) -> String {
    if path.ends_with(".notc") {
        return json!({"functions": [], "diagnostics": []}).to_string();
    }
    let mut vault = match prepare(inputs) {
        Ok(vault) => vault,
        Err(error) => return error.to_string(),
    };
    let result = (|| -> Result<Value, VaultError> {
        let source = crate::Resources::source(vault.resources(), std::path::Path::new(path))?;
        let analysis = vault.analyze(path, &source)?;
        let evaluated = vault.evaluate(path, &analysis)?;
        let environment = vault.environment_for(path, &source)?;
        let functions: Vec<_> = notist_html::components::component_ids(&evaluated.root)
            .iter()
            .filter_map(|id| environment.context().registry().get(id))
            .filter(|definition| !definition.code)
            .map(|definition| {
                json!({
                    "module": definition.id.module,
                    "root": environment.modules()[&definition.id.module].path.parent().unwrap(),
                    "name": definition.id.name,
                    "entries": notist_html::components::component_entries(&definition.id.name)
                })
            })
            .collect();
        Ok(json!({"functions": functions, "diagnostics": []}))
    })();
    match result {
        Ok(description) => description.to_string(),
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
            append_evaluation(&mut data, path, &output.evaluation);
            append_render(&mut data, path, output.rendered);
            data.to_string()
        }
        Err(error) => error_json(error).to_string(),
    }
}

pub fn analyze_prepared(path: &str, src: &str, inputs: &str) -> String {
    let mut vault = match prepare(inputs) {
        Ok(vault) => vault,
        Err(error) => return error.to_string(),
    };
    if path.ends_with(".notc") {
        let analysis = match vault.analyze_code(path, src) {
            Ok(analysis) => analysis,
            Err(error) => return error_json(error).to_string(),
        };
        let mut data: Value =
            serde_json::from_str(&crate::cst_json::module_json(src, &analysis)).unwrap();
        annotate_analysis(&mut data, path);
        return data.to_string();
    }
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
    match vault.render_output(path, &analysis, RenderOptions::default()) {
        Ok(output) => {
            append_evaluation(&mut data, path, &output.evaluation);
            // Debug consumers can compare the original and output identities.
            data["evaluated"] = crate::json::item(&output.evaluation.root);
            append_render(&mut data, path, output.rendered);
        }
        Err(error) => return error_json(error).to_string(),
    }
    data.to_string()
}
fn annotate_analysis(data: &mut Value, path: &str) {
    for diagnostic in data["diagnostics"].as_array_mut().unwrap() {
        diagnostic["path"] = diagnostic["source"]
            .as_str()
            .map(|s| json!(s))
            .unwrap_or_else(|| json!(path));
        diagnostic["origin"] = json!("analysis");
    }
}
fn append_evaluation(data: &mut Value, path: &str, output: &notist_pipeline::Evaluation) {
    data["diagnostics"].as_array_mut().unwrap().extend(
        output
            .diagnostics
            .iter()
            .map(|error| diagnostic_json(path, "evaluate", error)),
    );
}
fn append_render(data: &mut Value, path: &str, result: notist_html::RenderResult) {
    data["html"] = json!(result.html);
    data["source_map"] = json!(result.source_map.iter().map(|mapping|json!({"node_id":mapping.node_id,"start":mapping.range.start,"end":mapping.range.end,"kind":format!("{:?}",mapping.kind),"source":mapping.origin.as_ref().map(|o|&o.source),"expansion":mapping.origin.as_ref().map(|o|o.expansion.iter().map(|e|json!({"source":e.source,"start":e.start,"end":e.end})).collect::<Vec<_>>())})).collect::<Vec<_>>());
    data["used_components"] = json!(result.used_components.iter().map(|component| json!({"identity":component.id.to_string(),"name":component.id.name,"tag":component.tag,"module":component.module.url(),"resource":component.module.resource()})).collect::<Vec<_>>());
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
    json!({"source":error.source,"path":error.source.as_deref().unwrap_or(path),"origin":origin,"start":u32::from(error.span.start()),"end":u32::from(error.span.end()),"phase":error.phase.to_string(),"message":error.message})
}
