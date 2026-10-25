//! Common syntax-to-IR conversion for built-in and externally registered
//! frontends. Per-frontend document lowering lives in `crate::frontend`.
pub mod imports;

pub mod code;
pub mod content;
use content::ContentLowering;
pub mod literals;

use literals::{key_text, syntax_value, value_children};
use notist_core::diag::{Diagnostic, Phase};
use notist_core::expr::{BodyFlavor, CallArg, Expr};
use notist_core::item::{Dict, Value};
use notist_syntax::tree::SyntaxKind;
use notist_syntax::{
    Parse,
    code::ast::{Annotation, CodeCall, Entry},
};
use rowan::NodeOrToken;

/// The dict carried by an `@(dict)` annotation (stray members diagnosed).
pub fn annotation_dict(annotation: &Annotation, diags: &mut Vec<Diagnostic>) -> Dict {
    let mut dict = Dict::default();
    if let Some(node) = annotation.payload_dict() {
        for el in value_children(&node) {
            // the colon of the empty-dict spelling `(:)` is structural
            if !matches!(el.kind(), SyntaxKind::Entry | SyntaxKind::Colon) {
                diags.push(Diagnostic {
                    source: None,
                    code: None,
                    recovered: false,
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
pub fn call_header(
    call: &CodeCall,
    has_body: bool,
    parsed: &Parse,
    frontend: &dyn ContentLowering,
    diags: &mut Vec<Diagnostic>,
) -> Expr {
    let span = call.range();
    let mut args = Vec::new();
    for el in call.args() {
        if let NodeOrToken::Node(node) = &el
            && node.kind() == SyntaxKind::Entry
        {
            let entry = Entry::cast(node.clone()).unwrap();
            if let (Some(key), Some(value)) = (entry.key_token(), entry.value())
                && let (Some(name), Some(value)) = (
                    key_text(&key, diags),
                    code::expression(value, Some(parsed), Some(frontend), diags),
                )
            {
                args.push(CallArg::Named { name, value });
            }
            continue;
        }
        if let Some(value) = code::expression(el, Some(parsed), Some(frontend), diags) {
            args.push(CallArg::Positional(value));
        }
    }
    let callee = call
        .callee()
        .and_then(|node| {
            code::expression(NodeOrToken::Node(node), Some(parsed), Some(frontend), diags)
        })
        .unwrap_or_else(|| Expr::reference("<anonymous>", span));
    if !has_body
        && !call
            .syntax()
            .children_with_tokens()
            .any(|e| matches!(e.kind(), SyntaxKind::LParen | SyntaxKind::ComptimeArguments))
    {
        return callee;
    }
    let comptime = call
        .syntax()
        .children()
        .find(|n| n.kind() == SyntaxKind::ComptimeArguments)
        .map(|n| code::arguments(&n, Some(parsed), Some(frontend), diags))
        .unwrap_or_default();
    Expr::Call {
        callee: Box::new(callee),
        args,
        comptime,
        children: Vec::new(),
        body: BodyFlavor::None,
        attrs: Dict::default(),
        span,
    }
}
