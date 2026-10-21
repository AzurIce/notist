use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn source_imports(path: &str, source: &str) -> String {
    crate::preview::source_imports(path, source)
}
#[wasm_bindgen]
pub fn describe_prepared(path: &str, inputs: &str) -> String {
    crate::preview::describe_prepared(path, inputs)
}
#[wasm_bindgen]
pub fn analyze_prepared(path: &str, src: &str, inputs: &str) -> String {
    crate::preview::analyze_prepared(path, src, inputs)
}
#[wasm_bindgen]
pub fn render_prepared(path: &str, src: &str, inputs: &str) -> String {
    crate::preview::render_prepared(path, src, inputs)
}
