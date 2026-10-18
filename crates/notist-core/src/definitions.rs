//! Declarative content-function contracts, shared by native and source definitions.

use rowan::TextRange;
use std::collections::BTreeSet;

use crate::builtins::{Accepts, Level};
use crate::diag::{Diagnostic, Phase};
use crate::item::Value;

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

/// Type descriptions. `Parameter` occurs only in signature templates.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Type {
    Type,
    InlineContent,
    BlockContent,
    Parameter(String),
    Unit,
    Bool,
    Int,
    Float,
    String,
    /// None accepts any Value elements; Some checks elements recursively.
    Array(Option<Box<Type>>),
    Dict,
}

impl Type {
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

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Type => f.write_str("Type"),
            Self::InlineContent => f.write_str("InlineContent"),
            Self::BlockContent => f.write_str("BlockContent"),
            Self::Parameter(name) => f.write_str(name),
            Self::Unit => f.write_str("Unit"),
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ValueConstraint {
    #[default]
    Any,
    PositiveInt,
    ArrayStrings(Vec<String>),
}

impl ValueConstraint {
    pub fn accepts(&self, value: &Value) -> bool {
        match (self, value) {
            (Self::Any, _) => true,
            (Self::PositiveInt, Value::Int(value)) => *value > 0,
            (Self::ArrayStrings(allowed), Value::Array(values)) => values
                .iter()
                .all(|value| matches!(value, Value::Str(value) if allowed.contains(value))),
            _ => false,
        }
    }

