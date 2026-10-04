use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn analyze(src: &str) -> String {
    crate::cst_json::analyze_json(src)
}

#[wasm_bindgen]
pub fn configuration(config: &str) -> String {
    crate::preview::configuration_json(config)
}
#[wasm_bindgen]
pub fn describe_project(config: &str, packages: &str) -> String {
    crate::preview::project_description(config, packages)
}
#[wasm_bindgen]
pub fn analyze_project(path: &str, src: &str, config: &str, packages: &str) -> String {
    crate::preview::analyze_preview(path, src, config, packages)
}
#[wasm_bindgen]
pub fn analyze_module(package: &str, src: &str) -> String {
    crate::cst_json::analyze_module_json(package, src)
}
