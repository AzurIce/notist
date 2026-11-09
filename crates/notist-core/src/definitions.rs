//! Declarative content-function contracts, shared by native and source definitions.

use rowan::TextRange;
use std::collections::BTreeSet;

use crate::builtins::{Accepts, Level};
use crate::diag::{Diagnostic, Phase};
use crate::item::Value;

pub use crate::definition_identity::{DefId, FunctionId, ModuleId, ModuleRef};

/// A static file-level module dependency produced by either source frontend.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Import {
    pub path: String,
    pub kind: ImportKind,
    pub span: TextRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImportKind {
    Module { alias: String },
    Members(Vec<ImportMember>),
    Wildcard,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportMember {
    pub name: String,
    pub alias: String,
    pub span: TextRange,
}

/// Static name binding. Functions always retain their defining identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImportBinding {
    Module(ModuleId),
    Function(DefId),
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
    Path,
    /// An embed input: a bound resource Path or a URL string. No implicit
    /// String/Path conversion applies outside the embed boundary.
    ResourceTarget,
    /// None accepts any Value elements; Some checks elements recursively.
    Array(Option<Box<Type>>),
    /// Nominal optional data; identity requires equal payload types.
    Option(Box<Type>),
    /// Nominal fallible data; both success and error types are part of identity.
    Result(Box<Type>, Box<Type>),
    Dict,
    /// Uninhabited data: expressions of this type cannot return normally.
    Never,
    Data,
    DataError,
}

impl Type {
    /// Built-in, unparameterized type spelling at source-language boundaries.
    /// Container arguments and lexical type parameters are handled by callers.
    pub fn builtin(name: &str) -> Option<Self> {
        Some(match name {
            "Type" => Self::Type,
            "Never" => Self::Never,
            "Data" => Self::Data,
            "DataError" => Self::DataError,
            "InlineContent" => Self::InlineContent,
            "BlockContent" => Self::BlockContent,
            "Unit" => Self::Unit,
            "Bool" => Self::Bool,
            "Int" => Self::Int,
            "Float" => Self::Float,
            "String" => Self::String,
            "Path" => Self::Path,
            "ResourceTarget" => Self::ResourceTarget,
            "Dict" => Self::Dict,
            "Array" => Self::Array(None),
            _ => return None,
        })
    }

