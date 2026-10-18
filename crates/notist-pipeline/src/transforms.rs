//! Ordered same-signature execution-target selection. Semantic identities are immutable.
use notist_core::{
    definitions::{FunctionDef, FunctionId, ParameterMode, Type},
    diag::{Diagnostic, Phase},
    registry::Registry,
};
use rowan::TextRange;
#[derive(Debug, Clone)]
pub struct Replace {
    pub from: FunctionId,
    pub to: FunctionId,
    pub span: TextRange,
}
#[derive(Debug, Clone, Default)]
pub struct TransformPlan {
    replacements: Vec<Replace>,
}
impl TransformPlan {
    pub fn compile(rules: &[Replace], registry: &Registry) -> Result<Self, Vec<Diagnostic>> {
        let mut replacements = Vec::new();
        let mut diagnostics = Vec::new();
        for r in rules {
            match (registry.get(&r.from), registry.get(&r.to)) {
                (Some(a), Some(b)) => match compatible(a, b) {
                    Ok(()) => replacements.push(r.clone()),
                    Err(m) => diagnostics.push(Diagnostic::new(
                        Phase::Type,
                        r.span,
                        format!("cannot replace `{}` with `{}`: {m}", r.from, r.to),
                    )),
                },
                _ => diagnostics.push(Diagnostic::new(
                    Phase::Type,
                    r.span,
                    "unknown transform function",
                )),
            }
        }
        if diagnostics.is_empty() {
            Ok(Self { replacements })
        } else {
            Err(diagnostics)
        }
    }
    pub fn is_empty(&self) -> bool {
        self.replacements.is_empty()
    }
    pub(crate) fn revalidate(&self, registry: &Registry) -> Result<Self, Vec<Diagnostic>> {
        Self::compile(&self.replacements, registry)
    }
    /// Each rule can select the target once, in declaration order. No fixed point.
    pub fn target(&self, source: &FunctionId) -> FunctionId {
        let mut id = source.clone();
        for rule in &self.replacements {
            if id == rule.from {
                id = rule.to.clone();
            }
        }
        id
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
    if d.id.package == "notist"
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
