use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn analyze(src: &str) -> String {
    crate::cst_json::analyze_json(src)
}
