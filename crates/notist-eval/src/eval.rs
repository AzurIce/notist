use crate::{EvaluationContext, instance_cache::InstanceCache};
use notist_core::{
    builtins::Level,
    definitions::{
        ComptimeValue, FunctionImpl, NativeFunction, ParameterMode,
        ResolvedInstance as FunctionInstance, Type,
    },
    diag::{Diagnostic, Phase},
    evaluation::{CallSite, CheckedProgram, ExprKind, InstanceRecord, Invocation, TypedExpr},
    inputs::{EvaluationInputs, InputError, InputRead},
    item::{ContentItem, Ctor, Dict, Node, NodeContent, Origin, SourceLocation, Value},
    resource::ResourcePath,
    shared::Shared,
    source_chain::SourceChain,
};
use std::{collections::BTreeMap, sync::Arc};

#[path = "data.rs"]
mod data;
#[path = "operators.rs"]
mod operators;
pub use operators::{binary, unary};
#[cfg(feature = "test-support")]
#[path = "test_support.rs"]
#[doc(hidden)]
pub mod test_support;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub steps: usize,
    pub iterations: usize,
    pub loop_work: usize,
    /// Logical content work, including mounts which dissolve in the output.
    /// Recovery placeholders consume no normal-node budget.
    pub content_nodes: usize,
    pub call_depth: usize,
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
    pub sources: notist_core::diag::Sources,
}
#[derive(Debug, Clone, PartialEq)]
pub enum EvaluationOutcome {
    Complete(Box<Evaluation>),
    /// No partial tree or output hooks escape an unprepared evaluation.
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
    sources: notist_core::diag::Sources,
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            message,
            location: None,
            expansion: Vec::new(),
            sources: Default::default(),
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
    pub fn sources(&self) -> &notist_core::diag::Sources {
        &self.sources
    }
    fn at(message: impl Into<String>, call: &Invocation) -> Self {
        Self {
            message: message.into(),
            location: Some(location(call)),
            expansion: Vec::new(),
            sources: Default::default(),
        }
    }
}

/// Immutable lexical snapshots. Saving content shares the environment, never
/// substitutes values into checked code. A subsequent binding detaches the map.
#[derive(Debug, Clone, Default)]
struct Scope(Shared<BTreeMap<notist_core::binding::BindingId, Runtime>>);
impl Scope {
    fn new() -> Self {
        Self::default()
    }
    fn get(&self, id: &notist_core::binding::BindingId) -> Option<&Runtime> {
        self.0.get(id)
    }
    fn insert(&mut self, id: notist_core::binding::BindingId, value: Runtime) {
        self.0.insert(id, value);
    }
    fn contains_key(&self, id: &notist_core::binding::BindingId) -> bool {
        self.0.contains_key(id)
    }
}
#[derive(Debug, Clone)]
enum Runtime {
    Value(ComptimeValue),
    Content(Type, Content),
}