    pub fn accepts(&self, value: &Value) -> bool {
        match (self, value) {
            (Self::Unit, Value::Unit)
            | (Self::Bool, Value::Bool(_))
            | (Self::Int, Value::Int(_))
            | (Self::Float, Value::Float(_))
            | (Self::String, Value::Str(_))
            | (Self::Path, Value::Path(_))
            | (Self::ResourceTarget, Value::Path(_) | Value::Str(_))
            | (Self::Dict, Value::Dict(_))
            | (Self::Data, Value::Data(_))
            | (Self::DataError, Value::DataError(_)) => true,
            (Self::Array(element), Value::Array(values)) => {
                element.as_ref().is_none_or(|ty| values.accepts(ty))
            }
            (Self::Option(element), Value::Some(value)) => element.accepts(value),
            (Self::Option(_), Value::None) => true,
            (Self::Result(ok, _), Value::Ok(value)) => ok.accepts(value),
            (Self::Result(_, err), Value::Err(value)) => err.accepts(value),
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
            Self::Never => f.write_str("Never"),
            Self::Data => f.write_str("Data"),
            Self::DataError => f.write_str("DataError"),
            Self::Unit => f.write_str("Unit"),
            Self::Bool => f.write_str("boolean"),
            Self::Int => f.write_str("integer"),
            Self::Float => f.write_str("float"),
            Self::String => f.write_str("string"),
            Self::Path => f.write_str("Path"),
            Self::ResourceTarget => f.write_str("ResourceTarget"),
            Self::Dict => f.write_str("dict"),
            Self::Array(None) => f.write_str("array"),
            Self::Array(Some(element)) => write!(f, "array of {element}"),
            Self::Option(element) => write!(f, "option of {element}"),
            Self::Result(ok, err) => write!(f, "Result<{ok}, {err}>"),
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
            (Self::ArrayStrings(allowed), Value::Array(values)) => values.iter().all(
                |value| matches!(value, Value::Str(value) if allowed.iter().any(|s| s == value)),
            ),
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
                Value::Some(v) => format!("o{}", data(v)),
                Value::None => "o-".into(),
                Value::Ok(v) => format!("r+{}", data(v)),
                Value::Err(v) => format!("r-{}", data(v)),
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
/// A resolved invocation carries only its numeric declaration identity. Names
/// are owned by the enclosing Sources/Definitions snapshot, not each use site.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedInstance {
    pub function: DefId,
    pub arguments: Vec<ComptimeValue>,
}
impl ResolvedInstance {
    pub fn key(
        &self,
        names: &crate::definition_identity::Definitions,
    ) -> Result<String, crate::definition_identity::DefinitionError> {
        Ok(format!(
            "{}:{:?}",
            names.function(self.function)?,
            self.arguments
                .iter()
                .map(ComptimeValue::key)
                .collect::<Vec<_>>()
        ))
    }
}
impl From<FunctionInstance> for ResolvedInstance {
    fn from(instance: FunctionInstance) -> Self {
        Self {
            function: instance.function.def_id(),
            arguments: instance.arguments,
        }
    }
}
impl From<FunctionInstance> for crate::shared::Shared<ResolvedInstance> {
    fn from(instance: FunctionInstance) -> Self {
        crate::shared::Shared::new(instance.into())
    }
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
    /// Describe unannotated data. Absent Option/Result payload types use Unit
    /// placeholders; checked expressions and declared contracts retain the full
    /// nominal type and must not reconstruct it from this approximation.
    pub fn of(value: &Value) -> Self {
        match value {
            Value::Unit => Self::Unit,
            Value::Bool(_) => Self::Bool,
            Value::Int(_) => Self::Int,
            Value::Float(_) => Self::Float,
            Value::Str(_) => Self::String,
            Value::Path(_) => Self::Path,
            Value::Dict(_) => Self::Dict,
            Value::Data(_) => Self::Data,
            Value::DataError(_) => Self::DataError,
            Value::Array(v) => v.inferred_type(),
            Value::Some(v) => Self::Option(Box::new(Self::of(v))),
            // A bare none carries no payload type; checked expressions keep
            // their static Option<T> instead of deriving it here.
            Value::None => Self::Option(Box::new(Self::Unit)),
            // Like none, a variant alone cannot recover the absent payload
            // type. Checked expressions retain the complete nominal type.
            Value::Ok(v) => Self::Result(Box::new(Self::of(v)), Box::new(Self::Unit)),
            Value::Err(v) => Self::Result(Box::new(Self::Unit), Box::new(Self::of(v))),
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
            Self::Option(t) => t.concrete(),
            Self::Result(ok, err) => ok.concrete() && err.concrete(),
            _ => true,
        }
    }
    pub fn data(&self) -> bool {
        match self {
            Self::Type | Self::Parameter(_) | Self::InlineContent | Self::BlockContent => false,
            Self::Array(t) => t.as_deref().is_none_or(Self::data),
            Self::Option(t) => t.data(),
            Self::Result(ok, err) => ok.data() && err.data(),
            _ => true,
        }
    }
    pub fn runtime(&self) -> bool {
        match self {
            Self::Type | Self::Parameter(_) => false,
            Self::Array(t) => t.as_deref().is_none_or(Self::data),
            Self::Option(t) => t.data(),
            Self::Result(ok, err) => ok.data() && err.data(),
            _ => true,
        }
    }
}

/// Implementation origin is independent of the function's result type.
/// Code bodies remain in CodeProgram; native ids select trusted Rust code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FunctionImpl {
    Code,
    Native(NativeFunction),
}

impl Default for FunctionImpl {
    fn default() -> Self {
        Self::Native(NativeFunction::Construct)
    }
}

impl FunctionImpl {
    pub fn is_code(self) -> bool {
        self == Self::Code
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NativeFunction {
    /// Construct a semantic content node, including host-bound components.
    #[default]
    Construct,
    ReadText,
    Panic,
    Str,
    ArrayLen,
    PathJoin,
    Data(crate::data::DataFunction),
}
impl NativeFunction {
    pub fn reads_resources(self) -> bool {
        matches!(self, Self::ReadText) || matches!(self, Self::Data(f) if f.reads_resources())
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(try_from = "FunctionDefWire", into = "FunctionDefWire")
)]
pub struct FunctionDef {
    pub id: FunctionId,
    pub parameters: Vec<ParameterDef>,
    pub children: Accepts,
    pub returns: Type,
    pub comptime: Vec<ComptimeParameter>,
    /// Evaluated after comptime inference, in a scope containing only <> inputs.
    pub where_clause: Option<crate::expr::Expr>,
    pub children_type: Option<Type>,
    pub implementation: FunctionImpl,
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
            where_clause: None,
            children_type: match children {
                Accepts::Inline => Some(Type::InlineContent),
                Accepts::Nothing => None,
                _ => Some(Type::BlockContent),
            },
            implementation: FunctionImpl::Native(NativeFunction::Construct),
            span: TextRange::empty(0.into()),
        }
    }
}

// Preserve the previous wire representation for Code and content constructors.
// `code` is a compatibility projection, never a second internal source of truth.
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct FunctionDefWire<I = FunctionId> {
    pub(crate) id: I,
    pub(crate) parameters: Vec<ParameterDef>,
    pub(crate) children: Accepts,
    pub(crate) returns: Type,
    pub(crate) comptime: Vec<ComptimeParameter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) where_clause: Option<crate::expr::Expr>,
    pub(crate) children_type: Option<Type>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) implementation: Option<FunctionImpl>,
    #[serde(with = "crate::evaluation::span")]
    pub(crate) span: TextRange,
}

