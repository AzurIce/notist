use rowan::TextRange;

pub mod builtins;
pub mod cst_json;
pub mod desugar;
pub mod dump;
pub mod eval;
pub mod expr;
pub mod item;
#[cfg(not(target_arch = "wasm32"))]
pub mod lsp;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// The full pipeline: parse → desugar → eval, collecting the diagnostics of
/// every phase.
pub fn analyze(src: &str) -> (item::Item, Vec<notist_syntax::parser::Diagnostic>) {
    let parse = notist_syntax::parser::parse(src);
    let mut diagnostics = parse.diagnostics.clone();
    let Some(document) = notist_syntax::ast::Document::cast(parse.syntax()) else {
        return (
            item::Item::new(item::Ctor::Doc, TextRange::empty(0.into())),
            diagnostics,
        );
    };
    let expr = desugar::desugar(&document, &mut diagnostics);
    let item = eval::eval_doc(&expr.0, document.range(), expr.1, &mut diagnostics);
    (item, diagnostics)
}

pub fn dump_str(src: &str) -> String {
    let (item, diagnostics) = analyze(src);
    let mut out = String::new();
    for d in &diagnostics {
        out.push_str(&format!(
            "error @{}..{}: {}\n",
            u32::from(d.span.start()),
            u32::from(d.span.end()),
            d.message
        ));
    }
    out.push_str(&dump::dump(&item));
    out
}
