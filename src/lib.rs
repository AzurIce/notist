pub mod cst_json;
pub mod dump;
pub mod item;
pub mod lower;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub fn dump_str(src: &str) -> String {
    let parse = notist_syntax::parser::parse(src);
    let mut out = String::new();
    for d in &parse.diagnostics {
        out.push_str(&format!(
            "error @{}..{}: {}\n",
            u32::from(d.span.start()),
            u32::from(d.span.end()),
            d.message
        ));
    }
    let Some(document) = notist_syntax::ast::Document::cast(parse.syntax()) else {
        return out;
    };
    out.push_str(&dump::dump(&lower::lower(&document)));
    out
}
