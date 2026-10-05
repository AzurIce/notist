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
pub fn describe_prepared(inputs: &str) -> String {
    crate::preview::describe_prepared(inputs)
}
#[wasm_bindgen]
pub fn analyze_prepared(path: &str, src: &str, inputs: &str) -> String {
    crate::preview::analyze_prepared(path, src, inputs)
}
#[wasm_bindgen]
pub fn analyze_module(package: &str, src: &str) -> String {
    crate::cst_json::analyze_module_json(package, src)
}

#[wasm_bindgen]
pub fn render_prepared(path: &str, src: &str, inputs: &str) -> String {
    crate::preview::render_prepared(path, src, inputs)
}
