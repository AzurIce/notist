use crate::builtins::{self, Accepts};
use crate::diag::{Diagnostic, Phase};
use crate::expr::{BodyFlavor, Expr, RExpr};
use crate::item::{Ctor, Value};

/// IR₁ → IR₂: resolve source-level names to constructors and validate each
/// call against its signature. An unknown name is a type error; the call is
/// kept as an atomic `Ctor::Custom` recovery node and placed by its declared
/// flavor (a block-written body's block-sequence children must stay in a
/// block-level position to remain coherent). Per-node normalization happens
/// here too: positional args map to fields, an inline mount on a `Content`
/// constructor is promoted to a one-paragraph block, and literals insert as
/// text (the markup insertion rule).
pub fn resolve(forest: Vec<Expr>, diagnostics: &mut Vec<Diagnostic>) -> Vec<RExpr> {
    forest
        .into_iter()
        .map(|expr| resolve_expr(expr, diagnostics))
        .collect()
}

fn resolve_expr(expr: Expr, diags: &mut Vec<Diagnostic>) -> RExpr {
    match expr {
        Expr::Literal(value, span) => {
            let text = match value {
                Value::Str(s) => s,
                other => other.to_string(),
            };
            RExpr::resolved(Ctor::Text, span).with_field("text", Value::Str(text))
        }
        Expr::Call {
            name,
            args,
            mut fields,
            children,
            body,
            attrs,
            span,
        } => {
            let children: Vec<RExpr> = children
                .into_iter()
                .map(|child| resolve_expr(child, diags))
                .collect();
            let sig = builtins::builtin_signature(&name);
            let ctor = match Ctor::from_name(&name) {
                Some(ctor) => ctor,
                None => {
                    diags.push(Diagnostic {
                        phase: Phase::Type,
                        span,
                        message: format!("unknown constructor `{name}`"),
                    });
                    Ctor::Custom(name.clone())
                }
            };
            let mut extra = Vec::new();
            for (i, arg) in args.iter().enumerate() {
                let Some(value) = arg.literal_value() else {
                    continue;
                };
                match positional_field(&name, i) {
                    Some(field) => fields.insert(field, value),
                    None => extra.push(value),
                }
            }
            if !extra.is_empty() {
                if sig.is_none() {
                    fields.insert("args", Value::Array(extra));
                } else {
                    diags.push(Diagnostic {
                        phase: Phase::Type,
                        span,
                        message: format!("too many positional arguments for `{name}`"),
                    });
                }
            }
            let mut children = children;
            match (body, sig.map(|s| s.accepts)) {
                (BodyFlavor::Inline | BodyFlavor::Block, Some(Accepts::Nothing)) => {
                    diags.push(Diagnostic {
                        phase: Phase::Type,
                        span,
                        message: format!("`{name}` takes no children"),
                    });
                }
                (BodyFlavor::Block, Some(Accepts::Inline)) => {
                    diags.push(Diagnostic {
                        phase: Phase::Type,
                        span,
                        message: format!(
                            "`{name}` takes inline content; write the body as `[..]` without inner padding"
                        ),
                    });
                }
                (BodyFlavor::Inline, Some(Accepts::Content)) => {
                    children =
                        vec![RExpr::resolved(Ctor::Paragraph, span).with_children(children)];
                }
                _ => {}
            }
            Expr::Call {
                name: ctor,
                args: Vec::new(),
                fields,
                children,
                body,
                attrs,
                span,
            }
        }
    }
}

/// Which field a builtin's n-th positional argument maps to.
fn positional_field(name: &str, index: usize) -> Option<&'static str> {
    match (name, index) {
        ("link", 0) => Some("target"),
        ("raw", 0) | ("math", 0) => Some("text"),
        _ => None,
    }
}
