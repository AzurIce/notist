use crate::{EvaluationContext, instance_cache::InstanceCache};
#[path = "frame.rs"]
mod frame;
use frame::{EvaluationState, Frame as Item};
#[path = "data.rs"]
mod data;
#[cfg(feature = "test-support")]
#[path = "test_support.rs"]
#[doc(hidden)]
pub mod test_support;
use notist_core::{
    builtins::Level,
    definitions::{
        ComptimeValue, FunctionImpl, FunctionInstance, NativeFunction, ParameterMode, Type,
    },
    diag::{Diagnostic, Phase},
    evaluation::{
        CallSite, CheckedProgram, ExprKind, InstanceRecord, Invocation, PendingEvaluation,
        ResultArm, ResultPattern, TypedExpr,
    },
    inputs::{EvaluationInputs, InputError, InputRead},
    item::{Ctor, Dict, Node, SourceLocation, Value},
    resource::ResourcePath,
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub steps: usize,
    /// Cumulative across nested calls and repeated mounts, including empty output.
    pub iterations: usize,
    pub loop_work: usize,
    /// Maximum normal nodes in a content forest, including enclosing nodes.
    /// Recovery placeholders do not consume this budget.
    pub content_nodes: usize,
    /// The recursive interpreter additionally caps physical call depth at 32.
    pub call_depth: usize,
    /// Requests count even when the host serves them from its shared cache.
    pub read_requests: usize,
    pub value_bytes: usize,
    pub data: notist_core::data::DataLimits,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            steps: 100_000,
            iterations: 100_000,
            loop_work: 1_000_000,
            content_nodes: 100_000,
            call_depth: 32,
            read_requests: 1024,
            value_bytes: 16 * 1024 * 1024,
            data: Default::default(),
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub root: Node,
    pub diagnostics: Vec<Diagnostic>,
    pub dependencies: std::collections::BTreeSet<ResourcePath>,
    /// Exact-version data texts attached only when errors are upgraded.
    pub sources: notist_core::diag::Sources,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EvaluationOutcome {
    Complete(Box<Evaluation>),
    /// No partial tree or output hooks are exposed when an input is unprepared.
    NeedResources(Vec<ResourcePath>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvaluationMode {
    Runtime,
    Comptime,
}

#[derive(Debug)]
pub struct Failure {
    pub message: String,
    pub location: Option<SourceLocation>,
    expansion: Vec<SourceLocation>,
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            message,
            location: None,
            expansion: Vec::new(),
        }
    }
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl Failure {
    fn at(message: impl Into<String>, call: &Invocation) -> Self {
        Self {
            message: message.into(),
            location: Some(SourceLocation {
                source: call.source.as_str().into(),
                start: call.span.start().into(),
                end: call.span.end().into(),
            }),
            expansion: Vec::new(),
        }
    }
}

/// Content values retain opaque calls. Constructing or binding a forest does
/// not execute its content implementations; presentation walks only the result.
#[derive(Debug, Clone)]
enum Runtime {
    Value(ComptimeValue),
    Content(Type, Arc<Vec<Item>>),
}
type Scope = BTreeMap<String, Runtime>;

/// Facts about the result just constructed, not cached metadata on the IR.
#[derive(Clone, Copy, Default)]
struct ContentSummary {
    nodes: usize,
    failed: bool,
}
impl ContentSummary {
    const ERROR: Self = Self {
        nodes: 0,
        failed: true,
    };

    fn append(&mut self, other: Self) {
        self.nodes += other.nodes;
        self.failed |= other.failed;
    }
}

/// Successful presentation preserves a node's identity, source and annotations.
/// Only calls replace its fields/target; evaluation and children are overwritten
/// by error recovery anyway. Keep just that mutable portion when a preceding
/// sibling makes aggregate overflow possible.
struct PresentCheckpoint {
    enabled: bool,
    call_fields: Option<CallFields>,
}
struct CallFields {
    fields: Dict,
    target: Option<notist_core::shared::Shared<FunctionInstance>>,
}
impl PresentCheckpoint {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            call_fields: None,
        }
    }
    fn before_call(&mut self, item: &Item) {
        if self.enabled {
            self.call_fields = Some(CallFields {
                fields: item.fields.clone(),
                target: item.target.clone(),
            });
        }
    }
    fn restore(self, mut item: Item) -> Item {
        if let Some(original) = self.call_fields {
            #[cfg(any(test, feature = "test-support"))]
            CHECKPOINT_RESTORES.set(CHECKPOINT_RESTORES.get() + 1);
            item.fields = original.fields;
            item.target = original.target;
        }
        item
    }
}

#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    static REFERENCE_PRESENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CHECKPOINT_RESTORES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Evaluator<'a, C: EvaluationContext + ?Sized> {
    context: &'a C,
    program: CheckedProgram,
    implementations: InstanceCache,
    diagnostics: Vec<Diagnostic>,
    limits: Limits,
    steps: usize,
    iterations: usize,
    loop_work: usize,
    active_loops: usize,
    depth: usize,
    mode: EvaluationMode,
    inputs: Option<&'a dyn EvaluationInputs>,
    requests: usize,
    dependencies: std::collections::BTreeSet<ResourcePath>,
    needed: std::collections::BTreeSet<ResourcePath>,
    sources: notist_core::diag::Sources,
    data_retention: notist_core::data::DataRetention,
}

