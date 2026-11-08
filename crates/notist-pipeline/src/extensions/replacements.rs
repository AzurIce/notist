//! Validate execution replacements against their original signature contracts.
use notist_core::{
    definitions::{FunctionDef, ParameterMode},
    diag::{Diagnostic, Phase},
    extensions::Replacement,
    registry::Registry,
    signature::ResolvedSignature,
};

pub(super) fn validate(rules: &[Replacement], registry: &Registry) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for rule in rules {
        match (
            registry.signature(rule.from.def_id()),
            registry.signature(rule.to.def_id()),
        ) {
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
fn compatible(
    a_signature: &ResolvedSignature,
    b_signature: &ResolvedSignature,
) -> Result<(), String> {
    let a = a_signature.declaration();
    let b = b_signature.declaration();
    let same = |a, b| {
        a_signature
            .alpha_equivalent_type(a, b_signature, b)
            .map_err(|error| error.to_string())
    };
    if role(a) != role(b) {
        return Err("structural roles differ".into());
    }
    if a.comptime.len() != b.comptime.len() {
        return Err("comptime parameter contracts differ".into());
    }
    for (((x, y), xt), yt) in a
        .comptime
        .iter()
        .zip(&b.comptime)
        .zip(a_signature.comptime())
        .zip(b_signature.comptime())
    {
        if !same(&xt.ty, &yt.ty)?
            || x.constraint != y.constraint
            || x.default.as_ref().map(|v| v.key()) != y.default.as_ref().map(|v| v.key())
        {
            return Err("comptime parameter contracts differ".into());
        }
    }
    if !same(a_signature.returns(), b_signature.returns())? {
        return Err("result types differ".into());
    }
    if a.children != b.children {
        return Err("children contracts differ".into());
    }
    match (a_signature.children_type(), b_signature.children_type()) {
        (Some(a), Some(b)) if same(a, b)? => {}
        (None, None) => {}
        _ => return Err("children contracts differ".into()),
    }
    if a.parameters.len() != b.parameters.len() {
        return Err("parameter contracts differ".into());
    }
    for (((x, y), xt), yt) in a
        .parameters
        .iter()
        .zip(&b.parameters)
        .zip(a_signature.parameters())
        .zip(b_signature.parameters())
    {
        if x.name != y.name
            || x.positional != y.positional
            || !same(xt, yt)?
            || x.constraint != y.constraint
            || !same_mode(&x.mode, &y.mode)
        {
            return Err("parameter contracts differ".into());
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use notist_core::{
        builtins::Accepts,
        definitions::{
            ComptimeParameter, DefinitionModule, FunctionId, ParameterDef, Type, ValueConstraint,
        },
    };
    #[test]
    fn replacement_alpha_equivalence_uses_parameter_positions_not_spellings_or_ids() {
        for swapped in [false, true] {
            let mut module = DefinitionModule::new("resolved-alpha-contract");
            for (name, first, second, swap) in
                [("from", "T", "U", false), ("to", "X", "Y", swapped)]
            {
                let mut definition = FunctionDef::new(
                    FunctionId::new("resolved-alpha-contract", name),
                    Accepts::Nothing,
                    Type::InlineContent,
                );
                for parameter in [first, second] {
                    definition.comptime.push(ComptimeParameter {
                        name: parameter.into(),
                        ty: Type::Type,
                        default: None,
                        constraint: ValueConstraint::Any,
                        span: Default::default(),
                    });
                }
                let (a, b) = if swap {
                    (second, first)
                } else {
                    (first, second)
                };
                definition.parameters.push(ParameterDef::new(
                    "value",
                    Type::Result(
                        Box::new(Type::Parameter(a.into())),
                        Box::new(Type::Parameter(b.into())),
                    ),
                    ParameterMode::Required,
                ));
                module.functions.push(definition);
            }
            let mut registry = Registry::new();
            registry.register(module).unwrap();
            let rule = Replacement {
                from: FunctionId::new("resolved-alpha-contract", "from"),
                to: FunctionId::new("resolved-alpha-contract", "to"),
                span: Default::default(),
            };
            let result = validate(&[rule], &registry);
            assert_eq!(result.is_err(), swapped, "{result:?}");
        }
    }
}
