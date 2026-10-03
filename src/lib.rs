use rowan::TextRange;

pub use notist_core::{builtins, diag, dump, eval, expr, index, item, query, reflow, sectionize, shape};

pub mod cst_json;
pub mod desugar;
pub mod frontend;
#[cfg(not(target_arch = "wasm32"))]
pub mod lsp;
pub mod vault;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// The full pipeline: parse → desugar → shape → eval, collecting the
/// diagnostics of every phase (syntax from the parser, semantic from
/// desugar, type from eval).
pub fn analyze(src: &str) -> (item::Item, Vec<diag::Diagnostic>) {
    let (forest, module_attrs, mut diagnostics) = desugar::lower_not(src);
    let forest = shape::shape(forest);
    let span = TextRange::new(0.into(), (src.len() as u32).into());
    let item = eval::eval_doc(&forest, span, module_attrs, &mut diagnostics);
    (item, diagnostics)
}

pub fn dump_str(src: &str) -> String {
    let (item, diagnostics) = analyze(src);
    let mut out = String::new();
    for d in &diagnostics {
        out.push_str(&format!(
            "error[{}] @{}..{}: {}\n",
            d.phase,
            u32::from(d.span.start()),
            u32::from(d.span.end()),
            d.message
        ));
    }
    out.push_str(&dump::dump(&item));
    out
}
