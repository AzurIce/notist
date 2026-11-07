use notist_core::{
    builtins,
    definitions::{ComptimeValue, FunctionId, FunctionInstance, Type},
    diag::Diagnostic,
    evaluation::{CallSite, CheckedProgram, ExprKind, InstanceRecord, Invocation, TypedExpr},
    inputs::{EvaluationInputs, InputRead},
    item::{Ctor, Item, Value},
    registry::Registry,
    resource::ResourcePath,
};
use notist_eval::{EvaluationContext, EvaluationOutcome, Limits};
use std::{cell::Cell, sync::Arc};

/// Core-only embedding, without a parser, pipeline or filesystem dependency.
struct NativeContext {
    source: notist_core::source::Source,
    registry: Registry,
    specializations: Cell<usize>,
    mappings: Cell<usize>,
}

impl NativeContext {
    fn new() -> Self {
        Self {
            source: notist_core::source::Source::new("/doc.not", "native input").unwrap(),
            registry: builtins::registry().clone(),
            specializations: Cell::new(0),
            mappings: Cell::new(0),
        }
    }

    fn call(&self, name: &str, arguments: Vec<(&str, TypedExpr)>) -> Invocation {
        let mut definition = self.registry.function("notist", name).unwrap().clone();
        // These are already checked calls: str<T> has a concrete argument
        // type, just as it would after passing through the real checker.
        let comptime = if name == "str" {
            definition.parameters[0].ty = arguments[0].1.ty.clone().unwrap();
            vec![ComptimeValue::Type(definition.parameters[0].ty.clone())]
        } else {
            vec![]
        };
        Invocation {
            instance: FunctionInstance {
                function: definition.id.clone(),
                arguments: comptime,
            },
            definition: definition.into(),
            arguments: arguments
                .into_iter()
                .map(|(name, expr)| (name.into(), expr))
                .collect(),
            children: None,
            attrs: Default::default(),
            span: Default::default(),
            source: self.source.id(),
            site: CallSite::Code,
            resource_root: Some("/".into()),
            invalid: None,
        }
    }
}

impl EvaluationContext for NativeContext {
    fn has_mappings(&self) -> bool {
        false
    }
    fn registry(&self) -> &Registry {
        &self.registry
    }
    fn resource_root(&self) -> &str {
        "/"
    }
    fn target(&self, id: &FunctionId) -> FunctionId {
        self.mappings.set(self.mappings.get() + 1);
        id.clone()
    }
    fn specialize(
        &self,
        instance: &FunctionInstance,
        mut program: CheckedProgram,
    ) -> (CheckedProgram, Vec<Diagnostic>) {
        self.specializations.set(self.specializations.get() + 1);
        let mut definition = self.registry.get(&instance.function).unwrap().clone();
        let bindings = definition
            .comptime
            .iter()
            .zip(&instance.arguments)
            .map(|(parameter, value)| (parameter.name.clone(), value.clone()))
            .collect();
        definition.returns = definition.returns.substitute(&bindings);
        for parameter in &mut definition.parameters {
            parameter.ty = parameter.ty.substitute(&bindings);
        }
        assert!(!definition.implementation.is_code());
        Arc::make_mut(&mut program.instances).insert(
            instance.key(),
            Arc::new(InstanceRecord {
                definition,
                body: None,
                diagnostics: vec![],
                checking: false,
            }),
        );
        (program, vec![])
    }
}

fn value(value: Value) -> TypedExpr {
    TypedExpr {
        ty: Some(Type::of(&value)),
        known: Some(ComptimeValue::Data(value.clone())),
        kind: ExprKind::Constant(ComptimeValue::Data(value)),
        span: Default::default(),
        source: Default::default(),
        shape: Default::default(),
    }
}

fn document(context: &NativeContext, text: TypedExpr) -> Item {
    let call = context.call("text", vec![("text", text)]);
    let item = Item::expression(TypedExpr {
        ty: Some(Type::InlineContent),
        known: None,
        kind: ExprKind::Call(call.into()),
        span: Default::default(),
        source: context.source.id(),
        shape: Default::default(),
    });
    Item::new(Ctor::Doc, Default::default()).with_children(vec![item])
}

