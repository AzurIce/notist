pub mod code;
pub mod cst_json;
pub mod desugar;
pub mod dump;
pub mod eval;
pub mod expr;
pub mod item;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub fn dump_str(src: &str) -> String {
    let parse = notist_syntax::parser::parse(src);
    let mut diagnostics = parse.diagnostics.clone();
    let Some(document) = notist_syntax::ast::Document::cast(parse.syntax()) else {
        return String::new();
    };
    let expr = desugar::desugar(&document, &mut diagnostics);
    let item = eval::eval_doc(&expr.0, document.range(), expr.1, &mut diagnostics);
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
