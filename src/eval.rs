use rowan::TextRange;

use notist_syntax::parser::Diagnostic;

use crate::expr::Expr;
use crate::item::{Ctor, Dict, Item, Value};

/// The builtin constructor registry: the single bridge from source-level
/// names to core ctors. User scopes and plugin namespaces join here later.
pub fn resolve(name: &str) -> Option<Ctor> {
    Some(match name {
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
        _ => return None,
    })
}

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

/// Trivial evaluation: markup desugars to constructor calls with literal
/// arguments only. A literal inserts as text (the markup insertion rule);
/// an unresolved name is a diagnostic plus an empty text placeholder.
pub fn eval(expr: &Expr, diagnostics: &mut Vec<Diagnostic>) -> Item {
    match expr {
        Expr::Literal(value, span) => {
            let text = match value {
                Value::Str(s) => s.clone(),
                other => other.to_string(),
            };
            Item::new(Ctor::Text, *span).with_field("text", Value::Str(text))
        }
        Expr::Embed { text, span } => {
            Item::new(Ctor::CodeEmbed, *span).with_field("text", Value::Str(text.clone()))
        }
        Expr::Call {
            name,
            fields,
            children,
            attrs,
            span,
        } => match resolve(name) {
            Some(ctor) => {
                let mut item = Item::new(ctor, *span);
                item.fields = fields.clone();
                item.children = children.iter().map(|c| eval(c, diagnostics)).collect();
                item.attrs = attrs.clone();
                item
            }
            None => {
                diagnostics.push(Diagnostic {
                    span: *span,
                    message: format!("unknown constructor: {name}"),
                });
                Item::new(Ctor::Text, *span)
            }
        },
    }
}
