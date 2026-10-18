use super::*;
use crate::Context;
use notist_core::{
    builtins::Level,
    diag::Phase,
    item::{EvaluationState, SourceLocation, Value},
};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub steps: usize,
    /// Maximum normal nodes in a content forest, including enclosing nodes.
    /// Recovery placeholders do not consume this budget.
    pub content_nodes: usize,
    /// The recursive interpreter additionally caps physical call depth at 32.
    pub call_depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            steps: 100_000,
            content_nodes: 100_000,
            call_depth: 32,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub root: Item,
    pub diagnostics: Vec<Diagnostic>,
}

/// Content values retain opaque calls. Constructing or binding a forest does
/// not execute its content implementations; presentation walks only the result.
#[derive(Debug, Clone)]
enum Runtime {
    Value(ComptimeValue),
    Content(Type, Vec<Item>),
}
type Scope = BTreeMap<String, Runtime>;
struct Evaluator<'a> {
    context: &'a Context,
    program: CheckedProgram,
    diagnostics: Vec<Diagnostic>,
    limits: Limits,
    steps: usize,
    depth: usize,
    transforms: bool,
}

pub(crate) fn evaluate(
    root: &Item,
    program: &CheckedProgram,
    context: &Context,
    limits: Limits,
) -> Evaluation {
    let mut e = Evaluator {
        context,
        program: program.clone(),
        diagnostics: Vec::new(),
        limits,
        steps: 0,
        depth: 0,
        transforms: true,
    };
    let forest = e.bind_content(
        std::slice::from_ref(root),
        &Scope::new(),
        limits.content_nodes,
    );
    let mut roots = e.present(&forest, limits.content_nodes);
    for diagnostic in &mut e.diagnostics {
        diagnostic.recovered = true;
    }
    Evaluation {
        root: roots.remove(0),
        diagnostics: e.diagnostics,
    }
}
pub(super) fn constant(
    t: &TypedExpr,
    program: &CheckedProgram,
    context: &Context,
) -> Result<ComptimeValue, String> {
    let mut e = Evaluator {
        context,
        program: program.clone(),
        diagnostics: Vec::new(),
        limits: Limits::default(),
        steps: 0,
        depth: 0,
        transforms: false,
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
        if self.transforms {
            self.context.transforms().target(id)
        } else {
            id.clone()
        }
    }
    fn tick(&mut self) -> Result<(), String> {
        self.steps += 1;
        if self.steps > self.limits.steps {
            Err("evaluation step limit exceeded".into())
        } else {
            Ok(())
        }
    }
    fn expr(&mut self, t: &TypedExpr, scope: &Scope) -> Result<Runtime, String> {
        self.tick()?;
        if let Some(v) = &t.known {
            check_value_size(v)?;
            return Ok(Runtime::Value(v.clone()));
        }
        let result = match &t.kind {
            ExprKind::Constant(v) => Ok(Runtime::Value(v.clone())),
            ExprKind::Input(name) => scope
                .get(name)
                .cloned()
                .ok_or_else(|| format!("runtime input `{name}` is unavailable at compile time")),
            ExprKind::Failure(m) => Err(m.clone()),
            ExprKind::Content(forest) => {
                if !self.transforms {
                    return Err("content cannot be constructed at compile time".into());
                }
                Ok(Runtime::Content(
                    t.ty.clone().unwrap(),
                    self.bind_content(forest, scope, self.limits.content_nodes),
                ))
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
                unary(op, &v).map(Runtime::Value)
            }
            ExprKind::Binary(op, left, right) => {
                let left = value_of(self.expr(left, scope)?)?;
                if op == "&&" && left == ComptimeValue::Data(Value::Bool(false))
                    || op == "||" && left == ComptimeValue::Data(Value::Bool(true))
                {
                    return Ok(Runtime::Value(left));
                }
                let right = value_of(self.expr(right, scope)?)?;
                binary(op, &left, &right).map(Runtime::Value)
            }
            ExprKind::Array(values) => {
                let values = values
                    .iter()
                    .map(|v| self.expr(v, scope).and_then(data))
                    .collect::<Result<_, _>>()?;
                Ok(Runtime::Value(ComptimeValue::Data(Value::Array(values))))
            }
            ExprKind::Dict(values) => {
                let values = values
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), data(self.expr(v, scope)?)?)))
                    .collect::<Result<_, String>>()?;
                Ok(Runtime::Value(ComptimeValue::Data(Value::Dict(values))))
            }
            ExprKind::ValueCall(call) => {
                let children = match &call.children {
                    Some(child) => match self.expr(child, scope)? {
                        Runtime::Content(_, nodes) => nodes,
                        _ => return Err("invalid children".into()),
                    },
                    None => Vec::new(),
                };
                self.execute(call, scope, children, None)
            }
        }?;
        if let Runtime::Value(v) = &result {
            check_value_size(v)?;
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
                    Ok(Runtime::Content(_, nodes)) => nodes,
                    Ok(_) => vec![error_content(
                        node.clone(),
                        "content interpolation produced a data value".into(),
                    )],
                    Err(m) => vec![error_content(node.clone(), m)],
                },
                pending => {
                    let mut item = node.clone();
                    if let Some(PendingEvaluation::Call(call)) = pending {
                        item.pending =
                            Some(Box::new(PendingEvaluation::Call(capture_call(call, scope))));
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
        if item.evaluation.is_none() {
            if let Some(id) = item.function_id() {
                if self.target(&id) != id {
                    if let Some(call) = self.synthetic_call(item) {
                        return self.present_call(item, &call, limit);
                    }
                }
            }
        }
        let mut item = item.clone();
        item.pending = None;
        item.children = self.present(&item.children, limit.saturating_sub(1));
        vec![item]
    }
    fn present_call(&mut self, item: &Item, call: &Invocation, limit: usize) -> Vec<Item> {
        let target = self.target(&call.instance.function);
        let expands = self.context.registry().get(&target).is_some_and(|d| d.code);
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
            Err(m) => vec![self.error(item.clone(), m)],
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
                        ty: Some(Type::of(v)),
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
            definition: def,
            arguments,
            children: None,
            attrs: item.attrs.clone(),
            span: item.span,
            source,
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
    fn execute(
        &mut self,
        call: &Invocation,
        scope: &Scope,
        children: Vec<Item>,
        prototype: Option<Item>,
    ) -> Result<Runtime, String> {
        self.tick()?;
        if let Some(m) = &call.invalid {
            return Err(m.clone());
        }
        let mut inputs = Scope::new();
        let mut fields = Dict::default();
        for (name, expr) in &call.arguments {
            let value = self.expr(expr, scope)?;
            if let Some(param) = call.definition.parameters.iter().find(|p| p.name == *name) {
                if let Runtime::Value(ComptimeValue::Data(value)) = &value {
                    if !param.accepts(value) {
                        return Err(format!(
                            "invalid value for `{}.{name}`",
                            call.instance.function
                        ));
                    }
                }
            }
            if let Runtime::Value(ComptimeValue::Data(v)) = &value {
                fields.insert(name, v.clone());
            }
            inputs.insert(name.clone(), value);
        }
        if let Some(ty) = &call.definition.children_type {
            inputs.insert(
                "children".into(),
                Runtime::Content(ty.clone(), children.clone()),
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
        if call.definition.id == notist_core::definitions::FunctionId::new("notist", "heading") {
            if let Some(ComptimeValue::Data(v)) = call.instance.arguments.first() {
                fields.insert("level", v.clone());
            }
        }
        if !definition.code {
            if definition.returns.level().is_none() {
                return Err(format!(
                    "native value function `{}` has no implementation",
                    definition.id
                ));
            }
            let mut item = prototype.ok_or("missing materialized content call boundary")?;
            item.fields = fields;
            item.children = children;
            item.pending = None;
            item.target = Some(target);
            item.evaluation = Some(EvaluationState::Native);
            return Ok(Runtime::Content(
                call.definition.returns.clone(),
                vec![item],
            ));
        }
        let value_call = call.definition.returns.level().is_none();
        if value_call {
            if self.depth >= self.limits.call_depth.min(32) {
                return Err("evaluation call-depth limit exceeded".into());
            }
            self.depth += 1;
        }
        let result = match body {
            Some(body) => self.expr(&body, &inputs),
            None => Err("missing checked body".into()),
        };
        if value_call {
            self.depth -= 1;
        }
        let result = result?;
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
            item.evaluation = Some(EvaluationState::Expanded);
            let mut expansion = vec![SourceLocation {
                source: call.source.clone(),
                start: call.span.start().into(),
                end: call.span.end().into(),
            }];
            if let Some(origin) = &item.origin {
                expansion.extend(origin.expansion.iter().cloned());
            }
            for child in &mut forest {
                expand(child, &expansion);
            }
            Ok(Runtime::Content(call.definition.returns.clone(), {
                item.children = forest;
                vec![item]
            }))
        } else {
            Ok(result)
        }
    }
    fn implementation(
        &mut self,
        instance: &FunctionInstance,
    ) -> Result<(FunctionDef, Option<TypedExpr>), String> {
        let key = instance.key();
        if !self.program.instances.contains_key(&key) {
            let mut checker = super::check::Checker::with_program(
                self.context,
                std::mem::take(&mut self.program),
            );
            checker.install(instance);
            self.diagnostics.extend(checker.diagnostics);
            self.program = checker.program;
        }
        let record = self
            .program
            .instances
            .get(&key)
            .ok_or("missing function instance")?;
        if let Some(d) = record.diagnostics.first() {
            return Err(format!("{}: {}", instance.function, d.message));
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
            Some(Runtime::Content(_, forest)) => ExprKind::Content(forest.clone()),
            None => expr.kind.clone(),
        },
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
                Ok(D(Value::Str(format!("{a}{b}"))))
            }
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            "<" => boolean(a < b),
            "<=" => boolean(a <= b),
            ">" => boolean(a > b),
            ">=" => boolean(a >= b),
            _ => Err("invalid string operator".into()),
        },
        (D(Value::Bool(a)), D(Value::Bool(b))) => match op {
            "&&" => boolean(*a && *b),
            "||" => boolean(*a || *b),
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("invalid Bool operator".into()),
        },
        _ => Err("operator requires matching scalar operands".into()),
    }
}