#[test]
fn checked_ir_evaluates_without_the_compiler_and_does_not_mutate_inputs() {
    let context = NativeContext::new();
    let root = document(&context, value(Value::Str("hello".into())));
    let before = root.clone();
    let program = CheckedProgram::default();
    let result = notist_eval::evaluate(
        &root,
        &program,
        &context.source.clone().into(),
        &context,
        Limits::default(),
    );
    assert!(result.diagnostics.is_empty());
    assert_eq!(
        result.root.children()[0].field("text"),
        Some(&Value::Str("hello".into()))
    );
    assert!(
        result
            .root
            .descendants()
            .all(|item| item.as_expr().is_none())
    );
    assert_eq!(root, before);
    assert!(program.instances.is_empty());
    assert_eq!(context.specializations.get(), 1);
}

#[test]
fn standalone_evaluation_rejects_missing_owners_and_pins_escaped_results() {
    let context = NativeContext::new();
    let root = document(&context, value(Value::Str("hello".into())));
    let program = CheckedProgram::default();
    let missing = notist_eval::evaluate(
        &root,
        &program,
        &Default::default(),
        &context,
        Limits::default(),
    );
    assert_eq!(missing.diagnostics.len(), 1);
    assert!(missing.root.is_error());
    let result = notist_eval::evaluate(
        &root,
        &program,
        &context.source.clone().into(),
        &context,
        Limits::default(),
    );
    let id = context.source.id();
    drop(context);
    assert!(result.diagnostics.is_empty());
    assert_eq!(result.sources.name(id).unwrap(), "/doc.not");
    assert!(result.sources.validate_node(&result.root).is_ok());
}

#[test]
fn comptime_uses_the_same_engine_without_runtime_mapping() {
    let context = NativeContext::new();
    let expr = TypedExpr {
        ty: Some(Type::String),
        known: None,
        kind: ExprKind::Call(
            context
                .call("str", vec![("value", value(Value::Int(42)))])
                .into(),
        ),
        span: Default::default(),
        source: context.source.id(),
        shape: Default::default(),
    };
    let result = notist_eval::constant(
        &expr,
        &CheckedProgram::default(),
        &context.source.clone().into(),
        &context,
    )
    .unwrap();
    assert_eq!(result, ComptimeValue::Data(Value::Str("42".into())));
    assert_eq!(context.specializations.get(), 1);
    assert_eq!(context.mappings.get(), 0);
}

struct Inputs {
    reads: Cell<usize>,
    bytes: Option<Arc<[u8]>>,
}
impl EvaluationInputs for Inputs {
    fn file(&self, _: &ResourcePath) -> InputRead {
        self.reads.set(self.reads.get() + 1);
        self.bytes
            .clone()
            .map_or(InputRead::Pending, InputRead::Ready)
    }
}

#[test]
fn explicit_inputs_preserve_pending_complete_and_logical_read_limits() {
    let context = NativeContext::new();
    let path = ResourcePath::resolve("/", "/doc.not", "input.txt").unwrap();
    let expr = TypedExpr {
        ty: Some(Type::String),
        known: None,
        kind: ExprKind::Call(
            context
                .call(
                    "read_text",
                    vec![("source", value(Value::Path(path.clone())))],
                )
                .into(),
        ),
        span: Default::default(),
        source: context.source.id(),
        shape: Default::default(),
    };
    let root = document(&context, expr);
    let pending = Inputs {
        reads: Cell::new(0),
        bytes: None,
    };
    assert_eq!(
        notist_eval::run(
            &root,
            &CheckedProgram::default(),
            &context.source.clone().into(),
            &context,
            Limits::default(),
            Some(&pending)
        ),
        EvaluationOutcome::NeedResources(vec![path.clone()])
    );
    assert_eq!(pending.reads.get(), 1);
    let ready = Inputs {
        reads: Cell::new(0),
        bytes: Some(Arc::from(b"loaded".as_slice())),
    };
    let EvaluationOutcome::Complete(result) = notist_eval::run(
        &root,
        &CheckedProgram::default(),
        &context.source.clone().into(),
        &context,
        Limits::default(),
        Some(&ready),
    ) else {
        panic!("prepared input must complete")
    };
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.dependencies, [path].into_iter().collect());
    assert_eq!(
        result.root.children()[0].field("text"),
        Some(&Value::Str("loaded".into()))
    );
    assert_eq!(ready.reads.get(), 1);
    let EvaluationOutcome::Complete(limited) = notist_eval::run(
        &root,
        &CheckedProgram::default(),
        &context.source.clone().into(),
        &context,
        Limits {
            read_requests: 0,
            ..Limits::default()
        },
        Some(&ready),
    ) else {
        panic!("request limits must fail, not pend")
    };
    assert!(
        limited
            .diagnostics
            .iter()
            .any(|d| d.message.contains("resource request limit"))
    );
    assert_eq!(ready.reads.get(), 1);
}
