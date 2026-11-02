use super::*;
use std::sync::Arc;
#[path = "data.rs"]
mod data;
use crate::Context;
use notist_core::{
    builtins::Level,
    definitions::{FunctionImpl, NativeFunction, ParameterMode},
    diag::Phase,
    evaluation::{CallSite, ResultArm, ResultPattern},
    inputs::{EvaluationInputs, InputError, InputRead},
    item::{Ctor, EvaluationState, SourceLocation, Value},
    resource::ResourcePath,
};

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
    pub root: Item,
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
pub(super) struct Failure {
    pub(super) message: String,
    pub(super) location: Option<SourceLocation>,
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
                source: call.source.clone(),
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
struct Evaluator<'a> {
    context: &'a Context,
    program: CheckedProgram,
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

pub(crate) fn evaluate(
    root: &Item,
    program: &CheckedProgram,
    context: &Context,
    limits: Limits,
) -> Evaluation {
    match run(root, program, context, limits, None) {
        EvaluationOutcome::Complete(evaluation) => *evaluation,
        EvaluationOutcome::NeedResources(_) => {
            unreachable!("a missing input capability never requests preparation")
        }
    }
}
pub(crate) fn run(
    root: &Item,
    program: &CheckedProgram,
    context: &Context,
    limits: Limits,
    inputs: Option<&dyn EvaluationInputs>,
) -> EvaluationOutcome {
    let mut e = Evaluator {
        context,
        program: program.clone(),
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
    let forest = e.bind_content(
        std::slice::from_ref(root),
        &Scope::new(),
        limits.content_nodes,
    );
    let mut roots = e.present(&forest, limits.content_nodes);
    if !e.needed.is_empty() {
        return EvaluationOutcome::NeedResources(e.needed.into_iter().collect());
    }
    for diagnostic in &mut e.diagnostics {
        diagnostic.recovered = true;
    }
    EvaluationOutcome::Complete(Box::new(Evaluation {
        root: roots.remove(0),
        diagnostics: e.diagnostics,
        dependencies: e.dependencies,
        sources: e.sources,
    }))
}
pub(super) fn constant(
    t: &TypedExpr,
    program: &CheckedProgram,
    context: &Context,
) -> Result<ComptimeValue, Failure> {
    let mut e = Evaluator {
        context,
        program: program.clone(),
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
impl Evaluator<'_> {
    fn target(
        &self,
        id: &notist_core::definitions::FunctionId,
    ) -> notist_core::definitions::FunctionId {
        if self.mode == EvaluationMode::Runtime {
            self.context.extensions().target(id)
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
                    Arc::new(self.bind_content(forest, scope, self.limits.content_nodes)),
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
                    source: t.source.clone(),
                    start: t.span.start().into(),
                    end: t.span.end().into(),
                    expansion: Vec::new(),
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
            ExprKind::ValueCall(call) => {
                let children = match &call.children {
                    Some(child) => match self.expr(child, scope)? {
                        Runtime::Content(_, nodes) => nodes.as_ref().clone(),
                        _ => return Err("invalid children".into()),
                    },
                    None => Vec::new(),
                };
                self.execute(call, scope, children, None)
            }
        }?;
        if let Runtime::Value(v) = &result {
            check_value_size(v, self.limits.value_bytes)?;
        }
        Ok(result)
    }
    /// Bind a content value without executing opaque content calls. The result
    /// remains an Item forest; only mounted Items are expanded by `present`.
    fn bind_content(&mut self, forest: &[Item], scope: &Scope, limit: usize) -> Vec<Item> {
        let mut result = Vec::new();
        let mut count = 0usize;
        for node in forest {
            let nodes = match node.pending.as_deref() {
                Some(PendingEvaluation::Failure { message, .. }) => {
                    vec![error_content(node.clone(), message.clone())]
                }
                Some(PendingEvaluation::Splice(expr)) => match self.expr(expr, scope) {
                    Ok(Runtime::Content(ty, nodes))
                        if expr.ty.as_ref() == Some(&ty) && ty.level() == Some(node.level) =>
                    {
                        let mut group = node.clone();
                        group.pending = None;
                        group.evaluation = Some(EvaluationState::Expanded);
                        group.children = nodes.as_ref().clone();
                        vec![group]
                    }
                    Ok(Runtime::Content(..)) => vec![error_content(
                        node.clone(),
                        "content interpolation violated its checked category".into(),
                    )],
                    Ok(_) => vec![error_content(
                        node.clone(),
                        "content interpolation produced a data value".into(),
                    )],
                    Err(failure) => vec![failure_content(node.clone(), failure)],
                },
                pending => {
                    let mut item = node.clone();
                    if let Some(PendingEvaluation::Call(call)) = pending {
                        item.pending =
                            Some(Box::new(PendingEvaluation::Call(capture_call(call, scope))));
                    }
                    if let Some(PendingEvaluation::Repeat(expr)) = pending {
                        item.pending = Some(Box::new(PendingEvaluation::Repeat(capture_expr(
                            expr, scope,
                        ))));
                    }
                    item.children =
                        self.bind_content(&node.children, scope, limit.saturating_sub(1));
                    vec![item]
                }
            };
            let added = content_count(&nodes);
            if count.saturating_add(added) > limit {
                result.push(error_content(
                    node.clone(),
                    "content size limit exceeded".into(),
                ));
                continue;
            }
            count += added;
            result.extend(nodes);
        }
        result
    }
    fn present(&mut self, forest: &[Item], limit: usize) -> Vec<Item> {
        let mut result = Vec::new();
        let mut count = 0usize;
        for node in forest {
            let nodes = self.present_node(node, limit);
            let added = nodes
                .iter()
                .flat_map(Item::descendants)
                .filter(|i| !matches!(i.evaluation, Some(EvaluationState::Error(_))))
                .count();
            if count.saturating_add(added) > limit {
                result.push(self.error(node.clone(), "content size limit exceeded".into()));
                continue;
            }
            count += added;
            result.extend(nodes);
        }
        result
    }
    fn present_node(&mut self, item: &Item, limit: usize) -> Vec<Item> {
        if limit == 0 && !matches!(item.evaluation, Some(EvaluationState::Error(_))) {
            return vec![self.error(item.clone(), "content size limit exceeded".into())];
        }
        if let Some(EvaluationState::Error(message)) = &item.evaluation {
            return vec![self.error(item.clone(), message.clone())];
        }
        if let Some(PendingEvaluation::Call(call)) = item.pending.as_deref() {
            return self.present_call(item, call, limit);
        }
        if let Some(PendingEvaluation::Repeat(expr)) = item.pending.as_deref() {
            return match self.repeat(expr, limit.saturating_sub(1), item) {
                Ok(children) => {
                    let mut group = item.clone();
                    group.pending = None;
                    group.evaluation = Some(EvaluationState::Expanded);
                    group.children = children;
                    vec![group]
                }
                Err(failure) => vec![self.error_at(item.clone(), failure)],
            };
        }
        if item.evaluation.is_none()
            && let Some(id) = item.function_id()
            && self.target(&id) != id
            && let Some(call) = self.synthetic_call(item)
        {
            return self.present_call(item, &call, limit);
        }
        let mut item = item.clone();
        item.pending = None;
        item.children = self.present(&item.children, limit.saturating_sub(1));
        if !valid_structure(&item) {
            return vec![self.error(
                item,
                "evaluated content violated its checked structural role".into(),
            )];
        }
        vec![item]
    }
    fn repeat(
        &mut self,
        expr: &TypedExpr,
        limit: usize,
        group: &Item,
    ) -> Result<Vec<Item>, Failure> {
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
            let mut count = 0usize;
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
                let mut nodes = nodes.as_ref().clone();
                if let Some(origin) = &group.origin {
                    for node in &mut nodes {
                        expand(node, &origin.expansion);
                    }
                }
                let remaining = limit.saturating_sub(count);
                let nodes = self.present(&nodes, remaining);
                let added = content_count(&nodes);
                if count.saturating_add(added) > limit {
                    return Err("content size limit exceeded".into());
                }
                // A failed iteration never exposes a successfully assembled prefix.
                if nodes
                    .iter()
                    .flat_map(Item::descendants)
                    .any(|n| matches!(n.evaluation, Some(EvaluationState::Error(_))))
                {
                    return Err(format!("for iteration {index} failed").into());
                }
                count += added;
                result.extend(nodes);
            }
            Ok(result)
        })();
        self.active_loops -= 1;
        result
    }
    fn present_call(&mut self, item: &Item, call: &Invocation, limit: usize) -> Vec<Item> {
        let target = self.target(&call.instance.function);
        let expands = self
            .context
            .registry()
            .get(&target)
            .is_some_and(|d| d.implementation.is_code());
        if expands && self.depth >= self.limits.call_depth.min(32) {
            return vec![self.error(item.clone(), "evaluation call-depth limit exceeded".into())];
        }
        if expands {
            self.depth += 1;
        }
        // Each mounted content boundary owns its computation budget.
        let previous_steps = std::mem::replace(&mut self.steps, 0);
        let result = self.execute(
            call,
            &Scope::new(),
            item.children.clone(),
            Some(item.clone()),
        );
        let result = match result {
            Ok(Runtime::Content(_, nodes)) => self.present(&nodes, limit),
            Ok(_) => vec![self.error(item.clone(), "content call produced a data value".into())],
            Err(failure) => vec![self.error_at(item.clone(), failure)],
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
        let instance = item.instance.clone().unwrap_or_else(|| FunctionInstance {
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
            .map(|o| o.source.clone())
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
            definition: Box::new(def),
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
        d.source = item.origin.as_ref().map(|o| o.source.clone());
        self.diagnostics.push(d);
        if let Some(instance) = &item.instance {
            item.target = Some(FunctionInstance {
                function: self.target(&instance.function),
                arguments: instance.arguments.clone(),
            });
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
        self.error(item.clone(), message.clone())
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
        prototype: Option<Item>,
    ) -> Result<Runtime, Failure> {
        self.tick()?;
        if let Some(m) = &call.invalid {
            return Err(m.clone().into());
        }
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
                Runtime::Content(ty.clone(), Arc::new(children.clone())),
            );
        }
        let target = FunctionInstance {
            function: self.target(&call.instance.function),
            arguments: call.instance.arguments.clone(),
        };
        let (definition, body) = self.implementation(&target)?;
        for (param, value) in definition.comptime.iter().zip(&target.arguments) {
            inputs.insert(param.name.clone(), Runtime::Value(value.clone()));
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
            FunctionImpl::Code => match body {
                Some(body) => self.expr(&body, &inputs),
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
                        Arc::new(children),
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
                    source: call.source.clone(),
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
            let mut item = prototype.ok_or("missing materialized content call boundary")?;
            item.fields = fields;
            item.pending = None;
            item.target = Some(target);
            let native =
                definition.implementation == FunctionImpl::Native(NativeFunction::Construct);
            item.evaluation = Some(if native {
                EvaluationState::Native
            } else {
                EvaluationState::Expanded
            });
            if !native {
                let mut expansion = vec![SourceLocation {
                    source: call.source.clone(),
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
    ) -> Result<(FunctionDef, Option<TypedExpr>), Failure> {
        if instance
            .arguments
            .iter()
            .any(|v| matches!(v, ComptimeValue::Data(v) if v.contains_data_views()))
        {
            return Err("Data and DataError views cannot be comptime value arguments".into());
        }
        let key = instance.key();
        if !self.program.instances.contains_key(&key) {
            // Runtime specializations share the same retained backing-store
            // budget, including comptime views in currently unselected arms.
            self.program.data_retention = self.data_retention.clone();
            let mut checker = super::check::Checker::with_program(
                self.context,
                std::mem::take(&mut self.program),
            );
            checker.install(instance);
            self.data_retention = checker.program.data_retention.clone();
            self.diagnostics.extend(checker.diagnostics);
            self.program = checker.program;
        }
        let record = self
            .program
            .instances
            .get(&key)
            .ok_or("missing function instance")?;
        if let Some(d) = record.diagnostics.first() {
            return Err(format!("{}: {}", instance.function, d.message).into());
        }
        Ok((record.definition.clone(), record.body.clone()))
    }
}
// Capture only free inputs in pending expressions. Content calls remain opaque;
// substitution performs no interpretation, including in unused branches.
fn capture_call(call: &Invocation, scope: &Scope) -> Invocation {
    let mut call = call.clone();
    for (_, expr) in &mut call.arguments {
        *expr = capture_expr(expr, scope);
    }
    call.children = call
        .children
        .as_ref()
        .map(|e| Box::new(capture_expr(e, scope)));
    call
}
fn capture_item(item: &Item, scope: &Scope) -> Item {
    let mut item = item.clone();
    item.pending = item.pending.as_deref().map(|pending| {
        Box::new(match pending {
            PendingEvaluation::Call(call) => PendingEvaluation::Call(capture_call(call, scope)),
            PendingEvaluation::Splice(expr) => PendingEvaluation::Splice(capture_expr(expr, scope)),
            PendingEvaluation::Repeat(expr) => PendingEvaluation::Repeat(capture_expr(expr, scope)),
            failure => failure.clone(),
        })
    });
    item.children = item
        .children
        .iter()
        .map(|i| capture_item(i, scope))
        .collect();
    item
}
fn capture_expr(expr: &TypedExpr, scope: &Scope) -> TypedExpr {
    let mut expr = expr.clone();
    if expr.known.is_some() {
        return expr;
    }
    expr.kind = match &expr.kind {
        ExprKind::Input(name) => match scope.get(name) {
            Some(Runtime::Value(v)) => {
                expr.known = Some(v.clone());
                ExprKind::Constant(v.clone())
            }
            Some(Runtime::Content(_, forest)) => ExprKind::Content(forest.as_ref().clone()),
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
        ExprKind::ValueCall(call) => ExprKind::ValueCall(Box::new(capture_call(call, scope))),
        ExprKind::Content(forest) => {
            ExprKind::Content(forest.iter().map(|i| capture_item(i, scope)).collect())
        }
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
    expr
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
            expansion,
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
                || if item.ctor == Ctor::Node || item.is_evaluated_interpolation() {
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
    if let Some(origin) = &mut item.origin {
        origin.expansion.extend_from_slice(locations);
    }
    for child in &mut item.children {
        expand(child, locations);
    }
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

pub(super) fn unary(op: &str, v: &ComptimeValue) -> Result<ComptimeValue, String> {
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
pub(super) fn binary(
    op: &str,
    a: &ComptimeValue,
    b: &ComptimeValue,
) -> Result<ComptimeValue, String> {
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
    use crate::Pipeline;

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

    #[test]
    fn evaluation_reads_the_supplied_item_tree_without_a_parallel_representation() {
        let context = Context::default()
            .with_module(
                Pipeline::default()
                    .compile_module(
                        "content.not",
                        "demo",
                        "demo/lib.notc",
                        "fn greet(name: String) -> InlineContent { [#text(text: name)] }",
                    )
                    .unwrap(),
            )
            .unwrap();
        let pipeline = Pipeline::default();
        let analysis = pipeline
            .analyze("doc.not", "#self.greet(\"before\")", &context)
            .unwrap();
        let mut root = analysis.root().clone();
        let call = root.children[0].children[0].pending.as_deref_mut().unwrap();
        let PendingEvaluation::Call(call) = call else {
            panic!("expected a pending call")
        };
        let argument = &mut call.arguments[0].1;
        argument.known = Some(ComptimeValue::Data(Value::Str("after".into())));
        argument.kind = ExprKind::Constant(argument.known.clone().unwrap());
        // Only the Item and reusable Context are needed; instances can be checked on demand.
        let result = evaluate(
            &root,
            &CheckedProgram::default(),
            &context,
            Limits::default(),
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result
                .root
                .descendants()
                .any(|i| i.fields.get("text") == Some(&Value::Str("after".into())))
        );
        assert!(result.root.descendants().all(|i| i.pending.is_none()));
        assert_eq!(
            analysis.root().children[0].children[0].fields.get("name"),
            Some(&Value::Str("before".into()))
        );
    }

    #[test]
    fn checked_function_bodies_contain_typed_item_interpolations() {
        let context = Context::default()
            .with_module(
                Pipeline::default()
                    .compile_module(
                        "content.not",
                        "demo",
                        "demo/lib.notc",
                        "fn pass()[children: InlineContent] -> InlineContent { [#children] }",
                    )
                    .unwrap(),
            )
            .unwrap();
        let pipeline = Pipeline::default();
        let analysis = pipeline
            .analyze("doc.not", "#self.pass[one]", &context)
            .unwrap();
        let body = analysis
            .program
            .instances
            .values()
            .find(|r| r.definition.id.module == "demo")
            .unwrap()
            .body
            .as_ref()
            .unwrap();
        let ExprKind::Block(_, value) = &body.kind else {
            panic!("expected function body")
        };
        let ExprKind::Content(forest) = &value.kind else {
            panic!("expected Item forest")
        };
        assert_eq!(forest[0].ctor, notist_core::item::Ctor::Interpolation);
        let Some(PendingEvaluation::Splice(expr)) = forest[0].pending.as_deref() else {
            panic!("expected interpolation")
        };
        assert_eq!(expr.ty, Some(Type::InlineContent));
        assert!(matches!(&expr.kind, ExprKind::Input(name) if name == "children"));
        let evaluation = pipeline.evaluate(&analysis, &context);
        assert!(evaluation.diagnostics.is_empty());
        let pass = evaluation
            .root
            .find(|i| {
                i.function_id() == Some(notist_core::definitions::FunctionId::new("demo", "pass"))
            })
            .unwrap();
        assert_eq!(pass.children.len(), 1);
        assert!(pass.children[0].is_evaluated_interpolation());
        assert_eq!(
            pass.children[0].children[0].fields.get("text"),
            Some(&Value::Str("one".into()))
        );
        assert!(evaluation.root.descendants().all(|i| i.pending.is_none()));
    }
}