pub fn evaluate<C: EvaluationContext + ?Sized>(
    root: &Node,
    program: &CheckedProgram,
    context: &C,
    limits: Limits,
) -> Evaluation {
    match run(root, program, context, limits, None) {
        EvaluationOutcome::Complete(evaluation) => *evaluation,
        EvaluationOutcome::NeedResources(_) => {
            unreachable!("a missing input capability never requests preparation")
        }
    }
}
pub fn run<C: EvaluationContext + ?Sized>(
    root: &Node,
    program: &CheckedProgram,
    context: &C,
    limits: Limits,
    inputs: Option<&dyn EvaluationInputs>,
) -> EvaluationOutcome {
    let mut e = Evaluator {
        context,
        program: program.clone(),
        implementations: InstanceCache::default(),
        diagnostics: Vec::new(),
        limits,
        steps: 0,
        iterations: 0,
        loop_work: 0,
        active_loops: 0,
        depth: 0,
        mode: EvaluationMode::Runtime,
        inputs,
        requests: 0,
        dependencies: Default::default(),
        needed: Default::default(),
        sources: Default::default(),
        data_retention: program.data_retention.clone(),
    };
    let root = Item::from_node(root);
    let (forest, _) = e.bind_content(
        std::slice::from_ref(&root),
        &Scope::new(),
        limits.content_nodes,
    );
    let (mut roots, _) = e.present(forest, limits.content_nodes);
    if !e.needed.is_empty() {
        return EvaluationOutcome::NeedResources(e.needed.into_iter().collect());
    }
    for diagnostic in &mut e.diagnostics {
        diagnostic.recovered = true;
    }
    EvaluationOutcome::Complete(Box::new(Evaluation {
        root: {
            let mut nodes = roots.remove(0).finish();
            if nodes.len() == 1 {
                nodes.remove(0)
            } else {
                Node::new(Ctor::Group, root.span).with_children(nodes)
            }
        },
        diagnostics: e.diagnostics,
        dependencies: e.dependencies,
        sources: e.sources,
    }))
}
pub fn constant<C: EvaluationContext + ?Sized>(
    t: &TypedExpr,
    program: &CheckedProgram,
    context: &C,
) -> Result<ComptimeValue, Failure> {
    let mut e = Evaluator {
        context,
        program: program.clone(),
        implementations: InstanceCache::default(),
        diagnostics: Vec::new(),
        limits: Limits::default(),
        steps: 0,
        iterations: 0,
        loop_work: 0,
        active_loops: 0,
        depth: 0,
        mode: EvaluationMode::Comptime,
        inputs: None,
        requests: 0,
        dependencies: Default::default(),
        needed: Default::default(),
        sources: Default::default(),
        data_retention: program.data_retention.clone(),
    };
    match e.expr(t, &Scope::new())? {
        Runtime::Value(v) => Ok(v),
        Runtime::Content(..) => Err("content cannot be a comptime argument".into()),
    }
}
impl<C: EvaluationContext + ?Sized> Evaluator<'_, C> {
    fn target(
        &self,
        id: &notist_core::definitions::FunctionId,
    ) -> notist_core::definitions::FunctionId {
        if self.mode == EvaluationMode::Runtime {
            self.context.target(id)
        } else {
            id.clone()
        }
    }
    fn tick(&mut self) -> Result<(), Failure> {
        self.steps = self.steps.saturating_add(1);
        if self.active_loops > 0 {
            self.loop_work = self.loop_work.saturating_add(1);
            if self.loop_work > self.limits.loop_work {
                return Err("cumulative loop work limit exceeded".into());
            }
        }
        if self.steps > self.limits.steps {
            Err("evaluation step limit exceeded".into())
        } else {
            Ok(())
        }
    }
    fn expr(&mut self, t: &TypedExpr, scope: &Scope) -> Result<Runtime, Failure> {
        self.tick()?;
        if let Some(v) = &t.known {
            check_value_size(v, self.limits.value_bytes)?;
            if let ComptimeValue::Data(value) = v {
                self.data_retention
                    .observe(value, self.limits.data.working_bytes)
                    .map_err(|e| Failure::from(e.to_string()))?;
            }
            return Ok(Runtime::Value(v.clone()));
        }
        let result = match &t.kind {
            ExprKind::Constant(v) => Ok(Runtime::Value(v.clone())),
            ExprKind::Input(name) => scope.get(name).cloned().ok_or_else(|| {
                format!("runtime input `{name}` is unavailable at compile time").into()
            }),
            ExprKind::Failure(m) => Err(m.clone().into()),
            ExprKind::Content(forest) => {
                if self.mode == EvaluationMode::Comptime {
                    return Err("content cannot be constructed at compile time".into());
                }
                Ok(Runtime::Content(
                    t.ty.clone().unwrap(),
                    Arc::new(
                        self.bind_content(
                            &forest.iter().map(Item::from_node).collect::<Vec<_>>(),
                            scope,
                            self.limits.content_nodes,
                        )
                        .0,
                    ),
                ))
            }
            ExprKind::For { .. } => {
                if self.mode == EvaluationMode::Comptime {
                    return Err("content traversal cannot run during comptime evaluation".into());
                }
                let ty = t.ty.clone().ok_or("missing checked for result category")?;
                let mut item = Item::new(Ctor::Interpolation, t.span);
                item.level = ty.level().ok_or("for requires a content result")?;
                item.origin = Some(notist_core::item::Origin {
                    source: t.source.as_str().into(),
                    start: t.span.start().into(),
                    end: t.span.end().into(),
                    expansion: Default::default(),
                });
                item.pending = Some(Box::new(PendingEvaluation::Repeat(capture_expr(t, scope))));
                Ok(Runtime::Content(ty, Arc::new(vec![item])))
            }
            ExprKind::Block(bindings, value) => {
                let mut local = scope.clone();
                for (name, value) in bindings {
                    let value = self.expr(value, &local)?;
                    local.insert(name.clone(), value);
                }
                self.expr(value, &local)
            }
            ExprKind::If(condition, yes, no) => match data(self.expr(condition, scope)?)? {
                Value::Bool(c) => self.expr(if c { yes } else { no }, scope),
                _ => Err("condition must be Bool".into()),
            },
            ExprKind::Unary(op, value) => {
                let v = value_of(self.expr(value, scope)?)?;
                unary(op, &v).map(Runtime::Value).map_err(Into::into)
            }
            ExprKind::Binary(op, left, right) => {
                let left = value_of(self.expr(left, scope)?)?;
                if op == "&&" && left == ComptimeValue::Data(Value::Bool(false))
                    || op == "||" && left == ComptimeValue::Data(Value::Bool(true))
                {
                    return Ok(Runtime::Value(left));
                }
                let right = value_of(self.expr(right, scope)?)?;
                binary(op, &left, &right)
                    .map(Runtime::Value)
                    .map_err(Into::into)
            }
            ExprKind::Array(values) => {
                let values = values
                    .iter()
                    .map(|v| {
                        self.expr(v, scope)
                            .and_then(|value| data(value).map_err(Into::into))
                    })
                    .collect::<Result<_, _>>()?;
                Ok(Runtime::Value(ComptimeValue::Data(Value::Array(values))))
            }
            ExprKind::Dict(values) => {
                let values = values
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), data(self.expr(v, scope)?)?)))
                    .collect::<Result<_, Failure>>()?;
                Ok(Runtime::Value(ComptimeValue::Data(Value::Dict(values))))
            }
            ExprKind::Some(value) => {
                let value = data(self.expr(value, scope)?)?;
                Ok(Runtime::Value(ComptimeValue::Data(Value::Some(Box::new(
                    value,
                )))))
            }
            ExprKind::Unwrap(value) => match data(self.expr(value, scope)?)? {
                Value::Some(value) => Ok(Runtime::Value(ComptimeValue::Data(*value))),
                _ => Err("match arm requires `some`".into()),
            },
            ExprKind::Ok(value) | ExprKind::Err(value) => {
                let value = Box::new(data(self.expr(value, scope)?)?);
                let value = if matches!(&t.kind, ExprKind::Ok(_)) {
                    Value::Ok(value)
                } else {
                    Value::Err(value)
                };
                Ok(Runtime::Value(ComptimeValue::Data(value)))
            }
            ExprKind::MatchResult { subject, arms } => {
                let (ok, value) = match data(self.expr(subject, scope)?)? {
                    Value::Ok(value) => (true, value),
                    Value::Err(value) => (false, value),
                    _ => return Err("Result match requires a Result value".into()),
                };
                let mut selected = None;
                for arm in arms {
                    let binding = match &arm.pattern {
                        ResultPattern::Ok(name) if ok => Some(name),
                        ResultPattern::Err(name) if !ok => Some(name),
                        ResultPattern::Wildcard => None,
                        _ => continue,
                    };
                    let mut local = scope.clone();
                    if let Some(name) = binding.filter(|name| name.as_str() != "_") {
                        local.insert(name.clone(), Runtime::Value(ComptimeValue::Data(*value)));
                    }
                    selected = Some(self.expr(&arm.body, &local));
                    break;
                }
                selected.unwrap_or_else(|| Err("non-exhaustive Result match".into()))
            }
            ExprKind::Call(call) => {
                if call.definition.returns.level().is_some() {
                    if self.mode == EvaluationMode::Comptime {
                        return Err("content cannot be constructed at compile time".into());
                    }
                    let mut expr = t.clone();
                    expr.kind = ExprKind::Call(Box::new(capture_call(call, scope)));
                    let frame = Item::from_node(&Node::expression(expr));
                    let (nodes, _) = self.bind_content(
                        std::slice::from_ref(&frame),
                        &Scope::new(),
                        self.limits.content_nodes,
                    );
                    return Ok(Runtime::Content(
                        call.definition.returns.clone(),
                        Arc::new(nodes),
                    ));
                }
                let children = match &call.children {
                    Some(child) => match self.expr(child, scope)? {
                        Runtime::Content(_, nodes) => Arc::unwrap_or_clone(nodes),
                        _ => return Err("invalid children".into()),
                    },
                    None => Vec::new(),
                };
                self.execute(call, scope, children, &mut None)
            }
        }?;
        if let Runtime::Value(v) = &result {
            check_value_size(v, self.limits.value_bytes)?;
        }
        Ok(result)
    }
    /// Bind a content value without executing opaque content calls. The result
    /// remains an Item forest; only mounted Items are expanded by `present`.
    fn bind_content(&mut self, forest: &[Item], scope: &Scope, limit: usize) -> (Vec<Item>, usize) {
        let mut result = Vec::with_capacity(forest.len());
        let mut count = 0usize;
        for node in forest {
            let (item, added) = match node.pending.as_deref() {
                Some(PendingEvaluation::Failure { message, .. }) => {
                    (error_content(item_shell(node), message.clone()), 0)
                }
                Some(PendingEvaluation::Splice(expr)) => match self.expr(expr, scope) {
                    Ok(Runtime::Content(ty, nodes))
                        if expr.ty.as_ref() == Some(&ty) && ty.level() == Some(node.level) =>
                    {
                        let mut group = item_shell(node);
                        group.evaluation = Some(EvaluationState::Expanded);
                        group.children = Arc::unwrap_or_clone(nodes);
                        // A spliced value is shared independently of this binding
                        // walk, so count it once at the explicit mount boundary.
                        let added = 1 + content_count(&group.children);
                        (group, added)
                    }
                    Ok(Runtime::Content(..)) => (
                        error_content(
                            item_shell(node),
                            "content interpolation violated its checked category".into(),
                        ),
                        0,
                    ),
                    Ok(_) => (
                        error_content(
                            item_shell(node),
                            "content interpolation produced a data value".into(),
                        ),
                        0,
                    ),
                    Err(failure) => (failure_content(item_shell(node), failure), 0),
                },
                pending => {
                    let mut item = item_shell(node);
                    if let Some(PendingEvaluation::Call(call)) = pending {
                        item.pending =
                            Some(Box::new(PendingEvaluation::Call(capture_call(call, scope))));
                    }
                    if let Some(PendingEvaluation::Repeat(expr)) = pending {
                        item.pending = Some(Box::new(PendingEvaluation::Repeat(capture_expr(
                            expr, scope,
                        ))));
                    }
                    let (children, count) =
                        self.bind_content(&node.children, scope, limit.saturating_sub(1));
                    item.children = children;
                    let own =
                        usize::from(!matches!(item.evaluation, Some(EvaluationState::Error(_))));
                    (item, count + own)
                }
            };
            if count.saturating_add(added) > limit {
                result.push(error_content(
                    item_shell(node),
                    "content size limit exceeded".into(),
                ));
                continue;
            }
            count += added;
            result.push(item);
        }
        (result, count)
    }
    fn present(&mut self, forest: Vec<Item>, limit: usize) -> (Vec<Item>, ContentSummary) {
        #[cfg(any(test, feature = "test-support"))]
        if REFERENCE_PRESENT.get() {
            return self.present_reference(forest, limit);
        }
        let mut result = Vec::with_capacity(forest.len());
        let mut summary = ContentSummary::default();
        for node in forest {
            // Each node is itself bounded by limit. Aggregate overflow needs a
            // preceding normal sibling; errors contribute zero nodes.
            let mut checkpoint = PresentCheckpoint::new(summary.nodes != 0);
            let (item, added) = self.present_node(node, limit, &mut checkpoint);
            if summary.nodes.saturating_add(added.nodes) > limit {
                result.push(self.error(
                    checkpoint.restore(item),
                    "content size limit exceeded".into(),
                ));
                summary.failed = true;
                continue;
            }
            summary.append(added);
            result.push(item);
        }
        (result, summary)
    }
    #[cfg(any(test, feature = "test-support"))]
    fn present_reference(
        &mut self,
        forest: Vec<Item>,
        limit: usize,
    ) -> (Vec<Item>, ContentSummary) {
        let mut result = Vec::with_capacity(forest.len());
        let mut summary = ContentSummary::default();
        for node in forest {
            // The pre-checkpoint implementation, also used by recursive calls
            // while REFERENCE_PRESENT is enabled. Never linked into the CLI.
            let boundary = item_shell(&node);
            let (item, added) = self.present_node(node, limit, &mut PresentCheckpoint::new(false));
            if summary.nodes.saturating_add(added.nodes) > limit {
                result.push(self.error(boundary, "content size limit exceeded".into()));
                summary.failed = true;
                continue;
            }
            summary.append(added);
            result.push(item);
        }
        (result, summary)
    }
    fn present_node(
        &mut self,
        mut item: Item,
        limit: usize,
        checkpoint: &mut PresentCheckpoint,
    ) -> (Item, ContentSummary) {
        if limit == 0 && !matches!(item.evaluation, Some(EvaluationState::Error(_))) {
            return (
                self.error(item, "content size limit exceeded".into()),
                ContentSummary::ERROR,
            );
        }
        if let Some(EvaluationState::Error(message)) = &item.evaluation {
            let message = message.clone();
            return (self.error(item, message), ContentSummary::ERROR);
        }
        match item.pending.take().as_deref() {
            Some(PendingEvaluation::Call(call)) => {
                checkpoint.before_call(&item);
                return self.present_call(item, call, limit);
            }
            Some(PendingEvaluation::Repeat(expr)) => {
                return match self.repeat(expr, limit.saturating_sub(1), &item) {
                    Ok((children, mut summary)) => {
                        item.evaluation = Some(EvaluationState::Expanded);
                        item.children = children;
                        summary.nodes += 1;
                        (item, summary)
                    }
                    Err(failure) => (self.error_at(item, failure), ContentSummary::ERROR),
                };
            }
            _ => {}
        }
        if item.evaluation.is_none()
            && let Some(id) = item.function_id()
            && self.target(&id) != id
            && let Some(call) = self.synthetic_call(&item)
        {
            checkpoint.before_call(&item);
            return self.present_call(item, &call, limit);
        }
        let (children, mut summary) =
            self.present(std::mem::take(&mut item.children), limit.saturating_sub(1));
        item.children = children;
        if !valid_structure(&item) {
            return (
                self.error(
                    item,
                    "evaluated content violated its checked structural role".into(),
                ),
                ContentSummary::ERROR,
            );
        }
        summary.nodes += 1;
        (item, summary)
    }
    fn repeat(
        &mut self,
        expr: &TypedExpr,
        limit: usize,
        group: &Item,
    ) -> Result<(Vec<Item>, ContentSummary), Failure> {
        let ExprKind::For {
            binding,
            subject,
            body,
        } = &expr.kind
        else {
            return Err("invalid deferred for computation".into());
        };
        if self.active_loops >= 32 {
            return Err("for nesting limit exceeded".into());
        }
        self.active_loops += 1;
        let result = (|| {
            let Value::Array(values) = data(self.expr(subject, &Scope::new())?)? else {
                return Err("for subject did not produce an Array".into());
            };
            if values.len() > self.limits.iterations.saturating_sub(self.iterations) {
                return Err("cumulative iteration limit exceeded".into());
            }
            let mut result = Vec::new();
            let mut summary = ContentSummary::default();
            let mut local = Scope::new();
            for (index, value) in values.iter().enumerate() {
                self.iterations = self.iterations.saturating_add(1);
                if self.iterations > self.limits.iterations {
                    return Err("cumulative iteration limit exceeded".into());
                }
                self.tick()?;
                local.insert(
                    binding.clone(),
                    Runtime::Value(ComptimeValue::Data(value.clone())),
                );
                let value = self.expr(body, &local).map_err(|mut failure| {
                    failure.message = format!("for iteration {index}: {}", failure.message);
                    failure
                })?;
                let Runtime::Content(ty, nodes) = value else {
                    return Err("for body produced data".into());
                };
                if Some(&ty) != expr.ty.as_ref() {
                    return Err("for body violated its checked category".into());
                }
                let mut nodes = Arc::unwrap_or_clone(nodes);
                if let Some(origin) = &group.origin {
                    for node in &mut nodes {
                        expand(node, &origin.expansion);
                    }
                }
                let remaining = limit.saturating_sub(summary.nodes);
                let (nodes, added) = self.present(nodes, remaining);
                if summary.nodes.saturating_add(added.nodes) > limit {
                    return Err("content size limit exceeded".into());
                }
                // A failed iteration never exposes a successfully assembled prefix.
                if added.failed {
                    return Err(format!("for iteration {index} failed").into());
                }
                summary.append(added);
                result.extend(nodes);
            }
            Ok((result, summary))
        })();
        self.active_loops -= 1;
        result
    }
    fn present_call(
        &mut self,
        mut item: Item,
        call: &Invocation,
        limit: usize,
    ) -> (Item, ContentSummary) {
        let target = self.target(&call.instance.function);
        let expands = self
            .context
            .registry()
            .get(&target)
            .is_some_and(|d| d.implementation.is_code());
        if expands && self.depth >= self.limits.call_depth.min(32) {
            return (
                self.error(item, "evaluation call-depth limit exceeded".into()),
                ContentSummary::ERROR,
            );
        }
        if expands {
            self.depth += 1;
        }
        // Each mounted content boundary owns its computation budget.
        let previous_steps = std::mem::replace(&mut self.steps, 0);
        let children = std::mem::take(&mut item.children);
        // Keep ownership for recovery. execute only takes the boundary once all
        // fallible work has succeeded; no eager metadata backup is necessary.
        let mut boundary = Some(item);
        let result = self.execute(call, &Scope::new(), children, &mut boundary);
        let result = match result {
            Ok(Runtime::Content(_, nodes)) => {
                let (mut nodes, summary) = self.present(Arc::unwrap_or_clone(nodes), limit);
                // execute retains exactly one materialized call boundary,
                // regardless of how many children the implementation produced.
                debug_assert_eq!(nodes.len(), 1);
                (
                    nodes.pop().expect("content call retains its boundary"),
                    summary,
                )
            }
            Ok(_) => (
                self.error(
                    boundary.expect("data results retain the content boundary"),
                    "content call produced a data value".into(),
                ),
                ContentSummary::ERROR,
            ),
            Err(failure) => (
                self.error_at(
                    boundary.expect("failed calls retain the content boundary"),
                    failure,
                ),
                ContentSummary::ERROR,
            ),
        };
        if expands {
            self.depth -= 1;
        }
        self.steps = previous_steps;
        result
    }
    fn synthetic_call(&self, item: &Item) -> Option<Invocation> {
        let id = item.function_id()?;
        let mut def = self.context.registry().get(&id)?.clone();
        let instance = item
            .instance
            .as_ref()
            .map(|instance| (**instance).clone())
            .unwrap_or_else(|| FunctionInstance {
                function: id,
                arguments: def
                    .comptime
                    .iter()
                    .filter_map(|p| {
                        if p.ty == Type::Type {
                            Some(ComptimeValue::Type(if item.level == Level::Inline {
                                Type::InlineContent
                            } else {
                                Type::BlockContent
                            }))
                        } else {
                            p.default.clone()
                        }
                    })
                    .collect(),
            });
        let bindings = def
            .comptime
            .iter()
            .zip(&instance.arguments)
            .map(|(p, v)| (p.name.clone(), v.clone()))
            .collect();
        def.returns = def.returns.substitute(&bindings);
        def.children_type = def.children_type.as_ref().map(|t| t.substitute(&bindings));
        for parameter in &mut def.parameters {
            parameter.ty = parameter.ty.substitute(&bindings);
        }
        let source = item
            .origin
            .as_ref()
            .map(|o| o.source.to_string())
            .unwrap_or_default();
        let arguments = item
            .fields
            .iter()
            .map(|(name, v)| {
                (
                    name.into(),
                    TypedExpr {
                        shape: Default::default(),
                        ty: Some(
                            def.parameters
                                .iter()
                                .find(|p| p.name == name)
                                .map(|p| p.ty.clone())
                                .unwrap_or_else(|| Type::of(v)),
                        ),
                        known: Some(ComptimeValue::Data(v.clone())),
                        kind: ExprKind::Constant(ComptimeValue::Data(v.clone())),
                        span: item.span,
                        source: source.clone(),
                    },
                )
            })
            .collect();
        Some(Invocation {
            instance,
            definition: def.into(),
            arguments,
            children: None,
            attrs: item.attrs.clone(),
            span: item.span,
            source,
            site: notist_core::evaluation::CallSite::Markup,
            resource_root: Some(self.context.resource_root().into()),
            invalid: None,
        })
    }
    fn error(&mut self, mut item: Item, message: String) -> Item {
        let mut d = Diagnostic::new(Phase::Type, item.span, &message);
        d.source = item.origin.as_ref().map(|o| o.source.to_string());
        self.diagnostics.push(d);
        if let Some(instance) = &item.instance {
            item.target = Some(
                FunctionInstance {
                    function: self.target(&instance.function),
                    arguments: instance.arguments.clone(),
                }
                .into(),
            );
        }
        item.evaluation = Some(EvaluationState::Error(message));
        item.pending = None;
        item.children.clear();
        item
    }
    fn error_at(&mut self, item: Item, failure: Failure) -> Item {
        let item = failure_content(item, failure);
        let Some(EvaluationState::Error(message)) = &item.evaluation else {
            unreachable!()
        };
        let message = message.clone();
        self.error(item, message)
    }
    fn read_text(&mut self, call: &Invocation, fields: &Dict) -> Result<Runtime, Failure> {
        if self.mode == EvaluationMode::Comptime {
            return Err(Failure::at(
                "resource reading is not permitted during comptime evaluation",
                call,
            ));
        }
        if call.site != CallSite::Code {
            return Err(Failure::at(
                "read_text can only be called from notc Code",
                call,
            ));
        }
        self.requests += 1;
        if self.requests > self.limits.read_requests {
            return Err(Failure::at("resource request limit exceeded", call));
        }
        let Some(Value::Path(path)) = fields.get("source") else {
            return Err(Failure::at("read_text requires a Path", call));
        };
        self.dependencies.insert(path.clone());
        let data = match self
            .inputs
            .map(|inputs| inputs.file(path))
            .unwrap_or(InputRead::Failed(InputError::Unavailable))
        {
            InputRead::Ready(data) => data,
            InputRead::Failed(error) => {
                return Err(Failure::at(
                    format!("read_text `{}`: {error}", path.path()),
                    call,
                ));
            }
            InputRead::Pending => {
                self.needed.insert(path.clone());
                return Err(Failure::at("resource input is not prepared", call));
            }
        };
        if data.len() > self.limits.value_bytes {
            return Err(Failure::at("value size limit exceeded", call));
        }
        let text = std::str::from_utf8(&data).map_err(|_| {
            Failure::at(
                format!("read_text `{}`: resource is not valid UTF-8", path.path()),
                call,
            )
        })?;
        Ok(Runtime::Value(ComptimeValue::Data(Value::Str(text.into()))))
    }
    fn execute(
        &mut self,
        call: &Invocation,
        scope: &Scope,
        children: Vec<Item>,
        prototype: &mut Option<Item>,
    ) -> Result<Runtime, Failure> {
        self.tick()?;
        if let Some(m) = &call.invalid {
            return Err(m.clone().into());
        }
        let target = FunctionInstance {
            function: self.target(&call.instance.function),
            arguments: call.instance.arguments.clone(),
        };
        let code = self
            .context
            .registry()
            .get(&target.function)
            .is_some_and(|definition| definition.implementation.is_code());
        let mut inputs = Scope::new();
        let mut fields = Dict::default();
        for (name, expr) in &call.arguments {
            let value = self.expr(expr, scope)?;
            let param = call.definition.parameters.iter().find(|p| p.name == *name);
            // Forwarding unwraps one layer: some(v) provides v, none omits.
            let provided = if param.is_some_and(|p| p.mode == ParameterMode::Optional)
                && matches!(&expr.ty, Some(Type::Option(_)))
            {
                match value {
                    Runtime::Value(ComptimeValue::Data(Value::Some(v))) => {
                        Some(Runtime::Value(ComptimeValue::Data(*v)))
                    }
                    Runtime::Value(ComptimeValue::Data(Value::None)) => None,
                    _ => {
                        return Err(format!(
                            "invalid option for `{}.{}`",
                            call.instance.function, name
                        )
                        .into());
                    }
                }
            } else {
                Some(value)
            };
            let Some(value) = provided else { continue };
            if let Some(param) = param
                && let Runtime::Value(ComptimeValue::Data(value)) = &value
                && !param.accepts(value)
            {
                return Err(
                    format!("invalid value for `{}.{name}`", call.instance.function).into(),
                );
            }
            if let Runtime::Value(ComptimeValue::Data(v)) = &value {
                fields.insert(name, v.clone());
            }
            // Code bodies observe a `?: T` parameter as Option<T>.
            if code {
                let input = if param.is_some_and(|p| p.mode == ParameterMode::Optional) {
                    match value {
                        Runtime::Value(ComptimeValue::Data(v)) => {
                            Runtime::Value(ComptimeValue::Data(Value::Some(Box::new(v))))
                        }
                        other => other,
                    }
                } else {
                    value
                };
                inputs.insert(name.clone(), input);
            }
        }
        let mut children = Some(children);
        if code {
            for p in &call.definition.parameters {
                if p.mode == ParameterMode::Optional && !inputs.contains_key(&p.name) {
                    inputs.insert(
                        p.name.clone(),
                        Runtime::Value(ComptimeValue::Data(Value::None)),
                    );
                }
            }
            if let Some(ty) = &call.definition.children_type {
                inputs.insert(
                    "children".into(),
                    Runtime::Content(ty.clone(), Arc::new(children.take().unwrap())),
                );
            }
        }
        let implementation = self.implementation(&target)?;
        let definition = &implementation.definition;
        if code {
            for (param, value) in definition.comptime.iter().zip(&target.arguments) {
                inputs.insert(param.name.clone(), Runtime::Value(value.clone()));
            }
        }
        if call.definition.id == notist_core::definitions::FunctionId::new("notist", "heading")
            && let Some(ComptimeValue::Data(v)) = call.instance.arguments.first()
        {
            fields.insert("level", v.clone());
        }
        let value_call = call.definition.returns.level().is_none();
        if value_call {
            if self.depth >= self.limits.call_depth.min(32) {
                return Err("evaluation call-depth limit exceeded".into());
            }
            self.depth += 1;
        }
        let result = match definition.implementation {
            FunctionImpl::Native(NativeFunction::Str) => {
                let text: notist_core::shared::Text = match fields.get("value") {
                    Some(Value::Unit) => "()".into(),
                    Some(Value::Bool(v)) => v.to_string().into(),
                    Some(Value::Int(v)) => v.to_string().into(),
                    Some(Value::Str(v)) => v.clone(),
                    Some(Value::Float(v)) if v.is_nan() => "NaN".into(),
                    Some(Value::Float(v)) if *v == f64::INFINITY => "Infinity".into(),
                    Some(Value::Float(v)) if *v == f64::NEG_INFINITY => "-Infinity".into(),
                    Some(Value::Float(v)) => v.to_string().into(),
                    _ => {
                        return Err(Failure::at(
                            "str accepts only Unit, Bool, Int, Float or String",
                            call,
                        ));
                    }
                };
                Ok(Runtime::Value(ComptimeValue::Data(Value::Str(text))))
            }
            FunctionImpl::Native(NativeFunction::ArrayLen) => {
                let Some(Value::Array(values)) = fields.get("self") else {
                    return Err(Failure::at("len requires an Array receiver", call));
                };
                let len = i64::try_from(values.len())
                    .map_err(|_| Failure::at("array length exceeds Int", call))?;
                Ok(Runtime::Value(ComptimeValue::Data(Value::Int(len))))
            }
            FunctionImpl::Native(NativeFunction::ReadText) => self.read_text(call, &fields),
            FunctionImpl::Native(NativeFunction::Data(kind)) => {
                self.data_native(kind, call, &fields)
            }
            FunctionImpl::Native(NativeFunction::Panic) => {
                let Some(Value::Str(message)) = fields.get("message") else {
                    return Err(Failure::at("panic requires a String message", call));
                };
                Err(Failure::at(message.clone(), call))
            }
            FunctionImpl::Code => match &implementation.body {
                Some(body) => self.expr(body, &inputs),
                None => Err("missing checked body".into()),
            },
            FunctionImpl::Native(NativeFunction::Construct) => {
                if definition.returns.level().is_none() {
                    Err(format!(
                        "native value function `{}` has no implementation",
                        definition.id
                    )
                    .into())
                } else if self.mode == EvaluationMode::Comptime {
                    Err("content cannot be constructed at compile time".into())
                } else {
                    if definition.id == notist_core::definitions::FunctionId::new("notist", "embed")
                    {
                        notist_core::embed::normalize(
                            &mut fields,
                            call.resource_root
                                .as_deref()
                                .unwrap_or(self.context.resource_root()),
                            &call.source,
                        )?;
                    }
                    Ok(Runtime::Content(
                        definition.returns.clone(),
                        Arc::new(children.take().unwrap()),
                    ))
                }
            }
        };
        if value_call {
            self.depth -= 1;
        }
        let result = result.map_err(|mut failure| {
            if failure.location.is_some() {
                let location = SourceLocation {
                    source: call.source.as_str().into(),
                    start: call.span.start().into(),
                    end: call.span.end().into(),
                };
                if failure.location.as_ref() != Some(&location)
                    && failure.expansion.last() != Some(&location)
                {
                    failure.expansion.push(location);
                }
            }
            failure
        })?;
        if call.definition.returns.level().is_some() {
            let Runtime::Content(ty, mut forest) = result else {
                return Err("content implementation produced a data value".into());
            };
            if ty != call.definition.returns {
                return Err("implementation violated its content result type".into());
            }
            // Commit ownership only after every Result-producing operation.
            // On failure the caller still owns its unmodified recovery shell.
            let mut item = prototype
                .take()
                .ok_or("missing materialized content call boundary")?;
            item.fields = fields;
            item.pending = None;
            item.target = Some(target.into());
            let native =
                definition.implementation == FunctionImpl::Native(NativeFunction::Construct);
            item.evaluation = Some(if native {
                EvaluationState::Native
            } else {
                EvaluationState::Expanded
            });
            if !native {
                let mut expansion = vec![SourceLocation {
                    source: call.source.as_str().into(),
                    start: call.span.start().into(),
                    end: call.span.end().into(),
                }];
                if let Some(origin) = &item.origin {
                    expansion.extend(origin.expansion.iter().cloned());
                }
                for child in Arc::make_mut(&mut forest) {
                    expand(child, &expansion);
                }
            }
            Ok(Runtime::Content(call.definition.returns.clone(), {
                item.children = Arc::unwrap_or_clone(forest);
                Arc::new(vec![item])
            }))
        } else {
            let valid = match &result {
                Runtime::Value(ComptimeValue::Data(value)) => {
                    call.definition.returns.accepts(value)
                }
                Runtime::Value(ComptimeValue::Type(_)) => call.definition.returns == Type::Type,
                Runtime::Content(..) => false,
            };
            if !valid {
                return Err("implementation violated its data result type".into());
            }
            Ok(result)
        }
    }
    fn implementation(
        &mut self,
        instance: &FunctionInstance,
    ) -> Result<Arc<InstanceRecord>, Failure> {
        if instance
            .arguments
            .iter()
            .any(|v| matches!(v, ComptimeValue::Data(v) if v.contains_data_views()))
        {
            return Err("Data and DataError views cannot be comptime value arguments".into());
        }
        if let Some(record) = self.implementations.get(instance) {
            return Ok(record.clone());
        }
        let key = instance.key();
        if !self.program.instances.contains_key(&key) {
            // Runtime specializations share the same retained backing-store
            // budget, including comptime views in currently unselected arms.
            self.program.data_retention = self.data_retention.clone();
            let (program, diagnostics) = self
                .context
                .specialize(instance, std::mem::take(&mut self.program));
            self.data_retention = program.data_retention.clone();
            self.diagnostics.extend(diagnostics);
            self.program = program;
        }
        let record = self
            .program
            .instances
            .get(&key)
            .ok_or("missing function instance")?;
        if let Some(d) = record.diagnostics.first() {
            return Err(format!("{}: {}", instance.function, d.message).into());
        }
        self.implementations.insert(instance, record);
        Ok(record.clone())
    }
}
// Capture only free inputs in pending expressions. Content calls remain opaque;
// substitution performs no interpretation, including in unused branches.
fn capture_call(call: &Invocation, scope: &Scope) -> Invocation {
    if scope.is_empty() {
        return call.clone();
    }
    Invocation {
        instance: call.instance.clone(),
        definition: call.definition.clone(),
        arguments: call
            .arguments
            .iter()
            .map(|(name, expr)| (name.clone(), capture_expr(expr, scope)))
            .collect(),
        children: call
            .children
            .as_ref()
            .map(|e| Box::new(capture_expr(e, scope))),
        attrs: call.attrs.clone(),
        span: call.span,
        source: call.source.clone(),
        site: call.site,
        resource_root: call.resource_root.clone(),
        invalid: call.invalid.clone(),
    }
}
fn capture_item(item: &Item, scope: &Scope) -> Item {
    let mut captured = item_shell(item);
    captured.pending = item.pending.as_deref().map(|pending| {
        Box::new(match pending {
            PendingEvaluation::Call(call) => PendingEvaluation::Call(capture_call(call, scope)),
            PendingEvaluation::Splice(expr) => PendingEvaluation::Splice(capture_expr(expr, scope)),
            PendingEvaluation::Repeat(expr) => PendingEvaluation::Repeat(capture_expr(expr, scope)),
            failure => failure.clone(),
        })
    });
    captured.children = item
        .children
        .iter()
        .map(|i| capture_item(i, scope))
        .collect();
    captured
}
fn capture_expr(expr: &TypedExpr, scope: &Scope) -> TypedExpr {
    if scope.is_empty() || expr.known.is_some() {
        return expr.clone();
    }
    let mut known = expr.known.clone();
    let kind = match &expr.kind {
        ExprKind::Input(name) => match scope.get(name) {
            Some(Runtime::Value(v)) => {
                known = Some(v.clone());
                ExprKind::Constant(v.clone())
            }
            Some(Runtime::Content(_, forest)) => {
                ExprKind::Content(forest.iter().map(Item::recipe).collect())
            }
            None => expr.kind.clone(),
        },
        ExprKind::For {
            binding,
            subject,
            body,
        } => {
            let mut local = scope.clone();
            local.remove(binding);
            ExprKind::For {
                binding: binding.clone(),
                subject: Box::new(capture_expr(subject, scope)),
                body: Box::new(capture_expr(body, &local)),
            }
        }
        ExprKind::Array(values) => {
            ExprKind::Array(values.iter().map(|e| capture_expr(e, scope)).collect())
        }
        ExprKind::Dict(values) => ExprKind::Dict(
            values
                .iter()
                .map(|(k, e)| (k.clone(), capture_expr(e, scope)))
                .collect(),
        ),
        ExprKind::Unary(op, e) => ExprKind::Unary(op.clone(), Box::new(capture_expr(e, scope))),
        ExprKind::Some(e) => ExprKind::Some(Box::new(capture_expr(e, scope))),
        ExprKind::Unwrap(e) => ExprKind::Unwrap(Box::new(capture_expr(e, scope))),
        ExprKind::Ok(e) => ExprKind::Ok(Box::new(capture_expr(e, scope))),
        ExprKind::Err(e) => ExprKind::Err(Box::new(capture_expr(e, scope))),
        ExprKind::MatchResult { subject, arms } => ExprKind::MatchResult {
            subject: Box::new(capture_expr(subject, scope)),
            arms: arms
                .iter()
                .map(|arm| {
                    let mut local = scope.clone();
                    match &arm.pattern {
                        ResultPattern::Ok(name) | ResultPattern::Err(name) => {
                            local.remove(name);
                        }
                        ResultPattern::Wildcard => {}
                    }
                    ResultArm {
                        pattern: arm.pattern.clone(),
                        body: capture_expr(&arm.body, &local),
                    }
                })
                .collect(),
        },
        ExprKind::Binary(op, l, r) => ExprKind::Binary(
            op.clone(),
            Box::new(capture_expr(l, scope)),
            Box::new(capture_expr(r, scope)),
        ),
        ExprKind::If(c, y, n) => ExprKind::If(
            Box::new(capture_expr(c, scope)),
            Box::new(capture_expr(y, scope)),
            Box::new(capture_expr(n, scope)),
        ),
        ExprKind::Call(call) => ExprKind::Call(Box::new(capture_call(call, scope))),
        ExprKind::Content(forest) => ExprKind::Content(
            forest
                .iter()
                .map(|i| capture_item(&Item::from_node(i), scope).recipe())
                .collect(),
        ),
        ExprKind::Block(bindings, value) => {
            let mut local = scope.clone();
            let bindings = bindings
                .iter()
                .map(|(name, value)| {
                    let value = capture_expr(value, &local);
                    local.remove(name);
                    (name.clone(), value)
                })
                .collect();
            ExprKind::Block(bindings, Box::new(capture_expr(value, &local)))
        }
        other => other.clone(),
    };
    TypedExpr {
        ty: expr.ty.clone(),
        known,
        kind,
        span: expr.span,
        source: expr.source.clone(),
        shape: expr.shape,
    }
}

