//! Configurable source frontends producing the shared Notist document IR.
//!
//! ```
//! use notist::{Ctor, Notist};
//!
//! let notist = Notist::default(); // .not, .md, and .markdown
//! let document = notist.analyze("example.md", "# Title\n")?;
//! assert!(document.diagnostics().is_empty());
//! assert!(document.root().descendants().any(|node| node.ctor == Ctor::Heading));
//! # Ok::<(), notist::UnsupportedFormat>(())
//! ```
//!
//! Renderers consume [`Item`], the same type exported by `notist-core`, so
//! output crates can depend on `notist-core` without depending on frontends.

pub use rowan::{TextRange, TextSize};

pub use analysis::{Analysis, Notist, UnsupportedFormat};
pub use definitions::analyze_module;
pub use frontend::{Frontend, Frontends};
pub use notist_core::definitions::{
    DefinitionModule, FunctionDef, FunctionId, ParameterDef, ParameterMode, ReturnRule,
    ValueConstraint, ValueType,
};
pub use notist_core::diag::{Diagnostic, Phase};
pub use notist_core::item::{Ctor, Dict, Item, Value};
pub use notist_core::registry::Registry;
pub use notist_core::{
    builtins, diag, dump, expr, index, item, materialize, registry, resolve, shape,
};
pub use notist_syntax as syntax;

pub mod analysis;
pub mod cst_json;
pub mod definitions;
pub mod desugar;
pub mod frontend;
mod literals;
#[cfg(not(target_arch = "wasm32"))]
pub mod lsp;
pub mod query;
pub mod vault;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// Analyze a `.not` source with the full pipeline: parse → desugar, then the shared backend
/// (resolve → shape → materialize). Diagnostics are collected per phase
/// (syntax from the parser, semantic from desugar, type from resolve).
/// Use [`Notist::analyze`] to select a frontend by file extension.
pub fn analyze(src: &str) -> (item::Item, Vec<diag::Diagnostic>) {
    let (forest, module_attrs, mut diagnostics) = desugar::lower_not(src);
    let span = TextRange::new(0.into(), (src.len() as u32).into());
    let item = notist_core::analyze(forest, span, module_attrs, &mut diagnostics);
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