#[cfg(feature = "serde")]
impl From<FunctionDef> for FunctionDefWire {
    fn from(d: FunctionDef) -> Self {
        Self {
            id: d.id,
            parameters: d.parameters,
            children: d.children,
            returns: d.returns,
            comptime: d.comptime,
            where_clause: d.where_clause,
            children_type: d.children_type,
            code: Some(d.implementation.is_code()),
            implementation: matches!(
                d.implementation,
                FunctionImpl::Native(
                    NativeFunction::ReadText
                        | NativeFunction::Panic
                        | NativeFunction::Data(_)
                        | NativeFunction::Str
                        | NativeFunction::ArrayLen
                        | NativeFunction::PathJoin
                )
            )
            .then_some(d.implementation),
            span: d.span,
        }
    }
}

#[cfg(feature = "serde")]
impl TryFrom<FunctionDefWire> for FunctionDef {
    type Error = &'static str;
    fn try_from(d: FunctionDefWire) -> Result<Self, Self::Error> {
        let implementation = d.implementation.unwrap_or_else(|| {
            if d.code == Some(true) {
                FunctionImpl::Code
            } else {
                FunctionImpl::default()
            }
        });
        if d.code.is_some_and(|code| code != implementation.is_code()) {
            return Err("conflicting function implementation and legacy code flag");
        }
        Ok(Self {
            id: d.id,
            parameters: d.parameters,
            children: d.children,
            returns: d.returns,
            comptime: d.comptime,
            where_clause: d.where_clause,
            children_type: d.children_type,
            implementation,
            span: d.span,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DefinitionModule {
    pub module: ModuleRef,
    pub functions: Vec<FunctionDef>,
    pub span: TextRange,
}

impl DefinitionModule {
    pub fn new(module: impl Into<String>) -> Self {
        Self {
            module: ModuleRef::new(module.into()),
            functions: Vec::new(),
            span: TextRange::empty(0.into()),
        }
    }
}

/// Shared validation; definitions are checked before any registry mutation.
pub fn validate_module(module: &DefinitionModule) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut report = |span, message| diagnostics.push(Diagnostic::new(Phase::Type, span, message));
    if module.module.is_empty() {
        report(module.span, "module identity cannot be empty".to_string());
    }
    let mut names = BTreeSet::new();
    for function in &module.functions {
        if let Some(condition) = &function.where_clause {
            let names = function.comptime.iter().map(|p| p.name.clone()).collect();
            validate_where(condition, &names, &mut report);
        }
        let reserved = match function.implementation {
            FunctionImpl::Native(NativeFunction::Str) => Some(("notist", "str")),
            FunctionImpl::Native(NativeFunction::ArrayLen) => Some(("notist.array", "len")),
            FunctionImpl::Native(NativeFunction::PathJoin) => Some(("notist.path", "join")),
            _ => None,
        };
        if reserved
            .is_some_and(|(module, name)| function.id.module != module || function.id.name != name)
        {
            report(
                function.span,
                "scalar and array implementations are reserved for the standard library".into(),
            );
        }
        if let FunctionImpl::Native(NativeFunction::Data(kind)) = function.implementation
            && (function.id.module != "notist" || function.id.name != kind.name())
        {
            report(
                function.span,
                "data implementations are reserved for the standard library".into(),
            );
        }
        if function.implementation == FunctionImpl::Native(NativeFunction::ReadText)
            && (function.id.module != "notist" || function.id.name != "read_text")
        {
            report(
                function.span,
                "read_text implementation is reserved for the standard library".into(),
            );
        }
        if function.implementation == FunctionImpl::Native(NativeFunction::Panic)
            && (function.id.module != "notist" || function.id.name != "panic")
        {
            report(
                function.span,
                "panic implementation is reserved for the standard library".into(),
            );
        }
        if function.id.module_id() != module.module.id() {
            report(
                function.span,
                format!("`{}` belongs to a different module", function.id),
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
            if let Some(default) = &parameter.default
                && match default {
                    ComptimeValue::Type(ty) => parameter.ty != Type::Type || !ty.concrete(),
                    ComptimeValue::Data(v) => {
                        v.contains_data_views()
                            || parameter.ty.concrete() && !parameter.ty.accepts(v)
                            || !parameter.constraint.accepts(v)
                    }
                }
            {
                report(
                    parameter.span,
                    format!("invalid comptime default for `{}`", parameter.name),
                );
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
                Type::Option(t) => match t.as_ref() {
                    Type::InlineContent | Type::BlockContent => false,
                    _ => check(t, names),
                },
                Type::Result(ok, err) => [ok, err].iter().all(|t| {
                    !matches!(t.as_ref(), Type::InlineContent | Type::BlockContent)
                        && check(t, names)
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
                Type::Option(t) => data(t),
                Type::Result(ok, err) => data(ok) && data(err),
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

/// Inspect all branches, including ones a concrete instance will not execute.
/// A where clause cannot acquire ordinary inputs or module/import bindings.
fn validate_where(
    expr: &crate::expr::Expr,
    names: &BTreeSet<String>,
    report: &mut impl FnMut(TextRange, String),
) {
    use crate::expr::Expr;
    match expr {
        Expr::Ref { name, span } => {
            if !names.contains(name)
                && Type::builtin(name).is_none()
                && !matches!(name.as_str(), "Option" | "Result" | "_")
            {
                report(
                    *span,
                    format!(
                        "where can only reference <> parameters and built-in types; `{name}` is not available"
                    ),
                );
            }
        }
        Expr::Literal(..) | Expr::PathLiteral { .. } => {}
        Expr::Member { base, .. } => validate_where(base, names, report),
        Expr::Binary { left, right, .. } => {
            validate_where(left, names, report);
            validate_where(right, names, report);
        }
        Expr::Unary { value, .. } | Expr::Comptime { value, .. } => {
            validate_where(value, names, report)
        }
        Expr::If {
            condition, yes, no, ..
        } => {
            validate_where(condition, names, report);
            validate_where(yes, names, report);
            validate_where(no, names, report);
        }
        Expr::Match { subject, arms, .. } => {
            validate_where(subject, names, report);
            for (pattern, value) in arms {
                validate_where(pattern, names, report);
                validate_where(value, names, report);
            }
        }
        Expr::Block {
            bindings, value, ..
        } => {
            let mut locals = names.clone();
            for (name, value) in bindings {
                validate_where(value, &locals, report);
                locals.insert(name.clone());
            }
            validate_where(value, &locals, report);
        }
        Expr::For {
            binding,
            subject,
            body,
            ..
        } => {
            validate_where(subject, names, report);
            let mut locals = names.clone();
            locals.insert(binding.clone());
            validate_where(body, &locals, report);
        }
        Expr::Array(values, _) => {
            for value in values {
                validate_where(value, names, report);
            }
        }
        Expr::Dict(values, _) => {
            for (_, value) in values {
                validate_where(value, names, report);
            }
        }
        Expr::Call {
            callee,
            args,
            comptime,
            children,
            ..
        } => {
            validate_where(callee, names, report);
            for arg in args.iter().chain(comptime) {
                validate_where(arg.value(), names, report);
            }
            for child in children {
                validate_where(child, names, report);
            }
        }
        Expr::Content { .. } | Expr::Fragment { .. } => report(
            expr.span(),
            "where requires a comptime Bool, not content".into(),
        ),
    }
}

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}