/// Copy one boundary, never children or the pending computation being rebuilt.
fn item_shell(item: &Item) -> Item {
    Item {
        ctor: item.ctor.clone(),
        pending: None,
        instance: item.instance.clone(),
        target: item.target.clone(),
        evaluation: item.evaluation.clone(),
        origin: item.origin.clone(),
        level: item.level,
        fields: item.fields.clone(),
        children: Vec::new(),
        attrs: item.attrs.clone(),
        span: item.span,
    }
}

fn failure_content(mut item: Item, failure: Failure) -> Item {
    if let Some(location) = failure.location {
        let mut expansion = failure.expansion;
        if let Some(origin) = &item.origin {
            let location = SourceLocation {
                source: origin.source.clone(),
                start: origin.start,
                end: origin.end,
            };
            if expansion.last() != Some(&location) {
                expansion.push(location);
            }
            expansion.extend(origin.expansion.iter().cloned());
        }
        item.span = rowan::TextRange::new(location.start.into(), location.end.into());
        item.origin = Some(notist_core::item::Origin {
            source: location.source,
            start: location.start,
            end: location.end,
            expansion: expansion.into(),
        });
    }
    error_content(item, failure.message)
}
fn error_content(mut item: Item, message: String) -> Item {
    item.evaluation = Some(EvaluationState::Error(message));
    item.pending = None;
    item.children.clear();
    item
}
fn content_count(forest: &[Item]) -> usize {
    forest
        .iter()
        .flat_map(Item::descendants)
        .filter(|i| !matches!(i.evaluation, Some(EvaluationState::Error(_))))
        .count()
}
fn valid_structure(item: &Item) -> bool {
    fn children(items: &[Item], expected: &Ctor) -> bool {
        items.iter().all(|item| {
            matches!(item.evaluation, Some(EvaluationState::Error(_)))
                || if matches!(item.ctor, Ctor::Node | Ctor::Group)
                    || item.is_evaluated_interpolation()
                {
                    children(&item.children, expected)
                } else {
                    &item.ctor == expected
                }
        })
    }
    match item.ctor.accepts() {
        Some(notist_core::builtins::Accepts::Rows) => children(&item.children, &Ctor::TableRow),
        Some(notist_core::builtins::Accepts::Cells) => children(&item.children, &Ctor::TableCell),
        Some(notist_core::builtins::Accepts::Items) => children(&item.children, &Ctor::ListItem),
        _ => true,
    }
}
fn expand(item: &mut Item, locations: &[SourceLocation]) {
    if locations.is_empty() {
        return;
    }
    use notist_core::source_chain::SourceChain;
    fn visit(
        item: &mut Item,
        locations: &[SourceLocation],
        chains: &mut std::collections::HashMap<usize, (SourceChain, SourceChain)>,
    ) {
        if let Some(origin) = &mut item.origin {
            // Storage identity only deduplicates an immutable append operation.
            // Retain the old chain too: allocator address reuse cannot alias keys.
            let key = origin.expansion.as_ptr() as usize;
            let (_, extended) = chains.entry(key).or_insert_with(|| {
                let original = origin.expansion.clone();
                let mut extended = original.clone();
                extended.extend_from_slice(locations);
                (original, extended)
            });
            origin.expansion = extended.clone();
        }
        for child in &mut item.children {
            visit(child, locations, chains);
        }
    }
    visit(item, locations, &mut std::collections::HashMap::new());
}
fn value_of(v: Runtime) -> Result<ComptimeValue, String> {
    match v {
        Runtime::Value(v) => Ok(v),
        _ => Err("expected a data or Type value".into()),
    }
}
fn data(v: Runtime) -> Result<Value, String> {
    match v {
        Runtime::Value(ComptimeValue::Data(v)) => Ok(v),
        _ => Err("expected a data value".into()),
    }
}

