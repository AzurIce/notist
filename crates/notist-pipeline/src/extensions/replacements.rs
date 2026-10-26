//! Validate execution replacements against their original signature contracts.
use notist_core::{
    definitions::{FunctionDef, ParameterMode, Type},
    diag::{Diagnostic, Phase},
    extensions::Replacement,
    registry::Registry,
};

pub(super) fn validate(rules: &[Replacement], registry: &Registry) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for rule in rules {
        match (registry.get(&rule.from), registry.get(&rule.to)) {
            (Some(from), Some(to)) => {
                if let Err(message) = compatible(from, to) {
                    diagnostics.push(Diagnostic::new(
                        Phase::Type,
                        rule.span,
                        format!(
                            "cannot replace `{}` with `{}`: {message}",
                            rule.from, rule.to
                        ),
                    ));
                }
            }
            _ => diagnostics.push(Diagnostic::new(
                Phase::Type,
                rule.span,
                "unknown extension replacement function",
            )),
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}
fn normalized(t: &Type, d: &FunctionDef) -> Type {
    match t {
        Type::Parameter(n) => Type::Parameter(
            d.comptime
                .iter()
                .position(|p| p.name == *n)
                .map(|i| i.to_string())
                .unwrap_or_else(|| n.clone()),
        ),
        Type::Array(t) => Type::Array(t.as_ref().map(|t| Box::new(normalized(t, d)))),
        _ => t.clone(),
    }
}
fn role(d: &FunctionDef) -> Option<&str> {
    if d.id.module == "notist"
        && matches!(
            d.id.name.as_str(),
            "heading" | "section" | "table" | "row" | "cell" | "list" | "item" | "node"
        )
    {
        Some(&d.id.name)
    } else {
        None
    }
}
fn compatible(a: &FunctionDef, b: &FunctionDef) -> Result<(), &'static str> {
    if role(a) != role(b) {
        return Err("structural roles differ");
    }
    if a.comptime.len() != b.comptime.len()
        || a.comptime.iter().zip(&b.comptime).any(|(x, y)| {
            normalized(&x.ty, a) != normalized(&y.ty, b)
                || x.constraint != y.constraint
                || x.default.as_ref().map(|v| v.key()) != y.default.as_ref().map(|v| v.key())
        })
    {
        return Err("comptime parameter contracts differ");
    }
    if normalized(&a.returns, a) != normalized(&b.returns, b) {
        return Err("result types differ");
    }
    if a.children != b.children {
        return Err("children contracts differ");
    }
    if a.children_type.as_ref().map(|t| normalized(t, a))
        != b.children_type.as_ref().map(|t| normalized(t, b))
    {
        return Err("children contracts differ");
    }
    if a.parameters.len() != b.parameters.len()
        || a.parameters.iter().zip(&b.parameters).any(|(x, y)| {
            x.name != y.name
                || x.positional != y.positional
                || normalized(&x.ty, a) != normalized(&y.ty, b)
                || x.constraint != y.constraint
                || !same_mode(&x.mode, &y.mode)
        })
    {
        return Err("parameter contracts differ");
    }
    Ok(())
}
fn same_mode(a: &ParameterMode, b: &ParameterMode) -> bool {
    match (a, b) {
        (ParameterMode::Default(a), ParameterMode::Default(b)) => {
            notist_core::definitions::ComptimeValue::Data(a.clone()).key()
                == notist_core::definitions::ComptimeValue::Data(b.clone()).key()
        }
        _ => a == b,
    }
}
