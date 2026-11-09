//! Shared checked expression IR and specialization contracts.
//! These are pure IR data; checking and interpretation belong to the pipeline.

use crate::{
    binding::BindingId,
    definitions::{ComptimeValue, FunctionDef, ResolvedInstance, Type},
    item::{Dict, Item},
};
use rowan::TextRange;
use std::{collections::BTreeMap, sync::Arc};

/// One checked specialization shared by analysis and evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceRecord {
    definition: FunctionDef,
    native_inputs: Vec<(NativeParameter, usize)>,
    pub inputs: FunctionInputs,
    pub body: Option<TypedExpr>,
    pub diagnostics: Vec<crate::diag::Diagnostic>,
    pub checking: bool,
}

/// Implementation-local parameter roles, bound once when the record is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeParameter {
    Value,
    SelfValue,
    Source,
    Text,
    Data,
    Object,
    Array,
    Key,
    Index,
    Error,
    Message,
    Relative,
}
impl NativeParameter {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "value" => Self::Value,
            "self" => Self::SelfValue,
            "source" => Self::Source,
            "text" => Self::Text,
            "data" => Self::Data,
            "object" => Self::Object,
            "array" => Self::Array,
            "key" => Self::Key,
            "index" => Self::Index,
            "error" => Self::Error,
            "message" => Self::Message,
            "relative" => Self::Relative,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FunctionInputs {
    pub parameters: Box<[Option<BindingId>]>,
    pub children: Option<BindingId>,
}
impl InstanceRecord {
    pub fn new(definition: FunctionDef) -> Self {
        let native_inputs = if matches!(definition.implementation,
            crate::definitions::FunctionImpl::Native(native) if native != crate::definitions::NativeFunction::Construct)
        {
            definition
                .parameters
                .iter()
                .enumerate()
                .filter_map(|(index, p)| {
                    NativeParameter::from_name(&p.name).map(|role| (role, index))
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            definition,
            native_inputs,
            inputs: FunctionInputs::default(),
            body: None,
            diagnostics: Vec::new(),
            checking: false,
        }
    }
    pub fn definition(&self) -> &FunctionDef {
        &self.definition
    }
    pub fn with_definition(self, definition: FunctionDef) -> Self {
        let bindings: BTreeMap<_, _> = self
            .named_inputs()
            .map(|(name, id)| (name.to_owned(), id))
            .collect();
        let mut record = Self::new(definition);
        record.inputs = FunctionInputs {
            parameters: record
                .definition
                .parameters
                .iter()
                .map(|p| bindings.get(p.name.as_str()).copied())
                .collect(),
            children: bindings.get("children").copied(),
        };
        record.body = self.body;
        record.diagnostics = self.diagnostics;
        record.checking = self.checking;
        record
    }
    pub fn native_parameter(&self, role: NativeParameter) -> Option<usize> {
        self.native_inputs
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, index)| *index)
    }
    /// Named projection is for diagnostics, source validation and explicit wire environments.
    pub fn named_inputs(&self) -> impl Iterator<Item = (&str, BindingId)> {
        self.definition
            .parameters
            .iter()
            .zip(self.inputs.parameters.iter())
            .filter_map(|(p, id)| id.map(|id| (p.name.as_str(), id)))
            .chain(self.inputs.children.map(|id| ("children", id)))
    }
}

/// Checked instances and retained data backing evidence, independent of the
/// compiler and evaluator implementations. Inserting instances uses COW.
#[derive(Debug, Clone, Default)]
pub struct CheckedProgram {
    pub instances: Arc<BTreeMap<String, Arc<InstanceRecord>>>,
    pub data_retention: crate::data::DataRetention,
    pub sources: crate::source::Sources,
}

impl PartialEq for CheckedProgram {
    fn eq(&self, other: &Self) -> bool {
        self.instances == other.instances
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedExpr {
    pub ty: Option<Type>,
    pub known: Option<ComptimeValue>,
    pub kind: ExprKind,
    pub span: TextRange,
    pub source: crate::source::SourceId,
    /// Local checked forest evidence, not a public function return contract.
    pub shape: ContentShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ContentShape {
    Empty,
    Rows,
    Cells,
    Items,
    #[default]
    Other,
}
impl ContentShape {
    pub fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Empty, s) | (s, Self::Empty) => s,
            (a, b) if a == b => a,
            _ => Self::Other,
        }
    }
    pub fn of_items(items: &[Item]) -> Self {
        use crate::item::Ctor;
        items.iter().fold(Self::Empty, |shape, item| {
            shape.join(match item.ctor() {
                Ctor::TableRow => Self::Rows,
                Ctor::TableCell => Self::Cells,
                Ctor::ListItem => Self::Items,
                Ctor::Node | Ctor::Group => Self::of_items(item.children()),
                Ctor::Interpolation => item.as_expr().map(|expr| expr.shape).unwrap_or(Self::Other),
                _ => Self::Other,
            })
        })
    }
    pub fn of_expr(kind: &ExprKind) -> Self {
        match kind {
            ExprKind::Call(call)
                if call.definition().implementation
                    == crate::definitions::FunctionImpl::Native(
                        crate::definitions::NativeFunction::Construct,
                    )
                    && call.definition().id.module == "notist" =>
            {
                match call.definition().id.name.as_str() {
                    "row" => Self::Rows,
                    "cell" => Self::Cells,
                    "item" => Self::Items,
                    "node" => call
                        .children
                        .as_ref()
                        .map(|e| e.shape)
                        .unwrap_or(Self::Empty),
                    _ => Self::Other,
                }
            }
            ExprKind::Content(items) => Self::of_items(items),
            ExprKind::Block(_, value) => value.shape,
            ExprKind::For { body, .. } => body.shape,
            ExprKind::If(_, yes, no) => {
                if yes.ty == Some(Type::Never) {
                    no.shape
                } else if no.ty == Some(Type::Never) {
                    yes.shape
                } else {
                    yes.shape.join(no.shape)
                }
            }
            ExprKind::MatchResult { arms, .. } => arms
                .iter()
                .filter(|arm| arm.body.ty != Some(Type::Never))
                .fold(Self::Empty, |shape, arm| shape.join(arm.body.shape)),
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Constant(ComptimeValue),
    Input(BindingId),
    Array(Vec<TypedExpr>),
    Dict(Vec<(String, TypedExpr)>),
    Unary(String, Box<TypedExpr>),
    Binary(String, Box<TypedExpr>, Box<TypedExpr>),
    Block(Vec<(BindingId, TypedExpr)>, Box<TypedExpr>),
    If(Box<TypedExpr>, Box<TypedExpr>, Box<TypedExpr>),
    /// The `some` constructor: wraps its data payload as Value::Some.
    Some(Box<TypedExpr>),
    /// Payload of a matched Value::Some; evaluation fails on none.
    Unwrap(Box<TypedExpr>),
    Ok(Box<TypedExpr>),
    Err(Box<TypedExpr>),
    /// Evaluate the subject once, then bind only the selected variant payload.
    MatchResult {
        subject: Box<TypedExpr>,
        arms: Vec<ResultArm>,
    },
    /// Function application, independent of whether its result is data or content.
    Call(crate::shared::Shared<Invocation>),
    Content(crate::shared::Shared<Vec<Item>>),
    For {
        binding: BindingId,
        subject: crate::shared::Shared<TypedExpr>,
        body: crate::shared::Shared<TypedExpr>,
    },
    Failure(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultArm {
    pub pattern: ResultPattern,
    pub body: TypedExpr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResultPattern {
    Ok(BindingId),
    Err(BindingId),
    Wildcard,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub instance: ResolvedInstance,
    /// The call's contract snapshot. Captures share it; editing one invocation
    /// detaches its copy instead of changing another call's contract.
    definition: crate::shared::Shared<FunctionDef>,
    arguments: Vec<CheckedArgument>,
    /// Checked children expression, for both data and content calls.
    pub children: Option<Box<TypedExpr>>,
    pub attrs: Dict,
    pub span: TextRange,
    pub source: crate::source::SourceId,
    pub site: CallSite,
    /// Captured only for resource-target inputs, including deferred URL strings.
    pub resource_root: Option<String>,
    pub invalid: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArgumentTarget {
    Parameter(usize),
    Extra(crate::shared::Text),
}
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedArgument {
    pub target: ArgumentTarget,
    pub expression: TypedExpr,
}
impl Invocation {
    /// Bind names without sorting or deduplicating the caller's evaluation sequence.
    pub fn new(
        instance: ResolvedInstance,
        definition: FunctionDef,
        arguments: Vec<(crate::shared::Text, TypedExpr)>,
    ) -> Self {
        let mut call = Self {
            instance,
            definition: definition.into(),
            arguments: Vec::new(),
            children: None,
            attrs: Dict::default(),
            span: TextRange::default(),
            source: Default::default(),
            site: CallSite::default(),
            resource_root: None,
            invalid: None,
        };
        call.set_arguments(arguments);
        call
    }
    pub fn definition(&self) -> &FunctionDef {
        &self.definition
    }
    pub fn checked_arguments(&self) -> &[CheckedArgument] {
        &self.arguments
    }
    /// Resolve a target against this declaration; external positions may be invalid.
    pub fn argument_name<'a>(
        &'a self,
        target: &'a ArgumentTarget,
    ) -> Option<&'a crate::shared::Text> {
        match target {
            ArgumentTarget::Parameter(index) => {
                self.definition.parameters.get(*index).map(|p| &p.name)
            }
            ArgumentTarget::Extra(name) => Some(name),
        }
    }
    pub fn arguments(&self) -> impl ExactSizeIterator<Item = (&str, &TypedExpr)> {
        self.arguments.iter().map(|a| {
            (
                self.argument_name(&a.target)
                    .expect("invocation-bound target")
                    .as_str(),
                &a.expression,
            )
        })
    }
    pub fn set_arguments(&mut self, arguments: Vec<(crate::shared::Text, TypedExpr)>) {
        self.arguments = arguments
            .into_iter()
            .map(|(name, expression)| {
                let target = self
                    .definition
                    .parameters
                    .iter()
                    .position(|p| p.name == name)
                    .map(ArgumentTarget::Parameter)
                    .unwrap_or(ArgumentTarget::Extra(name));
                CheckedArgument { target, expression }
            })
            .collect();
    }
    /// Editing the declaration rebuilds bindings; there is no independently mutable layout.
    pub fn with_definition(self, definition: FunctionDef) -> Self {
        let arguments = self
            .arguments
            .iter()
            .map(|a| {
                (
                    self.argument_name(&a.target)
                        .expect("invocation-bound target")
                        .clone(),
                    a.expression.clone(),
                )
            })
            .collect();
        let mut rebound = Self::new(self.instance.clone(), definition, arguments);
        rebound.children = self.children;
        rebound.attrs = self.attrs;
        rebound.span = self.span;
        rebound.source = self.source;
        rebound.site = self.site;
        rebound.resource_root = self.resource_root;
        rebound.invalid = self.invalid;
        rebound
    }
}

/// Lexical call context, independent of file names, aliases or module labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CallSite {
    #[default]
    Markup,
    Code,
}
impl CallSite {
    pub fn is_markup(&self) -> bool {
        *self == Self::Markup
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        definitions::{FunctionId, ParameterDef, ParameterMode},
        item::Value,
    };

    fn expression(n: i64) -> TypedExpr {
        TypedExpr {
            ty: Some(Type::Int),
            known: Some(ComptimeValue::Data(Value::Int(n))),
            kind: ExprKind::Constant(ComptimeValue::Data(Value::Int(n))),
            span: Default::default(),
            source: Default::default(),
            shape: Default::default(),
        }
    }

    #[test]
    fn argument_binding_preserves_order_duplicates_and_extra_names_when_rebound() {
        let mut definition = FunctionDef::new(
            FunctionId::new("checked-arguments", "call"),
            crate::builtins::Accepts::Nothing,
            Type::Int,
        );
        for name in ["z", "a"] {
            definition
                .parameters
                .push(ParameterDef::new(name, Type::Int, ParameterMode::Required));
        }
        let instance = ResolvedInstance {
            function: definition.id.def_id(),
            arguments: Vec::new(),
        };
        let call = Invocation::new(
            instance,
            definition.clone(),
            ["z", "a", "extra", "z"]
                .into_iter()
                .enumerate()
                .map(|(i, name)| (name.into(), expression(i as i64)))
                .collect(),
        );
        assert_eq!(
            call.arguments().map(|(name, _)| name).collect::<Vec<_>>(),
            ["z", "a", "extra", "z"]
        );
        assert_eq!(
            call.checked_arguments()[0].target,
            ArgumentTarget::Parameter(0)
        );
        assert!(matches!(
            call.checked_arguments()[2].target,
            ArgumentTarget::Extra(_)
        ));
        assert_eq!(
            call.argument_name(&ArgumentTarget::Parameter(usize::MAX)),
            None
        );
        definition.parameters.reverse();
        let changed = call.clone().with_definition(definition);
        assert_eq!(
            changed.checked_arguments()[0].target,
            ArgumentTarget::Parameter(1)
        );
        assert_eq!(
            changed.checked_arguments()[1].target,
            ArgumentTarget::Parameter(0)
        );
        assert_eq!(
            changed
                .arguments()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            ["z", "a", "extra", "z"]
        );
        assert_eq!(
            call.checked_arguments()[0].target,
            ArgumentTarget::Parameter(0)
        );
    }

    #[test]
    fn native_roles_follow_the_implementation_declaration_not_a_global_position() {
        let mut definition = FunctionDef::new(
            FunctionId::new("checked-arguments", "native"),
            crate::builtins::Accepts::Nothing,
            Type::Int,
        );
        definition.implementation =
            crate::definitions::FunctionImpl::Native(crate::definitions::NativeFunction::ArrayLen);
        for name in ["self", "value"] {
            definition
                .parameters
                .push(ParameterDef::new(name, Type::Int, ParameterMode::Required));
        }
        let record = InstanceRecord::new(definition.clone());
        assert_eq!(record.native_parameter(NativeParameter::SelfValue), Some(0));
        definition.parameters.reverse();
        let changed = record.with_definition(definition);
        assert_eq!(
            changed.native_parameter(NativeParameter::SelfValue),
            Some(1)
        );
    }

    #[test]
    fn declaration_edit_rebases_input_slots_by_name() {
        let mut definition = FunctionDef::new(
            FunctionId::new("input-layout", "call"),
            crate::builtins::Accepts::Nothing,
            Type::Int,
        );
        for name in ["x", "y"] {
            definition
                .parameters
                .push(ParameterDef::new(name, Type::Int, ParameterMode::Required));
        }
        let mut bindings = crate::binding::Bindings::default();
        let source = crate::source::Source::metadata("input-layout.notc").unwrap();
        let x = bindings.declare("x", &source, Default::default()).unwrap();
        let y = bindings.declare("y", &source, Default::default()).unwrap();
        let mut record = InstanceRecord::new(definition.clone());
        record.inputs.parameters = vec![Some(x), Some(y)].into_boxed_slice();
        definition.parameters.reverse();
        let changed = record.with_definition(definition);
        assert_eq!(&*changed.inputs.parameters, &[Some(y), Some(x)]);
    }
}

#[cfg(feature = "serde")]
pub(crate) mod span {
    use rowan::TextRange;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(range: &TextRange, serializer: S) -> Result<S::Ok, S::Error> {
        (u32::from(range.start()), u32::from(range.end())).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<TextRange, D::Error> {
        let (start, end) = <(u32, u32)>::deserialize(deserializer)?;
        if start > end {
            return Err(serde::de::Error::custom("reversed source range"));
        }
        Ok(TextRange::new(start.into(), end.into()))
    }
}
