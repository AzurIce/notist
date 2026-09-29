pub mod ast;
pub mod cst_json;
pub mod dump;
pub mod item;
pub mod lexer;
pub mod lower;
pub mod parser;
pub mod syntax;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use item::{Ctor, Dict, Item, Value};
pub use parser::{Diagnostic, Parse, parse};

pub fn dump_str(src: &str) -> String {
    let parse = parse(src);
    let mut out = String::new();
    for d in &parse.diagnostics {
        out.push_str(&format!(
            "error @{}..{}: {}\n",
            u32::from(d.span.start()),
            u32::from(d.span.end()),
            d.message
        ));
    }
    let Some(document) = ast::Document::cast(parse.syntax()) else {
        return out;
    };
    out.push_str(&dump::dump(&lower::lower(&document)));
    out
}
