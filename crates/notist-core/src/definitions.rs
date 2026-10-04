//! Declarative content-function contracts, shared by native and source definitions.

use rowan::TextRange;
use std::collections::BTreeSet;

use crate::builtins::{Accepts, Level};
use crate::diag::{Diagnostic, Phase};
use crate::expr::BodyFlavor;
use crate::item::{Dict, Value};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FunctionId {
    pub package: String,
    pub name: String,
}

impl FunctionId {
    pub fn new(package: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            package: package.into(),
            name: name.into(),
        }
    }
}

impl std::fmt::Display for FunctionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}", self.package, self.name)
    }
}

/// Value-only types: Content can never occur inside an argument type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    Unit,
    Bool,
    Int,
    Float,
    String,
    /// None accepts any Value elements; Some checks elements recursively.
    Array(Option<Box<ValueType>>),
    Dict,
}

impl ValueType {
    pub fn accepts(&self, value: &Value) -> bool {
        match (self, value) {
            (Self::Unit, Value::Unit)
            | (Self::Bool, Value::Bool(_))
            | (Self::Int, Value::Int(_))
            | (Self::Float, Value::Float(_))
            | (Self::String, Value::Str(_))
            | (Self::Dict, Value::Dict(_)) => true,
            (Self::Array(element), Value::Array(values)) => element
                .as_ref()
                .is_none_or(|ty| values.iter().all(|value| ty.accepts(value))),
            _ => false,
        }
    }
}

impl std::fmt::Display for ValueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unit => f.write_str("unit"),
            Self::Bool => f.write_str("boolean"),
            Self::Int => f.write_str("integer"),
            Self::Float => f.write_str("float"),
            Self::String => f.write_str("string"),
            Self::Dict => f.write_str("dict"),
            Self::Array(None) => f.write_str("array"),
            Self::Array(Some(element)) => write!(f, "array of {element}"),
        }
    }
}

/// Data-only value constraints. Source syntax for advanced constraints is deferred.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ValueConstraint {
    #[default]
    Any,
    PositiveInt,
    ArrayStrings(Vec<String>),
}

impl ValueConstraint {
    fn accepts(&self, value: &Value) -> bool {
        match (self, value) {
            (Self::Any, _) => true,
            (Self::PositiveInt, Value::Int(value)) => *value > 0,
            (Self::ArrayStrings(allowed), Value::Array(values)) => values
                .iter()
                .all(|value| matches!(value, Value::Str(value) if allowed.contains(value))),
            _ => false,
        }
    }