pub fn unary(op: &str, v: &ComptimeValue) -> Result<ComptimeValue, String> {
    let v = match (op, v) {
        ("!", ComptimeValue::Data(Value::Bool(v))) => Value::Bool(!v),
        ("-", ComptimeValue::Data(Value::Int(v))) => {
            Value::Int(v.checked_neg().ok_or("integer overflow")?)
        }
        ("-", ComptimeValue::Data(Value::Float(v))) => Value::Float(-v),
        _ => return Err("invalid unary operator".into()),
    };
    Ok(ComptimeValue::Data(v))
}
pub fn binary(op: &str, a: &ComptimeValue, b: &ComptimeValue) -> Result<ComptimeValue, String> {
    use ComptimeValue::{Data as D, Type as T};
    let boolean = |v| Ok(D(Value::Bool(v)));
    if let (T(a), T(b)) = (a, b) {
        return match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("Type only supports equality".into()),
        };
    }
    match (a, b) {
        (D(Value::Int(a)), D(Value::Int(b))) => {
            let v = match op {
                "+" => a.checked_add(*b),
                "-" => a.checked_sub(*b),
                "*" => a.checked_mul(*b),
                "/" => a.checked_div(*b),
                "%" => a.checked_rem(*b),
                "==" => return boolean(a == b),
                "!=" => return boolean(a != b),
                "<" => return boolean(a < b),
                "<=" => return boolean(a <= b),
                ">" => return boolean(a > b),
                ">=" => return boolean(a >= b),
                _ => return Err("invalid integer operator".into()),
            };
            Ok(D(Value::Int(
                v.ok_or("integer overflow or division by zero")?,
            )))
        }
        (D(Value::Float(a)), D(Value::Float(b))) => Ok(D(match op {
            "+" => Value::Float(a + b),
            "-" => Value::Float(a - b),
            "*" => Value::Float(a * b),
            "/" => Value::Float(a / b),
            "%" => Value::Float(a % b),
            "==" => Value::Bool(a == b),
            "!=" => Value::Bool(a != b),
            "<" => Value::Bool(a < b),
            "<=" => Value::Bool(a <= b),
            ">" => Value::Bool(a > b),
            ">=" => Value::Bool(a >= b),
            _ => return Err("invalid float operator".into()),
        })),
        (D(Value::Str(a)), D(Value::Str(b))) => match op {
            "+" => {
                if a.len().saturating_add(b.len()) > 16 * 1024 * 1024 {
                    return Err("value size limit exceeded".into());
                }
                Ok(D(Value::Str(format!("{a}{b}").into())))
            }
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            "<" => boolean(a < b),
            "<=" => boolean(a <= b),
            ">" => boolean(a > b),
            ">=" => boolean(a >= b),
            _ => Err("invalid string operator".into()),
        },
        (D(Value::Path(a)), D(Value::Path(b))) => match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("Path only supports equality".into()),
        },
        (D(Value::Bool(a)), D(Value::Bool(b))) => match op {
            "&&" => boolean(*a && *b),
            "||" => boolean(*a || *b),
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("invalid Bool operator".into()),
        },
        (D(Value::Some(a)), D(Value::Some(b))) => match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("Option only supports equality".into()),
        },
        (D(Value::None), D(Value::None)) => match op {
            "==" => boolean(true),
            "!=" => boolean(false),
            _ => Err("Option only supports equality".into()),
        },
        (D(Value::Some(_)), D(Value::None)) | (D(Value::None), D(Value::Some(_))) => match op {
            "==" => boolean(false),
            "!=" => boolean(true),
            _ => Err("Option only supports equality".into()),
        },
        (D(Value::Ok(a)), D(Value::Ok(b))) | (D(Value::Err(a)), D(Value::Err(b))) => match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("Result only supports equality".into()),
        },
        (D(Value::Ok(_)), D(Value::Err(_))) | (D(Value::Err(_)), D(Value::Ok(_))) => match op {
            "==" => boolean(false),
            "!=" => boolean(true),
            _ => Err("Result only supports equality".into()),
        },
        (D(Value::Data(a)), D(Value::Data(b))) => match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("Data only supports equality".into()),
        },
        (D(Value::DataError(a)), D(Value::DataError(b))) => match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("DataError only supports equality".into()),
        },
        _ => Err("operator requires matching scalar operands".into()),
    }
}

