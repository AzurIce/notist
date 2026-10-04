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
    resolve_with_registry(forest, builtins::registry(), diagnostics)
}

pub fn resolve_with_registry(
    forest: Vec<Expr>,
    registry: &crate::registry::Registry,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<RExpr> {
    forest
        .into_iter()
        .map(|expr| resolve_expr(expr, registry, diagnostics))
        .collect()
}

fn resolve_expr(
    expr: Expr,
    registry: &crate::registry::Registry,
    diags: &mut Vec<Diagnostic>,
) -> RExpr {
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
                .map(|child| resolve_expr(child, registry, diags))
                .collect();
            let definition = registry.resolve(&name);
            let sig = definition
                .as_ref()
                .ok()
                .map(|definition| builtins::CtorSignature {
                    accepts: definition.children,
                    level: definition.returns.base_level(),
                });
            let mut ctor = match &definition {
                Ok(definition) if definition.id.package == "notist" => {
                    Ctor::from_name(&definition.id.name)
                        .expect("the standard registry contains native constructors")
                }
                Ok(definition) => Ctor::Extension(crate::item::ExtensionCtor {
                    id: definition.id.clone(),
                    accepts: definition.children,
                    level: definition.returns.level(&fields, body),
                }),
                Err(error) => {
                    let message = match error {
                        crate::registry::LookupError::Unknown => {
                            format!("unknown constructor `{name}`")
                        }
                        crate::registry::LookupError::UnsupportedPath => {
                            format!("module paths are not supported yet: `{name}`")
                        }
                    };
                    diags.push(Diagnostic::new(Phase::Type, span, message));
                    Ctor::Custom(name.clone())
                }
            };
            let positional = args.iter().filter_map(Expr::literal_value);
            if let Ok(definition) = definition {
                fields = definition.bind_fields(positional, fields, &name, span, diags);
                if let Ctor::Extension(function) = &mut ctor {
                    function.level = definition.returns.level(&fields, body);
                }
            } else {
                let extra: Vec<_> = positional.collect();
                if !extra.is_empty() {
                    fields.insert("args", Value::Array(extra));
                }
            }
            if sig.is_some_and(|sig| sig.accepts == Accepts::Inline)
                && body != BodyFlavor::Block
                && !(ctor == Ctor::Paragraph && body == BodyFlavor::None)
            {
                for child in &children {
                    if contains_block(child) {
                        diags.push(Diagnostic::new(
                            Phase::Type,
                            child.span(),
                            format!(
                                "`{name}` takes inline content; block children are not allowed"
                            ),
                        ));
                    }
                }
            }
            if sig.is_some_and(|sig| sig.accepts == Accepts::Nothing)
                && body == BodyFlavor::None
                && !children.is_empty()
            {
                diags.push(Diagnostic::new(
                    Phase::Type,
                    span,
                    format!("`{name}` takes no children"),
                ));
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
                    if !matches!(ctor, Ctor::TableCell | Ctor::ListItem) || !children.is_empty() {
                        children =
                            vec![RExpr::resolved(Ctor::Paragraph, span).with_children(children)];
                    }
                }
                _ => {}
            }
            if matches!(
                sig.map(|sig| sig.accepts),
                Some(Accepts::Items | Accepts::Rows | Accepts::Cells)
            ) {
                children = structural_children(children);
            }
            match sig.map(|s| s.accepts) {
                Some(Accepts::Rows) => {
                    check_structural_children(&children, &Ctor::TableRow, &name, diags)
                }
                Some(Accepts::Cells) => {
                    check_structural_children(&children, &Ctor::TableCell, &name, diags)
                }
                Some(Accepts::Items) => {
                    check_structural_children(&children, &Ctor::ListItem, &name, diags)
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

/// Block-written call bodies contain paragraph candidates until shaping.
/// Inspect those wrappers too, so stray text is diagnosed without rejecting
/// the row/cell calls that reflow will expose as direct structural children.
fn check_structural_children(
    children: &[RExpr],
    expected: &Ctor,
    parent: &str,
    diags: &mut Vec<Diagnostic>,
) {
    for child in children {
        match child {
            Expr::Call {
                name: Ctor::Group,
                children,
                ..
            } => {
                check_structural_children(children, expected, parent, diags);
            }
            Expr::Call {
                name: Ctor::Paragraph,
                children,
                body: BodyFlavor::None,
                ..
            } => {
                check_structural_children(children, expected, parent, diags);
            }
            Expr::Call { name, .. } if name == expected => {}
            _ => diags.push(Diagnostic::new(
                Phase::Type,
                child.span(),
                format!(
                    "`{parent}` takes only {} children",
                    if *expected == Ctor::TableRow {
                        "row"
                    } else if *expected == Ctor::TableCell {
                        "cell"
                    } else {
                        "item"
                    }
                ),
            )),
        }
    }
}

fn contains_block(expr: &RExpr) -> bool {
    if crate::shape::reflow::is_block_element(expr) {
        return true;
    }
    matches!(expr, Expr::Call { name: Ctor::Group, children, .. } if children.iter().any(contains_block))
}

/// Whitespace between structural calls is a separator, not an element.
fn structural_children(children: Vec<RExpr>) -> Vec<RExpr> {
    children.into_iter().filter_map(|mut child| {
        if let Expr::Call { name, fields, attrs, children, body, .. } = &mut child {
            if *name == Ctor::Text && attrs.is_empty()
                && matches!(fields.get("text"), Some(Value::Str(text)) if text.trim().is_empty()) {
                return None;
            }
            if *name == Ctor::Group || *name == Ctor::Paragraph && *body == BodyFlavor::None {
                *children = structural_children(std::mem::take(children));
                if children.is_empty() && attrs.is_empty() && fields.is_empty() { return None; }
            }
        }
        Some(child)
    }).collect()
}
