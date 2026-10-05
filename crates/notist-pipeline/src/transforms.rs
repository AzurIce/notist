//! Ordered transformations of materialized IR. Plans bind to an immutable
//! definition environment; neither compilation nor execution performs IO.
use notist_core::{
    builtins::{Accepts, Level},
    definitions::{FunctionDef, FunctionId, ParameterMode, ReturnRule},
    diag::{Diagnostic, Phase},
    expr::BodyFlavor,
    item::{Ctor, ExtensionCtor, Item, Value},
    registry::Registry,
};
use rowan::TextRange;

/// A function-identity replacement. The span belongs to the configuration,
/// while execution diagnostics refer to the original document node.
#[derive(Debug, Clone)]
pub struct Replace {
    pub from: FunctionId,
    pub to: FunctionId,
    pub span: TextRange,
}

#[derive(Debug, Clone)]
struct BoundReplace {
    source: FunctionDef,
    target: FunctionDef,
}

/// A complete, signature-checked plan. Failed compilation never returns a
/// partial plan. Definition snapshots make execution independent of later IO.
#[derive(Debug, Clone, Default)]
pub struct TransformPlan {
    replacements: Vec<BoundReplace>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransformOutput {
    pub root: Item,
    pub diagnostics: Vec<Diagnostic>,
}

impl TransformPlan {
    pub fn compile(rules: &[Replace], registry: &Registry) -> Result<Self, Vec<Diagnostic>> {
        let mut replacements = Vec::new();
        let mut diagnostics = Vec::new();
        for rule in rules {
            let mut lookup = |id: &FunctionId| {
                let definition = registry.get(id);
                if definition.is_none() {
                    diagnostics.push(Diagnostic::new(
                        Phase::Type,
                        rule.span,
                        format!("unknown transform function `{id}`"),
                    ));
                }
                definition
            };
            let source = lookup(&rule.from);
            let target = lookup(&rule.to);
            let (Some(source), Some(target)) = (source, target) else {
                continue;
            };
            let result = compatible(source, target);
            match result {
                Ok(()) => {
                    replacements.push(BoundReplace {
                        source: source.clone(),
                        target: target.clone(),
                    });
                }
                Err(reason) => diagnostics.push(Diagnostic::new(
                    Phase::Type,
                    rule.span,
                    format!(
                        "cannot replace `{}` with `{}`: {reason}",
                        rule.from, rule.to
                    ),
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

    /// Clone the analysis tree, then visit it once per rule in configuration
    /// order. Replaced nodes are not re-matched by that same rule.
    pub fn apply(&self, root: &Item) -> TransformOutput {
        let mut output = TransformOutput {
            root: root.clone(),
            diagnostics: Vec::new(),
        };
        for replacement in &self.replacements {
            replacement.visit(&mut output.root, &mut output.diagnostics);
        }
        output
    }
}

fn structural(definition: &FunctionDef) -> bool {
    matches!(
        definition.children,
        Accepts::Items | Accepts::Rows | Accepts::Cells
    ) || (definition.id.package == "notist"
        && matches!(
            definition.id.name.as_str(),
            "heading" | "section" | "list" | "item" | "table" | "row" | "cell"
        ))
}

fn compatible(source: &FunctionDef, target: &FunctionDef) -> Result<(), &'static str> {
    if structural(source) || structural(target) {
        return Err("structural functions are not supported by replace");
    }
    match (&source.returns, &target.returns) {
        (ReturnRule::Fixed(a), ReturnRule::Fixed(b)) if a == b && *a != Level::Inherit => {}
        (ReturnRule::BlockIfTrue(a), ReturnRule::BlockIfTrue(b)) if a == b => {}
        (ReturnRule::Inherit, _) | (_, ReturnRule::Inherit) => {
            return Err("replace requires fixed levels or matching boolean content rules");
        }
        _ => return Err("content levels differ"),
    }
    if source.children != target.children {
        return Err("children contracts differ");
    }
    if source.parameters.len() != target.parameters.len()
        || source.parameters.iter().any(|a| {
            target
                .parameters
                .iter()
                .find(|b| a.name == b.name)
                .is_none_or(|b| {
                    a.ty != b.ty || a.constraint != b.constraint || !same_mode(&a.mode, &b.mode)
                })
        })
    {
        return Err("parameter contracts differ");
    }
    Ok(())
}

fn same_mode(a: &ParameterMode, b: &ParameterMode) -> bool {
    match (a, b) {
        (ParameterMode::Default(a), ParameterMode::Default(b)) => same_value(a, b),
        _ => a == b,
    }
}

// Defaults include native nonfinite floats and ordered dictionaries. Comparing
// bits also distinguishes negative zero and preserves NaN payloads.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_value(a, b))
        }
        (Value::Dict(a), Value::Dict(b)) => {
            let a: Vec<_> = a.iter().collect();
            let b: Vec<_> = b.iter().collect();
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((ak, av), (bk, bv))| *ak == bk && same_value(av, bv))
        }
        _ => a == b,
    }
}

impl BoundReplace {
    fn visit(&self, item: &mut Item, diagnostics: &mut Vec<Diagnostic>) {
        if item.function_id().as_ref() == Some(&self.source.id) {
            let mut errors = Vec::new();
            self.source.validate_fields(
                &item.fields,
                &self.source.id.to_string(),
                item.span,
                &mut errors,
            );
            let defaults_present = self.source.parameters.iter().all(|p| {
                !matches!(p.mode, ParameterMode::Default(_)) || item.fields.get(&p.name).is_some()
            });
            let known_fields = item.fields.iter().all(|(name, _)| {
                self.source
                    .parameters
                    .iter()
                    .any(|parameter| parameter.name == name)
            });
            let children_valid = match self.source.children {
                Accepts::Nothing => item.children.is_empty(),
                Accepts::Inline => item
                    .children
                    .iter()
                    .all(|child| child.level == Level::Inline),
                Accepts::Content | Accepts::Any => true,
                Accepts::Items | Accepts::Rows | Accepts::Cells => false,
            };
            let level = self.source.returns.level(&item.fields, BodyFlavor::None);
            let ctor_level = if matches!(item.ctor, Ctor::Extension(_)) {
                level
            } else {
                self.source.returns.base_level()
            };
            let snapshot_valid = item.ctor.accepts() == Some(self.source.children)
                && item.ctor.level() == Some(ctor_level);
            if errors.is_empty()
                && defaults_present
                && known_fields
                && children_valid
                && snapshot_valid
                && item.level == level
            {
                item.ctor = if self.target.id.package == "notist" {
                    Ctor::from_name(&self.target.id.name).expect("registered builtin")
                } else {
                    Ctor::Extension(ExtensionCtor {
                        id: self.target.id.clone(),
                        accepts: self.target.children,
                        level,
                    })
                };
            } else {
                let reason = errors
                    .first()
                    .map(|error| error.message.as_str())
                    .unwrap_or(
                        "fields, children or content level violate the materialized contract",
                    );
                diagnostics.push(Diagnostic::new(
                    Phase::Type,
                    item.span,
                    format!("replace skipped for `{}`: {reason}", self.source.id),
                ));
            }
        }
        // A recovery parent never prevents valid descendants from transforming.
        for child in &mut item.children {
            self.visit(child, diagnostics);
        }
    }
}
