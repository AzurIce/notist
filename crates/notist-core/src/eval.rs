use rowan::TextRange;

use crate::diag::{Diagnostic, Phase};

use crate::builtins::Accepts;

use crate::expr::{BodyFlavor, Expr};
use crate::item::{Ctor, Dict, Item, Value};

/// The builtin constructor registry: the single bridge from source-level
/// names to core ctors. Unknown names are not errors — they become atomic
/// custom elements (`Ctor::Custom`), data rather than behavior.
pub fn resolve(name: &str) -> Ctor {
    match name {
        "paragraph" => Ctor::Paragraph,
        "heading" => Ctor::Heading,
        "text" => Ctor::Text,
        "strong" => Ctor::Strong,
        "emph" => Ctor::Emph,
        "raw" => Ctor::RawInline,
        "math" => Ctor::Math,
        "link" => Ctor::Link,
        "list" => Ctor::List,
        "item" => Ctor::ListItem,
        "group" => Ctor::Group,
        "section" => Ctor::Section,
        _ => Ctor::Custom(name.to_string()),
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

/// What a builtin accepts as its children mount is declared in
/// `crate::builtins` (the language's static constructor table);
/// custom constructors are unconstrained.

/// Evaluate a document: a module body is a top-level expression sequence,
/// and its result is wrapped in the `Doc` root item here — `Doc` is the
/// module boundary, not a constructor anyone can call.
pub fn eval_doc(
    forest: &[Expr],
    span: TextRange,
    module_attrs: Dict,
    diagnostics: &mut Vec<Diagnostic>,
) -> Item {
    let mut doc = Item::new(Ctor::Doc, span);
    doc.children = forest.iter().map(|e| eval(e, diagnostics)).collect();
    doc.attrs = module_attrs;
    doc
}

/// M1 evaluation: no environments, no computation — constructor resolution
/// and literal normalization only. A literal inserts as text (the markup
/// insertion rule).
pub fn eval(expr: &Expr, diagnostics: &mut Vec<Diagnostic>) -> Item {
    match expr {
        Expr::Literal(value, span) => {
            let text = match value {
                Value::Str(s) => s.clone(),
                other => other.to_string(),
            };
            Item::new(Ctor::Text, *span).with_field("text", Value::Str(text))
        }
        Expr::Call {
            name,
            args,
            fields,
            children,
            body,
            attrs,
            span,
        } => {
            let ctor = resolve(name);
            let sig = crate::builtins::builtin_signature(name).map(|s| s.accepts);
            let mut item = Item::new(ctor, *span);
            item.fields = fields.clone();
            let mut extra = Vec::new();
            for (i, arg) in args.iter().enumerate() {
                let Some(value) = arg.literal_value() else {
                    continue;
                };
                match positional_field(name, i) {
                    Some(field) => item.fields.insert(field, value),
                    None => extra.push(value),
                }
            }
            if !extra.is_empty() {
                if matches!(item.ctor, Ctor::Custom(_)) {
                    item.fields.insert("args", Value::Array(extra));
                } else {
                    diagnostics.push(Diagnostic {
                phase: Phase::Type,
                        span: *span,
                        message: format!("too many positional arguments for `{name}`"),
                    });
                }
            }
            item.children = children.iter().map(|c| eval(c, diagnostics)).collect();
            match (body, sig) {
                (BodyFlavor::Inline | BodyFlavor::Block, Some(Accepts::Nothing)) => {
                    diagnostics.push(Diagnostic {
                phase: Phase::Type,
                        span: *span,
                        message: format!("`{name}` takes no children"),
                    });
                }
                (BodyFlavor::Block, Some(Accepts::Inline)) => {
                    diagnostics.push(Diagnostic {
                phase: Phase::Type,
                        span: *span,
                        message: format!(
                            "`{name}` takes inline content; write the body as `[..]` without inner padding"
                        ),
                    });
                }
                (BodyFlavor::Inline, Some(Accepts::Content)) => {
                    let promoted = std::mem::take(&mut item.children);
                    item.children = vec![Item::new(Ctor::Paragraph, *span).with_children(promoted)];
                }
                _ => {}
            }
            item.attrs = attrs.clone();
            item
        }
    }
}
