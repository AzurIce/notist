//! Notist document AST → unresolved Expr forest (IR₁).
//!
//! This crate owns native document lowering and the annotation, call-header
//! and literal lowering shared with the Markdown frontend. Name resolution,
//! shaping and materialization belong to `notist-pipeline`.
//!
//! ```
//! let parsed = notist_syntax::parse_document("@(id: \"intro\")\n= Title");
//! let document = notist_syntax::ast::Document::cast(parsed.syntax()).unwrap();
//! let mut diagnostics = Vec::new();
//! let (forest, attrs) = notist_lowering::lower_document(&document, &mut diagnostics);
//! assert!(diagnostics.is_empty());
//! assert!(attrs.is_empty());
//! assert_eq!(forest.len(), 1);
//! ```
mod document;
pub use document::lower_document;
pub mod literals;

use literals::{key_text, syntax_value, value_children};
use notist_core::diag::{Diagnostic, Phase};
use notist_core::expr::{BodyFlavor, Expr};
use notist_core::item::{Dict, Value};
use notist_syntax::ast::{Annotation, CodeCall, Entry};
use notist_syntax::syntax::SyntaxKind;
use rowan::NodeOrToken;

/// The dict carried by an `@(dict)` annotation (stray members diagnosed).
pub fn annotation_dict(annotation: &Annotation, diags: &mut Vec<Diagnostic>) -> Dict {
    let mut dict = Dict::default();
    if let Some(node) = annotation.payload_dict() {
        for el in value_children(&node) {
            // the colon of the empty-dict spelling `(:)` is structural
            if !matches!(el.kind(), SyntaxKind::Entry | SyntaxKind::Colon) {
                diags.push(Diagnostic {
                    phase: Phase::Semantic,
                    span: el.text_range(),
                    message: "annotation entries must be `key: value`".to_string(),
                });
            }
        }
        if let Some(Value::Dict(d)) = syntax_value(&NodeOrToken::Node(node), diags) {
            dict = d;
        }
    }
    dict
}

/// Lower the call target and arguments; the frontend supplies its body.
pub fn call_header(call: &CodeCall, diags: &mut Vec<Diagnostic>) -> Expr {
    let span = call.range();
    let mut args = Vec::new();
    let mut fields = Dict::default();
    let arg_els = call.args();
    let mut i = 0;
    while i < arg_els.len() {
        match &arg_els[i] {
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::Entry => {
                let entry = Entry::cast(n.clone()).unwrap();
                let Some(key_token) = entry.key_token() else {
                    i += 1;
                    continue;
                };
                let Some(key) = key_text(&key_token, diags) else {
                    i += 1;
                    continue;
                };
                let Some(value_el) = entry.value() else {
                    i += 1;
                    continue;
                };
                if value_el.kind() == SyntaxKind::LBracket {
                    diags.push(Diagnostic {
                        phase: Phase::Semantic,
                        span: value_el.text_range(),
                        message: "content literals as entry values are not supported yet"
                            .to_string(),
                    });
                    i += 1;
                    continue;
                }
                if let Some(value) = syntax_value(&value_el, diags) {
                    fields.insert(key, value);
                }
                i += 1;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::LBracket => {
                // content is mounted via the body slot, never passed as an argument
                let open_span = t.text_range();
                diags.push(Diagnostic {
                    phase: Phase::Semantic,
                    span: open_span,
                    message:
                        "content is mounted with `[..]` after the call, not passed as an argument"
                            .to_string(),
                });
                let mut depth = 0usize;
                i += 1;
                while i < arg_els.len() {
                    match arg_els[i].kind() {
                        SyntaxKind::LBracket => depth += 1,
                        SyntaxKind::RBracket => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
            }
            other => {
                if let Some(value) = syntax_value(other, diags) {
                    args.push(Expr::Literal(value, span));
                }
                i += 1;
            }
        }
    }
    // `#[..]` is the anonymous constructor call: a transparent group node
    let name = call.name().unwrap_or_else(|| "group".to_string());
    Expr::Call {
        name,
        args,
        fields,
        children: Vec::new(),
        body: BodyFlavor::None,
        attrs: Dict::default(),
        span,
    }
}