/// A content value is a reusable construction plan, not an execution-state
/// tree. Calls and loops pair shared code with a lexical environment. Splices
/// retain the value computed at binding time. Only mount emits public Nodes.
#[derive(Debug, Clone, Default)]
struct Content {
    pieces: Shared<Vec<Piece>>,
    nodes: usize,
}
#[derive(Debug, Clone)]
struct Piece {
    source: Node,
    kind: Construction,
}
#[derive(Debug, Clone)]
enum Construction {
    /// An already concrete, valid subtree shares its child storage verbatim.
    Ready {
        nodes: usize,
    },
    Element(Content),
    Call {
        call: Shared<Invocation>,
        environment: Scope,
        children: Content,
    },
    Repeat {
        expression: Shared<TypedExpr>,
        environment: Scope,
    },
    Splice(Content),
    Error,
}
impl Content {
    fn one(piece: Piece, nodes: usize) -> Self {
        Self {
            pieces: vec![piece].into(),
            nodes,
        }
    }
}
#[derive(Clone, Copy, Default, Debug)]
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
impl<'a, C: EvaluationContext + ?Sized> Evaluator<'a, C> {
    fn new(
        context: &'a C,
        program: &CheckedProgram,
        sources: &notist_core::diag::Sources,
        limits: Limits,
        inputs: Option<&'a dyn EvaluationInputs>,
        mode: EvaluationMode,
    ) -> Self {
        Self {
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
            mode,
            inputs,
            requests: 0,
            dependencies: Default::default(),
            needed: Default::default(),
            sources: sources.clone(),
            data_retention: program.data_retention.clone(),
        }
    }
}
pub fn evaluate<C: EvaluationContext + ?Sized>(
    root: &Node,
    program: &CheckedProgram,
    sources: &notist_core::diag::Sources,
    context: &C,
    limits: Limits,
) -> Evaluation {
    match run(root, program, sources, context, limits, None) {
        EvaluationOutcome::Complete(value) => *value,
        EvaluationOutcome::NeedResources(_) => {
            unreachable!("missing capability never requests preparation")
        }
    }
}
pub fn run<C: EvaluationContext + ?Sized>(
    root: &Node,
    program: &CheckedProgram,
    sources: &notist_core::diag::Sources,
    context: &C,
    limits: Limits,
    inputs: Option<&dyn EvaluationInputs>,
) -> EvaluationOutcome {
    let mut source_view = sources
        .clone()
        .with_definitions(context.registry().identities());
    if let Err(error) = source_view
        .merge(&program.sources)
        .and_then(|()| source_view.validate_node(root))
        .and_then(|()| source_view.validate_program(program))
    {
        return EvaluationOutcome::Complete(Box::new(Evaluation {
            root: Node::new(Ctor::Error(error.to_string()), root.span),
            diagnostics: vec![Diagnostic::new(Phase::Type, root.span, error.to_string())],
            dependencies: Default::default(),
            sources: source_view,
        }));
    }
    let mut e = Evaluator::new(
        context,
        program,
        &source_view,
        limits,
        inputs,
        EvaluationMode::Runtime,
    );
    let content = e.bind_content(
        std::slice::from_ref(root),
        &Scope::new(),
        limits.content_nodes,
    );
    let (mut nodes, _) = e.present(&content, limits.content_nodes, &SourceChain::default());
    if !e.needed.is_empty() {
        return EvaluationOutcome::NeedResources(e.needed.into_iter().collect());
    }
    for diagnostic in &mut e.diagnostics {
        diagnostic.recovered = true;
    }
    let root = if nodes.len() == 1 {
        nodes.remove(0)
    } else {
        Node::new(Ctor::Group, root.span).with_children(nodes)
    };
    EvaluationOutcome::Complete(Box::new(Evaluation {
        root,
        diagnostics: e.diagnostics,
        dependencies: e.dependencies,
        sources: e.sources,
    }))
}
pub fn constant<C: EvaluationContext + ?Sized>(
    t: &TypedExpr,
    program: &CheckedProgram,
    sources: &notist_core::diag::Sources,
    context: &C,
) -> Result<ComptimeValue, Failure> {
    let mut source_view = sources
        .clone()
        .with_definitions(context.registry().identities());
    source_view
        .merge(&program.sources)
        .map_err(|error| Failure::from(error.to_string()))?;
    source_view
        .validate_expression(t)
        .and_then(|()| source_view.validate_program(program))
        .map_err(|error| Failure::from(error.to_string()))?;
    let mut e = Evaluator::new(
        context,
        program,
        &source_view,
        Limits::default(),
        None,
        EvaluationMode::Comptime,
    );
    let value = e.expr(t, &Scope::new()).map_err(|mut failure| {
        failure.sources = e.sources.clone();
        failure
    })?;
    match value {
        Runtime::Value(value) => Ok(value),
        Runtime::Content(..) => Err("content cannot be a comptime argument".into()),
    }
}
impl<C: EvaluationContext + ?Sized> Evaluator<'_, C> {
    fn target(&self, id: &notist_core::definitions::DefId) -> notist_core::definitions::DefId {
        if self.mode == EvaluationMode::Runtime {
            self.context.target_resolved(*id)
        } else {
            *id
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
        if let Some(value) = &t.known {
            check_value_size(value, self.limits.value_bytes)?;
            if let ComptimeValue::Data(value) = value {
                self.data_retention
                    .observe(value, self.limits.data.working_bytes)
                    .map_err(|error| Failure::from(error.to_string()))?;
            }
            return Ok(Runtime::Value(value.clone()));
        }
        let result =
            match &t.kind {
                ExprKind::Constant(value) => Runtime::Value(value.clone()),
                ExprKind::Input(id) => scope.get(id).cloned().ok_or_else(|| {
                    match self.sources.bindings().get(*id) {
                        Ok(info) => Failure::from(format!(
                            "runtime input `{}` is unavailable at compile time",
                            info.name
                        )),
                        Err(message) => Failure::from(message),
                    }
                })?,
                ExprKind::Failure(message) => return Err(message.clone().into()),
                ExprKind::Content(nodes) => {
                    self.require_runtime()?;
                    Runtime::Content(
                        t.ty.clone().ok_or("missing content type")?,
                        self.bind_content(nodes, scope, self.limits.content_nodes),
                    )
                }
                ExprKind::For { .. } => {
                    self.require_runtime()?;
                    let node = Node::expression(t.clone());
                    Runtime::Content(
                        t.ty.clone().ok_or("missing checked for result category")?,
                        Content::one(
                            Piece {
                                source: node,
                                kind: Construction::Repeat {
                                    expression: t.clone().into(),
                                    environment: scope.clone(),
                                },
                            },
                            1,
                        ),
                    )
                }
                ExprKind::Call(call) if call.definition.returns.level().is_some() => {
                    self.require_runtime()?;
                    let node = Node::expression(t.clone());
                    let content = self.bind_content(
                        std::slice::from_ref(&node),
                        scope,
                        self.limits.content_nodes,
                    );
                    Runtime::Content(call.definition.returns.clone(), content)
                }
                ExprKind::Call(call) => {
                    let children = self.call_children(call, scope, self.limits.content_nodes);
                    self.execute(call, scope, children, None)?
                }
                ExprKind::Block(bindings, tail) => {
                    let mut local = scope.clone();
                    for (id, value) in bindings {
                        let value = self.expr(value, &local)?;
                        local.insert(*id, value);
                    }
                    self.expr(tail, &local)?
                }
                ExprKind::If(condition, yes, no) => match data(self.expr(condition, scope)?)? {
                    Value::Bool(value) => self.expr(if value { yes } else { no }, scope)?,
                    _ => return Err("condition must be Bool".into()),
                },
                ExprKind::Unary(op, value) => {
                    Runtime::Value(unary(op, &value_of(self.expr(value, scope)?)?)?)
                }
                ExprKind::Binary(op, left, right) => {
                    let left = value_of(self.expr(left, scope)?)?;
                    if op == "&&" && left == ComptimeValue::Data(Value::Bool(false))
                        || op == "||" && left == ComptimeValue::Data(Value::Bool(true))
                    {
                        return Ok(Runtime::Value(left));
                    }
                    Runtime::Value(binary(op, &left, &value_of(self.expr(right, scope)?)?)?)
                }
                ExprKind::Array(values) => {
                    let values = values
                        .iter()
                        .map(|v| {
                            self.expr(v, scope)
                                .and_then(|v| data(v).map_err(Into::into))
                        })
                        .collect::<Result<_, _>>()?;
                    Runtime::Value(ComptimeValue::Data(Value::Array(values)))
                }
                ExprKind::Dict(values) => {
                    let values = values
                        .iter()
                        .map(|(k, v)| Ok((k.clone(), data(self.expr(v, scope)?)?)))
                        .collect::<Result<_, Failure>>()?;
                    Runtime::Value(ComptimeValue::Data(Value::Dict(values)))
                }
                ExprKind::Some(value) => Runtime::Value(ComptimeValue::Data(Value::Some(
                    Box::new(data(self.expr(value, scope)?)?),
                ))),
                ExprKind::Unwrap(value) => match data(self.expr(value, scope)?)? {
                    Value::Some(value) => Runtime::Value(ComptimeValue::Data(*value)),
                    _ => return Err("match arm requires `some`".into()),
                },
                ExprKind::Ok(value) | ExprKind::Err(value) => {
                    let value = Box::new(data(self.expr(value, scope)?)?);
                    Runtime::Value(ComptimeValue::Data(if matches!(t.kind, ExprKind::Ok(_)) {
                        Value::Ok(value)
                    } else {
                        Value::Err(value)
                    }))
                }
                ExprKind::MatchResult { subject, arms } => {
                    let (ok, value) = match data(self.expr(subject, scope)?)? {
                        Value::Ok(value) => (true, value),
                        Value::Err(value) => (false, value),
                        _ => return Err("Result match requires a Result value".into()),
                    };
                    let arm = arms.iter().find(|a| matches!(&a.pattern,
                    notist_core::evaluation::ResultPattern::Ok(_) if ok)
                    || matches!(&a.pattern, notist_core::evaluation::ResultPattern::Err(_) if !ok)
                    || matches!(&a.pattern, notist_core::evaluation::ResultPattern::Wildcard))
                    .ok_or("non-exhaustive Result match")?;
                    let mut local = scope.clone();
                    if let notist_core::evaluation::ResultPattern::Ok(name)
                    | notist_core::evaluation::ResultPattern::Err(name) = &arm.pattern
                    {
                        if self
                            .sources
                            .bindings()
                            .get(*name)
                            .map_err(Failure::from)?
                            .name
                            != "_"
                        {
                            local.insert(*name, Runtime::Value(ComptimeValue::Data(*value)));
                        }
                    }
                    self.expr(&arm.body, &local)?
                }
            };
        if let Runtime::Value(value) = &result {
            check_value_size(value, self.limits.value_bytes)?;
        }
        Ok(result)
    }
    fn require_runtime(&self) -> Result<(), Failure> {
        if self.mode == EvaluationMode::Comptime {
            Err("content cannot be constructed at compile time".into())
        } else {
            Ok(())
        }
    }
    fn call_children(&mut self, call: &Invocation, scope: &Scope, limit: usize) -> Content {
        match call.children.as_deref() {
            Some(TypedExpr {
                kind: ExprKind::Content(nodes),
                ..
            }) => self.bind_content(nodes, scope, limit),
            Some(expression) => {
                self.bind_content(&[Node::expression(expression.clone())], scope, limit)
            }
            None => Content::default(),
        }
    }
    fn bind_content(&mut self, nodes: &[Node], scope: &Scope, limit: usize) -> Content {
        let mut pieces = Vec::with_capacity(nodes.len());
        let mut count = 0usize;
        for node in nodes {
            if !self.context.has_mappings()
                && let Some(added) = ready_count(node)
                && count.saturating_add(added) <= limit
            {
                pieces.push(Piece {
                    source: node.clone(),
                    kind: Construction::Ready { nodes: added },
                });
                count += added;
                continue;
            }
            let (kind, added) = match &node.content {
                NodeContent::Item(item) if matches!(item.ctor, Ctor::Error(_)) => {
                    (Construction::Error, 0)
                }
                NodeContent::Item(item) => {
                    let children =
                        self.bind_content(&item.children, scope, limit.saturating_sub(1));
                    let added = 1 + children.nodes;
                    let call = if node.function_id().is_some_and(|id| self.target(&id) != id) {
                        match self.synthetic_call(node) {
                            Ok(call) => call,
                            Err(failure) => {
                                pieces.push(Piece {
                                    source: failure_content(node, failure),
                                    kind: Construction::Error,
                                });
                                continue;
                            }
                        }
                    } else {
                        None
                    };
                    (
                        match call {
                            Some(call) => Construction::Call {
                                call: call.into(),
                                environment: scope.clone(),
                                children,
                            },
                            None => Construction::Element(children),
                        },
                        added,
                    )
                }
                NodeContent::Expr(expression) => match &expression.kind {
                    ExprKind::Call(call) => {
                        let children = self.call_children(call, scope, limit.saturating_sub(1));
                        let added = 1 + children.nodes;
                        (
                            Construction::Call {
                                call: call.clone(),
                                environment: scope.clone(),
                                children,
                            },
                            added,
                        )
                    }
                    ExprKind::For { .. } => (
                        Construction::Repeat {
                            expression: expression.clone(),
                            environment: scope.clone(),
                        },
                        1,
                    ),
                    _ => match self.expr(expression, scope) {
                        Ok(Runtime::Content(ty, value))
                            if expression.ty.as_ref() == Some(&ty)
                                && ty.level() == Some(node.level) =>
                        {
                            let added = 1 + value.nodes;
                            (Construction::Splice(value), added)
                        }
                        Ok(Runtime::Content(..)) => {
                            pieces.push(error_piece(
                                node,
                                "content interpolation violated its checked category".into(),
                            ));
                            continue;
                        }
                        Ok(_) => {
                            pieces.push(error_piece(
                                node,
                                "content interpolation produced a data value".into(),
                            ));
                            continue;
                        }
                        Err(failure) => {
                            pieces.push(Piece {
                                source: failure_content(node, failure),
                                kind: Construction::Error,
                            });
                            continue;
                        }
                    },
                },
            };
            if count.saturating_add(added) > limit {
                pieces.push(error_piece(node, "content size limit exceeded".into()));
            } else {
                count += added;
                pieces.push(Piece {
                    source: node.clone(),
                    kind,
                });
            }
        }
        Content {
            pieces: pieces.into(),
            nodes: count,
        }
    }
    /// Emit final content directly. Aggregate rollback truncates only the output
    /// suffix; source templates and lexical closures are immutable throughout.
    fn present(
        &mut self,
        content: &Content,
        limit: usize,
        expansion: &SourceChain,
    ) -> (Vec<Node>, ContentSummary) {
        let mut result = Vec::with_capacity(content.pieces.len());
        let mut summary = ContentSummary::default();
        for piece in content.pieces.iter() {
            let start = result.len();
            let added = self.present_piece(piece, limit, expansion, &mut result);
            if summary.nodes.saturating_add(added.nodes) > limit {
                result.truncate(start);
                result.push(self.error(
                    &piece.source,
                    "content size limit exceeded".into(),
                    expansion,
                ));
                summary.failed = true;
            } else {
                summary.append(added);
            }
        }
        (result, summary)
    }
    fn present_piece(
        &mut self,
        piece: &Piece,
        limit: usize,
        expansion: &SourceChain,
        out: &mut Vec<Node>,
    ) -> ContentSummary {
        if matches!(piece.kind, Construction::Error) {
            let Ctor::Error(message) = piece.source.ctor() else {
                unreachable!()
            };
            out.push(self.error(&piece.source, message, expansion));
            return ContentSummary::ERROR;
        }
        if limit == 0 {
            out.push(self.error(
                &piece.source,
                "content size limit exceeded".into(),
                expansion,
            ));
            return ContentSummary::ERROR;
        }
        match &piece.kind {
            Construction::Ready { nodes } => {
                if *nodes > limit {
                    out.push(self.error(
                        &piece.source,
                        "content size limit exceeded".into(),
                        expansion,
                    ));
                    return ContentSummary::ERROR;
                }
                let mut node = piece.source.clone();
                expand_ready(&mut node, expansion);
                out.push(node);
                ContentSummary {
                    nodes: *nodes,
                    failed: false,
                }
            }
            Construction::Element(children) => {
                let (children, mut summary) =
                    self.present(children, limit.saturating_sub(1), expansion);
                let mut node = item_with_content(
                    &piece.source,
                    ContentItem {
                        ctor: piece.source.ctor(),
                        fields: piece
                            .source
                            .as_item()
                            .map(|i| i.fields.clone())
                            .unwrap_or_default(),
                        children: children.into(),
                        instance: piece.source.as_item().and_then(|i| i.instance.clone()),
                    },
                );
                append_expansion(&mut node, expansion);
                if !valid_structure(&node) {
                    out.push(self.error(
                        &piece.source,
                        "evaluated content violated its checked structural role".into(),
                        expansion,
                    ));
                    return ContentSummary::ERROR;
                }
                out.push(node);
                summary.nodes += 1;
                summary
            }
            Construction::Splice(content) => {
                let (nodes, mut summary) =
                    self.present(content, limit.saturating_sub(1), expansion);
                emit_mount(&piece.source, nodes, expansion, out);
                summary.nodes += 1;
                summary
            }
            Construction::Repeat {
                expression,
                environment,
            } => {
                let mut chain = piece
                    .source
                    .origin
                    .as_ref()
                    .map(|o| o.expansion.clone())
                    .unwrap_or_default();
                if chain.is_empty() {
                    chain = expansion.clone();
                } else {
                    chain.extend_from_slice(expansion);
                }
                match self.repeat(expression, environment, limit.saturating_sub(1), &chain) {
                    Ok((nodes, mut summary)) => {
                        emit_mount(&piece.source, nodes, expansion, out);
                        summary.nodes += 1;
                        summary
                    }
                    Err(failure) => {
                        out.push(self.error_at(&piece.source, failure, expansion));
                        ContentSummary::ERROR
                    }
                }
            }
            Construction::Call {
                call,
                environment,
                children,
            } => self.present_call(
                &piece.source,
                call,
                environment,
                children,
                limit,
                expansion,
                out,
            ),
            Construction::Error => unreachable!(),
        }
    }
    fn repeat(
        &mut self,
        expression: &TypedExpr,
        environment: &Scope,
        limit: usize,
        expansion: &SourceChain,
    ) -> Result<(Vec<Node>, ContentSummary), Failure> {
        let ExprKind::For {
            binding,
            subject,
            body,
        } = &expression.kind
        else {
            return Err("invalid deferred for computation".into());
        };
        if self.active_loops >= 32 {
            return Err("for nesting limit exceeded".into());
        }
        self.active_loops += 1;
        let result = (|| {
            let Value::Array(values) = data(self.expr(subject, environment)?)? else {
                return Err("for subject did not produce an Array".into());
            };
            if values.len() > self.limits.iterations.saturating_sub(self.iterations) {
                return Err("cumulative iteration limit exceeded".into());
            }
            let mut nodes = Vec::new();
            let mut summary = ContentSummary::default();
            for (index, value) in values.iter().enumerate() {
                self.iterations = self.iterations.saturating_add(1);
                if self.iterations > self.limits.iterations {
                    return Err("cumulative iteration limit exceeded".into());
                }
                self.tick()?;
                let mut local = environment.clone();
                local.insert(*binding, Runtime::Value(ComptimeValue::Data(value.clone())));
                let value = self.expr(body, &local).map_err(|mut failure| {
                    failure.message = format!("for iteration {index}: {}", failure.message);
                    failure
                })?;
                let Runtime::Content(ty, content) = value else {
                    return Err("for body produced data".into());
                };
                if Some(&ty) != expression.ty.as_ref() {
                    return Err("for body violated its checked category".into());
                }
                let (iteration, added) =
                    self.present(&content, limit.saturating_sub(summary.nodes), expansion);
                if summary.nodes.saturating_add(added.nodes) > limit {
                    return Err("content size limit exceeded".into());
                }
                if added.failed {
                    return Err(format!("for iteration {index} failed").into());
                }
                summary.append(added);
                nodes.extend(iteration);
            }
            Ok((nodes, summary))
        })();
        self.active_loops -= 1;
        result
    }
    fn present_call(
        &mut self,
        source: &Node,
        call: &Invocation,
        environment: &Scope,
        children: &Content,
        limit: usize,
        expansion: &SourceChain,
        out: &mut Vec<Node>,
    ) -> ContentSummary {
        let target = self.target(&call.instance.function);
        let code = self
            .context
            .registry()
            .definition(target)
            .is_some_and(|d| d.implementation.is_code());
        if code && self.depth >= self.limits.call_depth.min(32) {
            out.push(self.error(
                source,
                "evaluation call-depth limit exceeded".into(),
                expansion,
            ));
            return ContentSummary::ERROR;
        }
        if code {
            self.depth += 1;
        }
        let previous_steps = std::mem::replace(&mut self.steps, 0);
        let result = self.execute(call, environment, children.clone(), Some(source));
        let summary = match result {
            Ok(Runtime::Content(_, content)) => {
                if code {
                    let mut chain = vec![location(call)];
                    if let Some(origin) = &source.origin {
                        chain.extend_from_slice(&origin.expansion);
                    }
                    chain.extend_from_slice(expansion);
                    let chain = SourceChain::from(chain);
                    let (nodes, mut summary) =
                        self.present(&content, limit.saturating_sub(1), &chain);
                    emit_mount(source, nodes, expansion, out);
                    summary.nodes += 1;
                    summary
                } else {
                    let (nodes, summary) = self.present(&content, limit, expansion);
                    out.extend(nodes);
                    summary
                }
            }
            Ok(_) => {
                out.push(self.error(
                    source,
                    "content call produced a data value".into(),
                    expansion,
                ));
                ContentSummary::ERROR
            }
            Err(failure) => {
                out.push(self.error_at(source, failure, expansion));
                ContentSummary::ERROR
            }
        };
        if code {
            self.depth -= 1;
        }
        self.steps = previous_steps;
        summary
    }
    fn synthetic_call(&self, item: &Node) -> Result<Option<Invocation>, Failure> {
        let Some(id) = item.function_id() else {
            return Ok(None);
        };
        let definition = self
            .context
            .registry()
            .definition(id)
            .ok_or("missing synthetic call declaration")?
            .clone();
        let instance = item
            .instance()
            .cloned()
            .unwrap_or_else(|| FunctionInstance {
                function: id,
                arguments: definition
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
        let signature = self
            .context
            .registry()
            .signature(id)
            .ok_or("missing synthetic call signature")?
            .clone();
        if signature.comptime().len() != instance.arguments.len() {
            return Err("synthetic call comptime arity mismatch".into());
        }
        let mut bindings = notist_core::signature::ComptimeBindings::new(signature.clone());
        for (p, v) in signature.comptime().iter().zip(&instance.arguments) {
            bindings
                .insert(p.id, v.clone())
                .map_err(|error| Failure::from(error.to_string()))?;
        }
        let definition = bindings
            .instantiate()
            .map_err(|error| Failure::from(error.to_string()))?;
        let source = item.origin.as_ref().map(|o| o.source).unwrap_or_default();
        let arguments = item
            .fields()
            .iter()
            .map(|(name, value)| {
                (
                    name.into(),
                    TypedExpr {
                        ty: Some(
                            definition
                                .parameters
                                .iter()
                                .find(|p| p.name == name)
                                .map(|p| p.ty.clone())
                                .unwrap_or_else(|| Type::of(value)),
                        ),
                        known: Some(ComptimeValue::Data(value.clone())),
                        kind: ExprKind::Constant(ComptimeValue::Data(value.clone())),
                        span: item.span,
                        source,
                        shape: Default::default(),
                    },
                )
            })
            .collect();
        Ok(Some(Invocation {
            instance,
            definition: definition.into(),
            arguments,
            children: None,
            attrs: item.attrs.clone(),
            span: item.span,
            source,
            site: CallSite::Markup,
            resource_root: Some(self.context.resource_root().into()),
            invalid: None,
        }))
    }
    fn error(&mut self, source: &Node, message: String, expansion: &SourceChain) -> Node {
        let mut node = error_content(source, message.clone());
        append_expansion(&mut node, expansion);
        let mut diagnostic = Diagnostic::new(Phase::Type, node.span, message);
        if let Some(origin) = &node.origin {
            diagnostic = diagnostic.at_source(&self.sources, origin.source);
        }
        self.diagnostics.push(diagnostic);
        node
    }
    fn error_at(&mut self, source: &Node, failure: Failure, expansion: &SourceChain) -> Node {
        let node = failure_content(source, failure);
        let Ctor::Error(message) = node.ctor() else {
            unreachable!()
        };
        self.error(&node, message, expansion)
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
        let bytes = match self
            .inputs
            .map(|i| i.file(path))
            .unwrap_or(InputRead::Failed(InputError::Unavailable))
        {
            InputRead::Ready(bytes) => bytes,
            InputRead::Pending => {
                self.needed.insert(path.clone());
                return Err(Failure::at("resource input is not prepared", call));
            }
            InputRead::Failed(error) => {
                return Err(Failure::at(
                    format!("read_text `{}`: {error}", path.path()),
                    call,
                ));
            }
        };
        if bytes.len() > self.limits.value_bytes {
            return Err(Failure::at("value size limit exceeded", call));
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| {
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
        children: Content,
        source: Option<&Node>,
    ) -> Result<Runtime, Failure> {
        self.tick()?;
        if let Some(message) = &call.invalid {
            return Err(message.clone().into());
        }
        let target = FunctionInstance {
            function: self.target(&call.instance.function),
            arguments: call.instance.arguments.clone(),
        };
        let code = self
            .context
            .registry()
            .definition(target.function)
            .is_some_and(|d| d.implementation.is_code());
        let mut pending_inputs = Vec::new();
        let mut fields = Dict::default();
        for (name, expression) in &call.arguments {
            let value = self.expr(expression, scope)?;
            let parameter = call.definition.parameters.iter().find(|p| p.name == *name);
            let value = if parameter.is_some_and(|p| p.mode == ParameterMode::Optional)
                && matches!(expression.ty, Some(Type::Option(_)))
            {
                match value {
                    Runtime::Value(ComptimeValue::Data(Value::Some(value))) => {
                        Runtime::Value(ComptimeValue::Data(*value))
                    }
                    Runtime::Value(ComptimeValue::Data(Value::None)) => continue,
                    _ => {
                        return Err(
                            format!("invalid option for `{}.{name}`", call.definition.id).into(),
                        );
                    }
                }
            } else {
                value
            };
            if let Some(parameter) = parameter
                && let Runtime::Value(ComptimeValue::Data(value)) = &value
                && !parameter.accepts(value)
            {
                return Err(format!("invalid value for `{}.{name}`", call.definition.id).into());
            }
            if let Runtime::Value(ComptimeValue::Data(value)) = &value {
                fields.insert(name, value.clone());
            }
            if code {
                let input = if parameter.is_some_and(|p| p.mode == ParameterMode::Optional) {
                    match value {
                        Runtime::Value(ComptimeValue::Data(v)) => {
                            Runtime::Value(ComptimeValue::Data(Value::Some(Box::new(v))))
                        }
                        other => other,
                    }
                } else {
                    value
                };
                pending_inputs.push((name.as_str(), input));
            }
        }
        let implementation = self.implementation(&target)?;
        let definition = &implementation.definition;
        let inputs = if code {
            let mut inputs = Scope::new();
            for (name, value) in pending_inputs {
                let id = implementation
                    .inputs
                    .get(name)
                    .ok_or("missing checked parameter binding")?;
                inputs.insert(*id, value);
            }
            for p in &call.definition.parameters {
                let id = implementation
                    .inputs
                    .get(&p.name)
                    .ok_or("missing checked parameter binding")?;
                if p.mode == ParameterMode::Optional && !inputs.contains_key(id) {
                    inputs.insert(*id, Runtime::Value(ComptimeValue::Data(Value::None)));
                }
            }
            if let Some(ty) = &call.definition.children_type {
                inputs.insert(
                    *implementation
                        .inputs
                        .get("children")
                        .ok_or("missing checked children binding")?,
                    Runtime::Content(ty.clone(), children.clone()),
                );
            }
            Some(inputs)
        } else {
            None
        };
        if call.definition.id.module == "notist"
            && call.definition.id.name == "heading"
            && let Some(ComptimeValue::Data(value)) = call.instance.arguments.first()
        {
            fields.insert("level", value.clone());
        }
        let data_call = call.definition.returns.level().is_none();
        if data_call {
            if self.depth >= self.limits.call_depth.min(32) {
                return Err("evaluation call-depth limit exceeded".into());
            }
            self.depth += 1;
        }
        // The closure ensures every early failure restores physical call depth.
        let result = (|| match definition.implementation {
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
            FunctionImpl::Native(NativeFunction::PathJoin) => {
                let (Some(Value::Path(base)), Some(Value::Str(relative))) =
                    (fields.get("self"), fields.get("relative"))
                else {
                    return Err(Failure::at(
                        "Path.join requires a Path receiver and String relative path",
                        call,
                    ));
                };
                let path = base
                    .join(relative)
                    .map_err(|error| Failure::at(error, call))?;
                Ok(Runtime::Value(ComptimeValue::Data(Value::Path(path))))
            }
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
                Some(body) => self.expr(body, &inputs.unwrap_or_default()),
                None => Err("missing checked body".into()),
            },
            FunctionImpl::Native(NativeFunction::Construct) => {
                if definition.returns.level().is_none() {
                    return Err(format!(
                        "native value function `{}` has no implementation",
                        definition.id
                    )
                    .into());
                }
                self.require_runtime()?;
                if definition.id.module == "notist" && definition.id.name == "embed" {
                    notist_core::embed::normalize(
                        &mut fields,
                        call.resource_root
                            .as_deref()
                            .unwrap_or(self.context.resource_root()),
                        self.sources
                            .name(call.source)
                            .map_err(|e| Failure::from(e.to_string()))?,
                    )?;
                }
                let source = source.ok_or("missing content source")?;
                let ctor = if definition.id.module == "notist" {
                    Ctor::from_name(&definition.id.name).unwrap_or_else(|| source.ctor())
                } else {
                    Ctor::Extension(notist_core::item::ExtensionCtor {
                        id: definition.id.def_id(),
                        accepts: definition.children,
                        level: source.level,
                    })
                };
                let node = item_with_content(
                    source,
                    ContentItem {
                        ctor,
                        fields,
                        children: Default::default(),
                        instance: Some(target.clone().into()),
                    },
                );
                let count = 1 + children.nodes;
                Ok(Runtime::Content(
                    call.definition.returns.clone(),
                    Content::one(
                        Piece {
                            source: node,
                            kind: Construction::Element(children),
                        },
                        count,
                    ),
                ))
            }
        })();
        if data_call {
            self.depth -= 1;
        }
        let result = result.map_err(|mut failure| {
            if failure.location.is_some() {
                let here = location(call);
                if failure.location.as_ref() != Some(&here)
                    && failure.expansion.last() != Some(&here)
                {
                    failure.expansion.push(here);
                }
            }
            failure
        })?;
        let valid = match &result {
            Runtime::Content(ty, _) => *ty == call.definition.returns,
            Runtime::Value(ComptimeValue::Data(value)) => call.definition.returns.accepts(value),
            Runtime::Value(ComptimeValue::Type(_)) => call.definition.returns == Type::Type,
        };
        if !valid {
            return Err(if data_call {
                "implementation violated its data result type"
            } else {
                "implementation violated its content result type"
            }
            .into());
        }
        Ok(result)
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
        let key = instance
            .key(self.sources.definitions())
            .map_err(|e| Failure::from(e.to_string()))?;
        if !self.program.instances.contains_key(&key) {
            self.program.data_retention = self.data_retention.clone();
            let (program, diagnostics) = self
                .context
                .specialize_resolved(instance, std::mem::take(&mut self.program));
            self.data_retention = program.data_retention.clone();
            self.diagnostics.extend(diagnostics);
            self.sources
                .merge(&program.sources)
                .map_err(|e| Failure::from(e.to_string()))?;
            self.program = program;
        }
        let record = self
            .program
            .instances
            .get(&key)
            .ok_or("missing function instance")?;
        if record.definition.id.def_id() != instance.function {
            return Err("function instance identity mismatch".into());
        }
        if let Some(diagnostic) = record.diagnostics.first() {
            return Err(format!("{}: {}", record.definition.id, diagnostic.message).into());
        }
        self.implementations.insert(instance, record);
        Ok(record.clone())
    }
}
fn location(call: &Invocation) -> SourceLocation {
    SourceLocation {
        source: call.source,
        start: call.span.start().into(),
        end: call.span.end().into(),
    }
}
fn ready_count(root: &Node) -> Option<usize> {
    let mut count = 0;
    for node in root.descendants() {
        let NodeContent::Item(item) = &node.content else {
            return None;
        };
        if matches!(item.ctor, Ctor::Error(_) | Ctor::Interpolation) || !valid_structure(node) {
            return None;
        }
        count += 1;
    }
    Some(count)
}
fn expand_ready(node: &mut Node, expansion: &SourceChain) {
    if expansion.is_empty() {
        return;
    }
    append_expansion(node, expansion);
    for child in node.children_mut().into_iter().flatten() {
        expand_ready(child, expansion);
    }
}
fn item_with_content(source: &Node, content: ContentItem) -> Node {
    Node {
        content: NodeContent::Item(content),
        attrs: source.attrs.clone(),
        origin: source.origin.clone(),
        level: source.level,
        span: source.span,
    }
}
fn error_content(source: &Node, message: String) -> Node {
    item_with_content(
        source,
        ContentItem {
            ctor: Ctor::Error(message),
            fields: Dict::default(),
            children: Default::default(),
            instance: None,
        },
    )
}
fn error_piece(source: &Node, message: String) -> Piece {
    Piece {
        source: error_content(source, message),
        kind: Construction::Error,
    }
}
fn failure_content(source: &Node, failure: Failure) -> Node {
    let mut node = error_content(source, failure.message);
    if let Some(location) = failure.location {
        let mut expansion = failure.expansion;
        if let Some(origin) = &source.origin {
            let here = SourceLocation {
                source: origin.source,
                start: origin.start,
                end: origin.end,
            };
            if expansion.last() != Some(&here) {
                expansion.push(here);
            }
            expansion.extend(origin.expansion.iter().cloned());
        }
        node.span = rowan::TextRange::new(location.start.into(), location.end.into());
        node.origin = Some(Origin {
            source: location.source,
            start: location.start,
            end: location.end,
            expansion: expansion.into(),
        });
    }
    node
}
fn append_expansion(node: &mut Node, expansion: &SourceChain) {
    if !expansion.is_empty()
        && let Some(origin) = &mut node.origin
    {
        if origin.expansion.is_empty() {
            origin.expansion = expansion.clone();
        } else {
            origin.expansion.extend_from_slice(expansion);
        }
    }
}
fn emit_mount(source: &Node, nodes: Vec<Node>, expansion: &SourceChain, out: &mut Vec<Node>) {
    if source.attrs.is_empty() {
        out.extend(nodes);
    } else {
        let mut group = item_with_content(
            source,
            ContentItem {
                ctor: Ctor::Group,
                fields: Dict::default(),
                children: nodes.into(),
                instance: None,
            },
        );
        append_expansion(&mut group, expansion);
        out.push(group);
    }
}
fn valid_structure(node: &Node) -> bool {
    fn children(nodes: &[Node], expected: &Ctor) -> bool {
        nodes.iter().all(|node| {
            node.is_error()
                || if matches!(node.ctor(), Ctor::Node | Ctor::Group) {
                    children(node.children(), expected)
                } else {
                    node.ctor() == *expected
                }
        })
    }
    match node.ctor().accepts() {
        Some(notist_core::builtins::Accepts::Rows) => children(node.children(), &Ctor::TableRow),
        Some(notist_core::builtins::Accepts::Cells) => children(node.children(), &Ctor::TableCell),
        Some(notist_core::builtins::Accepts::Items) => children(node.children(), &Ctor::ListItem),
        _ => true,
    }
}
fn value_of(value: Runtime) -> Result<ComptimeValue, String> {
    match value {
        Runtime::Value(v) => Ok(v),
        _ => Err("expected a data or Type value".into()),
    }
}
fn data(value: Runtime) -> Result<Value, String> {
    match value {
        Runtime::Value(ComptimeValue::Data(v)) => Ok(v),
        _ => Err("expected a data value".into()),
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
    use notist_core::{definitions::FunctionId, registry::Registry};

    struct Context;
    impl EvaluationContext for Context {
        fn registry(&self) -> &Registry {
            notist_core::builtins::registry()
        }
        fn resource_root(&self) -> &str {
            "/project"
        }
        fn target(&self, id: &FunctionId) -> FunctionId {
            id.clone()
        }
        fn has_mappings(&self) -> bool {
            false
        }
        fn specialize(
            &self,
            _: &notist_core::definitions::FunctionInstance,
            _: CheckedProgram,
        ) -> (CheckedProgram, Vec<Diagnostic>) {
            panic!("constructing a closure must not execute or specialize it")
        }
    }
    fn input(id: notist_core::binding::BindingId, ty: Type) -> TypedExpr {
        TypedExpr {
            ty: Some(ty),
            known: None,
            kind: ExprKind::Input(id),
            span: Default::default(),
            source: Default::default(),
            shape: Default::default(),
        }
    }
    fn evaluator<'a>(context: &'a Context, program: &CheckedProgram) -> Evaluator<'a, Context> {
        Evaluator::new(
            context,
            program,
            &Default::default(),
            Limits::default(),
            None,
            EvaluationMode::Runtime,
        )
    }
    #[test]
    fn final_item_payload_preserves_source_metadata_without_source_fields_or_children() {
        let source = Node::text("source".into(), rowan::TextRange::new(3.into(), 9.into()));
        let mut source =
            source.with_children(vec![Node::text("old child".into(), Default::default())]);
        source.attrs.insert("id", Value::Str("mount".into()));
        let owner = notist_core::source::Source::metadata("origin.not").unwrap();
        source.origin = Some(notist_core::item::Origin {
            source: owner.id(),
            start: 3,
            end: 9,
            expansion: Default::default(),
        });
        let children: Shared<Vec<Node>> =
            vec![Node::text("final child".into(), Default::default())].into();
        let node = item_with_content(
            &source,
            ContentItem {
                ctor: Ctor::Group,
                fields: Dict::default(),
                children: children.clone(),
                instance: None,
            },
        );
        assert_eq!(node.attrs, source.attrs);
        assert_eq!(node.origin, source.origin);
        assert_eq!(node.level, source.level);
        assert_eq!(node.span, source.span);
        assert_eq!(node.ctor(), Ctor::Group);
        assert!(node.fields().is_empty());
        assert!(node.as_item().unwrap().children.shares_storage(&children));
        assert_eq!(
            source.children()[0].field("text"),
            Some(&Value::Str("old child".into()))
        );

        let error = error_content(&source, "failure".into());
        assert_eq!(error.attrs, source.attrs);
        assert_eq!(error.origin, source.origin);
        assert_eq!(error.span, source.span);
        assert!(error.children().is_empty());
        assert!(error.fields().is_empty());
    }

    #[test]
    fn call_closures_share_code_and_capture_lexical_snapshots_without_substitution() {
        let context = Context;
        let mut e = evaluator(&context, &CheckedProgram::default());
        let x = e
            .sources
            .bindings_mut()
            .declare(
                "x",
                &notist_core::source::Source::detached(),
                Default::default(),
            )
            .unwrap();
        let definition = context
            .registry()
            .function("notist", "text")
            .unwrap()
            .clone();
        let call: Shared<Invocation> = Invocation {
            instance: FunctionInstance {
                function: definition.id.def_id(),
                arguments: Vec::new(),
            },
            definition: definition.into(),
            arguments: vec![("text".into(), input(x, Type::String))],
            children: None,
            attrs: Dict::default(),
            span: Default::default(),
            source: Default::default(),
            site: CallSite::Code,
            resource_root: None,
            invalid: None,
        }
        .into();
        let expression = TypedExpr {
            ty: Some(Type::InlineContent),
            kind: ExprKind::Call(call.clone()),
            known: None,
            span: Default::default(),
            source: Default::default(),
            shape: Default::default(),
        };
        let node = Node::expression(expression);
        let mut scope = Scope::new();
        scope.insert(
            x,
            Runtime::Value(ComptimeValue::Data(Value::Str("first".into()))),
        );
        let content = e.bind_content(&[node], &scope, 100);
        scope.insert(
            x,
            Runtime::Value(ComptimeValue::Data(Value::Str("second".into()))),
        );
        let Construction::Call {
            call: saved,
            environment,
            ..
        } = &content.pieces[0].kind
        else {
            panic!()
        };
        assert!(saved.shares_storage(&call));
        assert!(matches!(&saved.arguments[0].1.kind, ExprKind::Input(id) if *id == x));
        assert!(
            matches!(environment.get(&x), Some(Runtime::Value(ComptimeValue::Data(Value::Str(v)))) if v.as_str() == "first")
        );
        assert!(content.pieces.shares_storage(&content.clone().pieces));
        assert_eq!(e.requests, 0);
    }
    #[test]
    fn loop_closures_share_templates_and_do_not_evaluate_the_subject_when_saved() {
        let context = Context;
        let mut e = evaluator(&context, &CheckedProgram::default());
        let content_id = e
            .sources
            .bindings_mut()
            .declare(
                "content",
                &notist_core::source::Source::detached(),
                Default::default(),
            )
            .unwrap();
        let x = e
            .sources
            .bindings_mut()
            .declare(
                "x",
                &notist_core::source::Source::detached(),
                Default::default(),
            )
            .unwrap();
        let missing = e
            .sources
            .bindings_mut()
            .declare(
                "missing",
                &notist_core::source::Source::detached(),
                Default::default(),
            )
            .unwrap();
        let body: Shared<TypedExpr> = input(content_id, Type::InlineContent).into();
        let expression = TypedExpr {
            ty: Some(Type::InlineContent),
            known: None,
            kind: ExprKind::For {
                binding: x,
                subject: input(missing, Type::Array(None)).into(),
                body: body.clone(),
            },
            source: Default::default(),
            span: Default::default(),
            shape: Default::default(),
        };
        let Runtime::Content(_, content) = e.expr(&expression, &Scope::new()).unwrap() else {
            panic!()
        };
        let Construction::Repeat {
            expression: saved, ..
        } = &content.pieces[0].kind
        else {
            panic!()
        };
        let ExprKind::For { body: saved, .. } = &saved.kind else {
            panic!()
        };
        assert!(body.shares_storage(saved));
        assert_eq!(e.iterations, 0);
    }
    #[test]
    fn concrete_content_shares_output_storage_but_mutation_detaches() {
        let context = Context;
        let mut e = evaluator(&context, &CheckedProgram::default());
        let source = Node::new(Ctor::Doc, Default::default())
            .with_children(vec![Node::text("one".into(), Default::default())]);
        let content = e.bind_content(std::slice::from_ref(&source), &Scope::new(), 100);
        assert!(matches!(
            content.pieces[0].kind,
            Construction::Ready { nodes: 2 }
        ));
        let (mut result, summary) = e.present(&content, 100, &SourceChain::default());
        assert_eq!(summary.nodes, 2);
        assert!(
            result[0]
                .as_item()
                .unwrap()
                .children
                .shares_storage(&source.as_item().unwrap().children)
        );
        result[0].children_mut().unwrap().clear();
        assert_eq!(source.children().len(), 1);
        assert_eq!(content.nodes, 2);
    }
    #[test]
    fn recovered_source_children_are_never_bound_or_executed() {
        let context = Context;
        let mut e = evaluator(&context, &CheckedProgram::default());
        let missing = e
            .sources
            .bindings_mut()
            .declare(
                "missing",
                &notist_core::source::Source::detached(),
                Default::default(),
            )
            .unwrap();
        let source = Node::new(Ctor::Error("broken".into()), Default::default())
            .with_children(vec![Node::expression(input(missing, Type::InlineContent))]);
        let content = e.bind_content(&[source], &Scope::new(), 100);
        assert_eq!(e.steps, 0);
        let (nodes, summary) = e.present(&content, 100, &SourceChain::default());
        assert!(nodes[0].is_error());
        assert!(nodes[0].children().is_empty());
        assert!(summary.failed);
        assert_eq!(e.diagnostics.len(), 1);
    }
    #[test]
    fn expansion_shares_new_suffixes_without_merging_distinct_source_chains() {
        let owner = notist_core::source::Source::metadata("outer.not").unwrap();
        let suffix: SourceChain = vec![SourceLocation {
            source: owner.id(),
            start: 5,
            end: 9,
        }]
        .into();
        let mut bindings = notist_core::binding::Bindings::default();
        let x = bindings.declare("x", &owner, Default::default()).unwrap();
        let mut first = Node::expression(input(x, Type::InlineContent));
        let mut second = first.clone();
        append_expansion(&mut first, &suffix);
        append_expansion(&mut second, &suffix);
        assert!(
            first
                .origin
                .as_ref()
                .unwrap()
                .expansion
                .shares_storage(&second.origin.as_ref().unwrap().expansion)
        );
        first.origin.as_mut().unwrap().expansion[0].start = 1;
        assert_eq!(second.origin.as_ref().unwrap().expansion[0].start, 5);
        append_expansion(&mut first, &suffix);
        assert_eq!(first.origin.as_ref().unwrap().expansion.len(), 2);
        assert_eq!(second.origin.as_ref().unwrap().expansion.len(), 1);
    }
    #[test]
    fn both_result_payloads_count_towards_the_value_limit() {
        for value in [
            Value::Ok(Box::new(Value::Str("oversized".into()))),
            Value::Err(Box::new(Value::Str("oversized".into()))),
        ] {
            assert_eq!(
                check_value_size(&ComptimeValue::Data(value), 3).unwrap_err(),
                "value size limit exceeded"
            );
        }
    }
}
