//! Resolve data-only source literals without executing Code or reading resources.
use notist_core::{
    annotation::LiteralDict,
    diag::{Diagnostic, Phase},
    expr::Expr,
    item::{Dict, Value},
    resource::ResourcePath,
};

pub(crate) fn resolve_literal(expr: &Expr, root: &str, source: &str) -> Result<Value, Diagnostic> {
    let error = |message: String| {
        let mut diagnostic = Diagnostic::new(Phase::Type, expr.span(), message);
        diagnostic.source = Some(source.into());
        diagnostic
    };
    match expr {
        Expr::Literal(value, _) => Ok(value.clone()),
        Expr::PathLiteral { text, .. } => ResourcePath::resolve(root, source, text)
            .map(Value::Path)
            .map_err(error),
        Expr::Array(items, _) => items
            .iter()
            .map(|item| resolve_literal(item, root, source))
            .collect::<Result<notist_core::shared::Array, _>>()
            .map(Value::Array),
        Expr::Dict(fields, _) => {
            let mut result = Dict::default();
            for (key, value) in fields {
                result.insert(key, resolve_literal(value, root, source)?);
            }
            Ok(Value::Dict(result))
        }
        _ => Err(error("expected a data literal".into())),
    }
}

pub(crate) fn resolve_dict(
    dict: &LiteralDict,
    root: &str,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Dict {
    let mut result = Dict::default();
    for (key, expr) in dict.iter() {
        match resolve_literal(expr, root, source) {
            Ok(value) => result.insert(key, value),
            Err(error) => diagnostics.push(error),
        }
    }
    result
}