fn check_value_size(value: &ComptimeValue, limit: usize) -> Result<(), String> {
    if let ComptimeValue::Data(v) = value
        && v.logical_size() > limit
    {
        return Err("value size limit exceeded".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_append_shares_storage_without_merging_origins_or_reordering() {
        use notist_core::item::Origin;
        let location = |source: &str, start| SourceLocation {
            source: source.into(),
            start,
            end: start + 1,
        };
        let mut first = Item::new(Ctor::Text, Default::default());
        first.origin = Some(Origin {
            source: "definition.notc".into(),
            start: 1,
            end: 2,
            expansion: vec![location("inner.notc", 3)].into(),
        });
        let mut different = first.clone();
        different.origin.as_mut().unwrap().expansion[0].start = 8;
        let mut root = Item::new(Ctor::Node, Default::default()).with_children(vec![
            first.clone(),
            first.clone(),
            different,
        ]);
        expand(&mut root, &[location("outer.not", 5)]);
        let chains: Vec<_> = root
            .children
            .iter()
            .map(|item| &item.origin.as_ref().unwrap().expansion)
            .collect();
        assert!(chains[0].shares_storage(chains[1]));
        assert!(!chains[0].shares_storage(chains[2]));
        assert_eq!(
            chains[0]
                .iter()
                .map(|location| (location.source.as_str(), location.start))
                .collect::<Vec<_>>(),
            [("inner.notc", 3), ("outer.not", 5)]
        );
        assert_eq!(chains[2][0].start, 8);
        assert_eq!(first.origin.as_ref().unwrap().expansion.len(), 1);
        root.children[0].origin.as_mut().unwrap().expansion.clear();
        assert_eq!(root.children[1].origin.as_ref().unwrap().expansion.len(), 2);
    }

    #[test]
    fn first_node_and_unchanged_boundaries_need_no_call_backup() {
        let original = Item::text("before".into(), Default::default());
        let mut checkpoint = PresentCheckpoint::new(false);
        checkpoint.before_call(&original);
        assert!(checkpoint.call_fields.is_none());
        let mut checkpoint = PresentCheckpoint::new(true);
        checkpoint.before_call(&original);
        let changed = Item::text("after".into(), Default::default());
        let restored = checkpoint.restore(changed);
        assert_eq!(restored.fields, original.fields);
        assert_eq!(restored.target, original.target);
    }

    #[test]
    fn both_result_payloads_count_towards_the_value_size_limit() {
        for value in [
            Value::Ok(Box::new(Value::Str("x".repeat(16 * 1024 * 1024).into()))),
            Value::Err(Box::new(Value::Str("x".repeat(16 * 1024 * 1024).into()))),
        ] {
            assert_eq!(
                check_value_size(&ComptimeValue::Data(value), 16 * 1024 * 1024).unwrap_err(),
                "value size limit exceeded",
            );
        }
    }
}