fn check_value_size(value: &ComptimeValue) -> Result<(), String> {
    fn size(v: &Value) -> usize {
        match v {
            Value::Str(s) => s.len(),
            Value::Array(v) => v
                .iter()
                .fold(v.len().saturating_mul(8), |n, v| n.saturating_add(size(v))),
            Value::Dict(v) => v.iter().fold(0usize, |n, (k, v)| {
                n.saturating_add(k.len()).saturating_add(size(v))
            }),
            _ => 8,
        }
    }
    if let ComptimeValue::Data(v) = value {
        if size(v) > 16 * 1024 * 1024 {
            return Err("value size limit exceeded".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pipeline, analyze_module};

    #[test]
    fn evaluation_reads_the_supplied_item_tree_without_a_parallel_representation() {
        let context = Context::default()
            .with_module(
                analyze_module(
                    "demo",
                    "fn greet(name: String) -> InlineContent { [#text(text: name)] }",
                )
                .unwrap(),
            )
            .unwrap();
        let pipeline = Pipeline::default();
        let analysis = pipeline
            .analyze("doc.not", "#demo::greet(\"before\")", &context)
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
                analyze_module(
                    "demo",
                    "fn pass()[children: InlineContent] -> InlineContent { [#children] }",
                )
                .unwrap(),
            )
            .unwrap();
        let pipeline = Pipeline::default();
        let analysis = pipeline
            .analyze("doc.not", "#demo::pass[one]", &context)
            .unwrap();
        let body = analysis
            .program
            .instances
            .values()
            .find(|r| r.definition.id.package == "demo")
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
        assert_eq!(
            pass.children[0].fields.get("text"),
            Some(&Value::Str("one".into()))
        );
        assert!(evaluation.root.descendants().all(|i| i.pending.is_none()));
    }
}
