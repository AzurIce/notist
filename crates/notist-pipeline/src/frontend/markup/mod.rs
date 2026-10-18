//! Native Notist Markup frontend and syntax-to-IR lowering.
//!
//! Parsing and AST definitions live in `notist-syntax::markup` and
//! `notist-syntax::code`; shared conversion lives in `crate::lowering`.
//!
//! ```
//! use notist_syntax::markup::parse_document;
//! use notist_pipeline::frontend::markup::lower_document;
//!
//! let parsed = parse_document("@(id: \"intro\")\n= Title");
//! let mut diagnostics = Vec::new();
//! let (forest, attrs, annotations) = lower_document(&parsed, &mut diagnostics);
//! assert!(diagnostics.is_empty());
//! assert!(attrs.is_empty());
//! assert_eq!(annotations.len(), 1);
//! assert_eq!(forest.len(), 1);
//! ```

mod lowering;
pub use lowering::lower_document;

use notist_core::diag::{Diagnostic, Phase};
use notist_core::frontend::{Frontend, FrontendOptions, FrontendOutput};
use notist_syntax::markup;

#[derive(Debug, Default)]
pub struct NotistFrontend;

impl notist_syntax::content::ContentParser for NotistFrontend {
    fn parse_content(
        &self,
        source: &str,
        open: rowan::TextSize,
    ) -> notist_syntax::content::ParsedContent {
        notist_syntax::content::ContentParser::parse_content(
            &markup::MarkupContentParser,
            source,
            open,
        )
    }
}

impl crate::lowering::content::ContentLowering for NotistFrontend {
    fn lower_content(
        &self,
        content: &notist_syntax::content::ParsedContent,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> crate::lowering::content::Lowered {
        let parsed = content
            .syntax
            .as_ref()
            .as_any()
            .downcast_ref::<notist_syntax::Parse>()
            .expect("native content parser returns a native Parse");
        let mut local_diags = Vec::new();
        let mut lowered = lowering::lower_content(parsed, content.flavor, &mut local_diags);
        for expr in &mut lowered.forest {
            expr.map_spans(&mut |span| span + content.body.start());
        }
        for annotation in &mut lowered.annotations {
            annotation.map_spans(&mut |span| span + content.body.start());
        }
        diagnostics.extend(local_diags.into_iter().map(|mut d| {
            d.span += content.body.start();
            d
        }));
        lowered
    }
}

impl Frontend for NotistFrontend {
    fn extensions(&self) -> &[&str] {
        &["not"]
    }

    fn compile(&self, source: &str, options: FrontendOptions) -> FrontendOutput {
        let parsed = markup::parse_document(source);
        let syntax = parsed.syntax();
        let mut diagnostics = parsed
            .diagnostics
            .iter()
            .map(|diagnostic| {
                Diagnostic::new(Phase::Syntax, diagnostic.span, diagnostic.message.clone())
            })
            .collect();
        let (forest, module_attrs, annotations) = lower_document(&parsed, &mut diagnostics);
        FrontendOutput {
            forest,
            annotations,
            module_attrs,
            diagnostics,
            syntax: options.capture_syntax.then(|| Box::new(syntax) as _),
        }
    }
}
