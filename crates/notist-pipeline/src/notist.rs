//! Native `.not` frontend: source parsing followed by document AST lowering.
use notist_core::diag::{Diagnostic, Phase};
use notist_core::frontend::{Frontend, FrontendOptions, FrontendOutput};
use notist_syntax::ast::Document;

#[derive(Debug, Default)]
pub struct NotistFrontend;

impl Frontend for NotistFrontend {
    fn extensions(&self) -> &[&str] {
        &["not"]
    }

    fn compile(&self, source: &str, options: FrontendOptions) -> FrontendOutput {
        let parsed = notist_syntax::parse_document(source);
        let syntax = parsed.syntax();
        let document = Document::cast(syntax.clone()).expect("parse_document returns a Document");
        let mut diagnostics = parsed
            .diagnostics
            .into_iter()
            .map(|diagnostic| Diagnostic::new(Phase::Syntax, diagnostic.span, diagnostic.message))
            .collect();
        let (forest, module_attrs) = notist_lowering::lower_document(&document, &mut diagnostics);
        FrontendOutput {
            forest,
            module_attrs,
            diagnostics,
            syntax: options.capture_syntax.then(|| Box::new(syntax) as _),
        }
    }
}