    fn compatible(&self, ty: &Type) -> bool {
        match (self, ty) {
            (Self::Any, _) | (Self::PositiveInt, Type::Int) => true,
            (Self::ArrayStrings(_), Type::Array(element)) => {
                element.as_deref().is_none_or(|ty| *ty == Type::String)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ParameterMode {
    Required,
    Optional,
    Default(Value),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParameterDef {
    pub name: String,
    pub ty: Type,
    pub mode: ParameterMode,
    pub positional: bool,
    pub constraint: ValueConstraint,
    /// A domain-specific description for diagnostics, otherwise derived from ty.
    pub expectation: Option<String>,
    #[cfg_attr(feature = "serde", serde(with = "crate::evaluation::span"))]
    pub span: TextRange,
}

impl ParameterDef {
    pub fn new(name: impl Into<String>, ty: Type, mode: ParameterMode) -> Self {
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

    pub fn accepts(&self, value: &Value) -> bool {
        self.ty.accepts(value) && self.constraint.accepts(value)
    }

    pub fn expected(&self) -> String {
        self.expectation
            .clone()
            .unwrap_or_else(|| self.ty.to_string())
    }
}

/// Values carried by explicit or inferred compile-time arguments.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ComptimeValue {
    Type(Type),
    Data(Value),
}

impl ComptimeValue {
    pub fn ty(&self) -> Type {
        match self {
            Self::Type(_) => Type::Type,
            Self::Data(v) => Type::of(v),
        }
    }
    /// Canonical cache key; floating point values retain their exact bits.
    pub fn key(&self) -> String {
        fn data(v: &Value) -> String {
            match v {
                Value::Float(v) => format!("f{:016x}", v.to_bits()),
                Value::Array(v) => format!("a{:?}", v.iter().map(data).collect::<Vec<_>>()),
                Value::Dict(v) => format!(
                    "d{:?}",
                    v.iter().map(|(k, v)| (k, data(v))).collect::<Vec<_>>()
                ),
                v => format!("{v:?}"),
            }
        }
        match self {
            Self::Type(t) => format!("t{t:?}"),
            Self::Data(v) => data(v),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ComptimeParameter {
    pub constraint: ValueConstraint,
    pub name: String,
    pub ty: Type,
    pub default: Option<ComptimeValue>,
    #[cfg_attr(feature = "serde", serde(with = "crate::evaluation::span"))]
    pub span: TextRange,
}

/// Stable instance identity, independent of any analysis session's cache indices.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FunctionInstance {
    pub function: FunctionId,
    pub arguments: Vec<ComptimeValue>,
}
impl FunctionInstance {
    pub fn key(&self) -> String {
        format!(
            "{}:{:?}",
            self.function,
            self.arguments
                .iter()
                .map(ComptimeValue::key)
                .collect::<Vec<_>>()
        )
    }
}

impl Type {
    pub fn of(value: &Value) -> Self {
        match value {
            Value::Unit => Self::Unit,
            Value::Bool(_) => Self::Bool,
            Value::Int(_) => Self::Int,
            Value::Float(_) => Self::Float,
            Value::Str(_) => Self::String,
            Value::Dict(_) => Self::Dict,
            Value::Array(v) => {
                let first = v.first().map(Self::of);
                Self::Array(
                    first
                        .filter(|t| v.iter().all(|v| Self::of(v) == *t))
                        .map(Box::new),
                )
            }
        }
    }
    pub fn level(&self) -> Option<Level> {
        match self {
            Self::InlineContent => Some(Level::Inline),
            Self::BlockContent => Some(Level::Block),
            _ => None,
        }
    }
    pub fn concrete(&self) -> bool {
        match self {
            Self::Parameter(_) => false,
            Self::Array(t) => t.as_deref().is_none_or(Self::concrete),
            _ => true,
        }
    }
    pub fn data(&self) -> bool {
        match self {
            Self::Type | Self::Parameter(_) | Self::InlineContent | Self::BlockContent => false,
            Self::Array(t) => t.as_deref().is_none_or(Self::data),
            _ => true,
        }
    }
    pub fn runtime(&self) -> bool {
        match self {
            Self::Type | Self::Parameter(_) => false,
            Self::Array(t) => t.as_deref().is_none_or(Self::data),
            _ => true,
        }
    }
    pub fn substitute(&self, bindings: &std::collections::BTreeMap<String, ComptimeValue>) -> Self {
        match self {
            Self::Parameter(name) => match bindings.get(name) {
                Some(ComptimeValue::Type(ty)) => ty.clone(),
                _ => self.clone(),
            },
            Self::Array(t) => Self::Array(t.as_ref().map(|t| Box::new(t.substitute(bindings)))),
            _ => self.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FunctionDef {
    pub id: FunctionId,
    pub parameters: Vec<ParameterDef>,
    pub children: Accepts,
    pub returns: Type,
    pub comptime: Vec<ComptimeParameter>,
    pub children_type: Option<Type>,
    /// Body expressions live in CodeProgram, keyed by this function's identity.
    pub code: bool,
    #[cfg_attr(feature = "serde", serde(with = "crate::evaluation::span"))]
    pub span: TextRange,
}

impl FunctionDef {
    pub fn new(id: FunctionId, children: Accepts, returns: Type) -> Self {
        Self {
            id,
            parameters: Vec::new(),
            children,
            returns,
            comptime: Vec::new(),
            children_type: match children {
                Accepts::Inline => Some(Type::InlineContent),
                Accepts::Nothing => None,
                _ => Some(Type::BlockContent),
            },
            code: false,
            span: TextRange::empty(0.into()),
        }
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
        if function.children_type.is_some() {
            parameters.insert("children".to_string());
        }
        for parameter in &function.parameters {
            if !valid_name(&parameter.name) {
                report(parameter.span, "invalid parameter name".to_string());
            }
            if !parameters.insert(parameter.name.clone()) {
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
                && (!parameter.constraint.accepts(value)
                    || parameter.ty.concrete() && !parameter.ty.accepts(value))
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
        let mut ct_names = BTreeSet::new();
        for parameter in &function.comptime {
            if !valid_name(&parameter.name) {
                report(parameter.span, "invalid comptime parameter name".into());
            }
            if !ct_names.insert(&parameter.name) || parameters.contains(&parameter.name) {
                report(
                    parameter.span,
                    format!("duplicate parameter `{}`", parameter.name),
                );
            }
            if (parameter.ty != Type::Type && !data(&parameter.ty))
                || !parameter.constraint.compatible(&parameter.ty)
            {
                report(
                    parameter.span,
                    "comptime parameters require Type or data types".into(),
                );
            }
            if let Some(default) = &parameter.default {
                if match default {
                    ComptimeValue::Type(_) => parameter.ty != Type::Type,
                    ComptimeValue::Data(v) => {
                        parameter.ty.concrete() && !parameter.ty.accepts(v)
                            || !parameter.constraint.accepts(v)
                    }
                } {
                    report(
                        parameter.span,
                        format!("invalid comptime default for `{}`", parameter.name),
                    );
                }
            }
        }
        fn check(ty: &Type, names: &BTreeSet<&String>) -> bool {
            match ty {
                Type::Type => false,
                Type::Parameter(name) => names.contains(name),
                Type::Array(t) => t.as_deref().is_none_or(|t| match t {
                    Type::InlineContent | Type::BlockContent => false,
                    _ => check(t, names),
                }),
                _ => true,
            }
        }
        let types = function
            .comptime
            .iter()
            .filter(|p| p.ty == Type::Type)
            .map(|p| &p.name)
            .collect();
        fn data(t: &Type) -> bool {
            match t {
                Type::Parameter(_) => true,
                Type::Array(t) => t.as_deref().is_none_or(data),
                _ => t.data(),
            }
        }
        if !check(&function.returns, &types)
            || function
                .parameters
                .iter()
                .any(|p| !check(&p.ty, &types) || !data(&p.ty))
            || function.children_type.as_ref().is_some_and(|t| {
                !check(t, &types) || t.level().is_none() && !matches!(t, Type::Parameter(_))
            })
        {
            report(
                function.span,
                "runtime signatures require concrete runtime types or Type parameters".into(),
            );
        }
    }
    diagnostics
}

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}