    fn compatible(&self, ty: &ValueType) -> bool {
        match (self, ty) {
            (Self::Any, _) | (Self::PositiveInt, ValueType::Int) => true,
            (Self::ArrayStrings(_), ValueType::Array(element)) => {
                element.as_deref().is_none_or(|ty| *ty == ValueType::String)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParameterMode {
    Required,
    Optional,
    Default(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParameterDef {
    pub name: String,
    pub ty: ValueType,
    pub mode: ParameterMode,
    pub positional: bool,
    pub constraint: ValueConstraint,
    /// A domain-specific description for diagnostics, otherwise derived from ty.
    pub expectation: Option<String>,
    pub span: TextRange,
}

impl ParameterDef {
    pub fn new(name: impl Into<String>, ty: ValueType, mode: ParameterMode) -> Self {
        Self {
            name: name.into(),
            ty,
            mode,
            positional: true,
            constraint: ValueConstraint::Any,
            expectation: None,
            span: TextRange::empty(0.into()),
        }
    }

    fn accepts(&self, value: &Value) -> bool {
        self.ty.accepts(value) && self.constraint.accepts(value)
    }

    fn expected(&self) -> String {
        self.expectation
            .clone()
            .unwrap_or_else(|| self.ty.to_string())
    }
}

/// Fixed or declaratively selected content levels, without evaluation callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnRule {
    Fixed(Level),
    /// The transparent group inherits the written content mount's flavor.
    Inherit,
    /// Select a block result only when the named boolean argument is true.
    BlockIfTrue(String),
}

impl ReturnRule {
    pub fn base_level(&self) -> Level {
        match self {
            Self::Fixed(level) => *level,
            Self::Inherit => Level::Inherit,
            Self::BlockIfTrue(_) => Level::Inline,
        }
    }

    pub fn level(&self, fields: &Dict, body: BodyFlavor) -> Level {
        match self {
            Self::Fixed(level) => *level,
            Self::Inherit => {
                if body == BodyFlavor::Block {
                    Level::Block
                } else {
                    Level::Inline
                }
            }
            Self::BlockIfTrue(field) => {
                if fields.get(field) == Some(&Value::Bool(true)) {
                    Level::Block
                } else {
                    Level::Inline
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDef {
    pub id: FunctionId,
    pub parameters: Vec<ParameterDef>,
    pub children: Accepts,
    pub returns: ReturnRule,
    pub span: TextRange,
}

impl FunctionDef {
    pub fn new(id: FunctionId, children: Accepts, returns: ReturnRule) -> Self {
        Self {
            id,
            parameters: Vec::new(),
            children,
            returns,
            span: TextRange::empty(0.into()),
        }
    }

    /// Normalize and check call fields, preserving invalid and extra named values.
    /// Both definition sources use this binder. Explicit values are never replaced
    /// by defaults; positional arguments retain the existing last-value policy.
    pub fn bind_fields(
        &self,
        positional: impl IntoIterator<Item = Value>,
        mut fields: Dict,
        call_name: &str,
        span: TextRange,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Dict {
        let parameters: Vec<_> = self.parameters.iter().filter(|p| p.positional).collect();
        let mut extra = false;
        for (index, value) in positional.into_iter().enumerate() {
            if let Some(parameter) = parameters.get(index) {
                fields.insert(&parameter.name, value);
            } else {
                extra = true;
            }
        }
        if extra {
            diagnostics.push(Diagnostic::new(
                Phase::Type,
                span,
                format!("too many positional arguments for `{call_name}`"),
            ));
        }
        // Validate in signature order, then insert defaults to preserve field order.
        for parameter in &self.parameters {
            match fields.get(&parameter.name) {
                None if parameter.mode == ParameterMode::Required => {
                    diagnostics.push(Diagnostic::new(
                        Phase::Type,
                        span,
                        format!("`{call_name}` requires `{}`", parameter.name),
                    ))
                }
                Some(value) if !parameter.accepts(value) => diagnostics.push(Diagnostic::new(
                    Phase::Type,
                    span,
                    format!(
                        "`{call_name}.{}` must be {}",
                        parameter.name,
                        parameter.expected()
                    ),
                )),
                _ => {}
            }
        }
        for parameter in &self.parameters {
            if fields.get(&parameter.name).is_none()
                && let ParameterMode::Default(value) = &parameter.mode
            {
                fields.insert(&parameter.name, value.clone());
            }
        }
        fields
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DefinitionModule {
    pub package: String,
    pub functions: Vec<FunctionDef>,
    pub span: TextRange,
}

impl DefinitionModule {
    pub fn new(package: impl Into<String>) -> Self {
        Self {
            package: package.into(),
            functions: Vec::new(),
            span: TextRange::empty(0.into()),
        }
    }
}

/// Shared validation; definitions are checked before any registry mutation.
pub fn validate_module(module: &DefinitionModule) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut report = |span, message| diagnostics.push(Diagnostic::new(Phase::Type, span, message));
    if !valid_name(&module.package) {
        report(module.span, "invalid package name".to_string());
    }
    let mut names = BTreeSet::new();
    for function in &module.functions {
        if function.id.package != module.package {
            report(
                function.span,
                format!("`{}` belongs to a different package", function.id),
            );
        }
        if !valid_name(&function.id.name) {
            report(function.span, "invalid function name".to_string());
        }
        if !names.insert(&function.id.name) {
            report(
                function.span,
                format!("duplicate function `{}`", function.id.name),
            );
        }
        let mut parameters = BTreeSet::new();
        for parameter in &function.parameters {
            if !valid_name(&parameter.name) {
                report(parameter.span, "invalid parameter name".to_string());
            }
            if !parameters.insert(&parameter.name) {
                report(
                    parameter.span,
                    format!("duplicate parameter `{}`", parameter.name),
                );
            }
            if !parameter.constraint.compatible(&parameter.ty) {
                report(
                    parameter.span,
                    format!("constraint is incompatible with `{}`", parameter.name),
                );
            }
            if let ParameterMode::Default(value) = &parameter.mode
                && !parameter.accepts(value)
            {
                report(
                    parameter.span,
                    format!(
                        "default for `{}` must be {}",
                        parameter.name,
                        parameter.expected()
                    ),
                );
            }
        }
        match &function.returns {
            ReturnRule::Fixed(Level::Inherit) => report(
                function.span,
                "use ReturnRule::Inherit for a transparent result".to_string(),
            ),
            ReturnRule::Inherit if function.children != Accepts::Any => report(
                function.span,
                "an inherited result requires a transparent children contract".to_string(),
            ),
            ReturnRule::BlockIfTrue(field)
                if !function
                    .parameters
                    .iter()
                    .any(|p| p.name == *field && p.ty == ValueType::Bool) =>
            {
                report(
                    function.span,
                    format!("dynamic result references a missing boolean parameter `{field}`"),
                )
            }
            _ => {}
        }
    }
    diagnostics
}

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}
