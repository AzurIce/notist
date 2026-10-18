use super::*;
use crate::Context;
use notist_core::{
    annotation::Annotation,
    builtins::{Accepts, Level},
    definitions::ParameterMode,
    diag::Phase,
    expr::{BodyFlavor, CallArg, Expr},
    item::{Ctor, ExtensionCtor, Origin, Value},
};
use rowan::TextRange;

type Scope = BTreeMap<String, TypedExpr>;
struct CallFailure {
    definition: Option<FunctionDef>,
    fields: Dict,
    instance: Option<FunctionInstance>,
    message: String,
}

pub(crate) struct Checker<'a> {
    context: &'a Context,
    pub program: CheckedProgram,
    pub diagnostics: Vec<Diagnostic>,
    scope: Scope,
    package: String,
    source: String,
    depth: usize,
    // A function owns direct checking failures; interpolation scopes recover locally.
    failures: Vec<Vec<Diagnostic>>,
}
impl<'a> Checker<'a> {
    pub fn new(context: &'a Context, source: &str) -> Self {
        Self {
            context,
            program: Default::default(),
            diagnostics: Vec::new(),
            scope: BTreeMap::new(),
            package: String::new(),
            source: source.into(),
            depth: 0,
            failures: Vec::new(),
        }
    }
    fn report(&mut self, span: TextRange, message: impl Into<String>) {
        let mut d = Diagnostic::new(Phase::Type, span, message);
        d.source = Some(self.source.clone());
        if let Some(frame) = self.failures.last_mut() {
            frame.push(d.clone());
        }
        self.diagnostics.push(d);
    }
    fn typed(
        &self,
        ty: Option<Type>,
        kind: ExprKind,
        known: Option<ComptimeValue>,
        span: TextRange,
    ) -> TypedExpr {
        TypedExpr {
            ty,
            kind,
            known,
            span,
            source: self.source.clone(),
        }
    }
    fn constant(&self, v: ComptimeValue, span: TextRange) -> TypedExpr {
        self.typed(Some(v.ty()), ExprKind::Constant(v.clone()), Some(v), span)
    }
    fn failed(&mut self, span: TextRange, message: impl Into<String>) -> TypedExpr {
        let message = message.into();
        self.report(span, &message);
        self.typed(None, ExprKind::Failure(message), None, span)
    }
    fn expect(&mut self, t: &TypedExpr, expected: &Type) {
        if t.ty.is_some() && !value_fits(t, expected) {
            self.report(
                t.span,
                format!("expected {expected}, found {}", t.ty.as_ref().unwrap()),
            );
        }
    }

    pub fn document(
        mut self,
        forest: &[Expr],
        annotations: &[Annotation],
        span: TextRange,
        attrs: Dict,
    ) -> (Item, CheckedProgram, Vec<Diagnostic>, Vec<RExpr>) {
        let (children, shaped) = self.content(forest, annotations, BodyFlavor::Block);
        let mut doc = Item::new(Ctor::Doc, span);
        doc.attrs = attrs;
        doc.origin = Some(self.origin(span));
        doc.children = children;
        for diagnostic in &mut self.diagnostics {
            diagnostic.recovered = true;
        }
        (doc, self.program, self.diagnostics, shaped)
    }
    pub fn check_definitions(mut self) -> Vec<Diagnostic> {
        self.check_selected(None);
        self.diagnostics
    }
    pub fn check_module(mut self, package: &str) -> Vec<Diagnostic> {
        for (id, body) in &self.context.program().bodies {
            if id.package == package {
                self.diagnostics.extend(body.diagnostics.iter().cloned());
            }
        }
        self.check_selected(Some(package));
        let mut unique = Vec::new();
        for diagnostic in self.diagnostics {
            if !unique.contains(&diagnostic) {
                unique.push(diagnostic);
            }
        }
        unique
    }
    fn check_selected(&mut self, package: Option<&str>) {
        let definitions: Vec<_> = self
            .context
            .registry()
            .functions()
            .filter(|d| d.code && package.is_none_or(|p| d.id.package == p))
            .cloned()
            .collect();
        for def in definitions {
            let args = def
                .comptime
                .iter()
                .map(|p| p.default.clone())
                .collect::<Option<Vec<_>>>();
            if let Some(arguments) = args {
                self.install(&FunctionInstance {
                    function: def.id,
                    arguments,
                });
            }
        }
    }
    fn origin(&self, span: TextRange) -> Origin {
        Origin {
            source: self.source.clone(),
            start: span.start().into(),
            end: span.end().into(),
            expansion: Vec::new(),
        }
    }
    pub fn resolve(&mut self, forest: &[Expr]) -> Vec<RExpr> {
        forest.iter().map(|e| self.prepare(e)).collect()
    }

    pub fn with_program(context: &'a Context, program: CheckedProgram) -> Self {
        let mut s = Self::new(context, "<evaluation>");
        s.program = program;
        s
    }
    pub fn install(&mut self, instance: &FunctionInstance) {
        let source = self
            .context
            .program()
            .source_path(&instance.function.package)
            .unwrap_or(&self.source)
            .to_owned();
        let previous = std::mem::replace(&mut self.source, source);
        self.install_instance(instance);
        self.source = previous;
    }
    fn install_instance(&mut self, instance: &FunctionInstance) {
        let Some(mut def) = self.context.registry().get(&instance.function).cloned() else {
            return;
        };
        if def.comptime.len() != instance.arguments.len() {
            self.report(def.span, "transform comptime arity mismatch");
            return;
        }
        let ct: BTreeMap<_, _> = def
            .comptime
            .iter()
            .zip(&instance.arguments)
            .map(|(p, v)| (p.name.clone(), v.clone()))
            .collect();
        if !self.check_comptime(&def, &ct, def.span) {
            return;
        }
        def.returns = def.returns.substitute(&ct);
        def.children_type = def.children_type.as_ref().map(|t| t.substitute(&ct));
        for p in &mut def.parameters {
            p.ty = p.ty.substitute(&ct);
        }
        if !def.returns.runtime()
            || def.parameters.iter().any(|p| !p.ty.data())
            || def
                .children_type
                .as_ref()
                .is_some_and(|t| t.level().is_none())
        {
            self.report(
                def.span,
                "runtime positions require concrete data/content types; Type is restricted to <>",
            );
            return;
        }
        self.ensure_instance(instance, &def, &ct);
    }
    fn type_name(name: &str) -> Option<Type> {
        Some(match name {
            "Type" => Type::Type,
            "InlineContent" => Type::InlineContent,
            "BlockContent" => Type::BlockContent,
            "Unit" => Type::Unit,
            "Bool" => Type::Bool,
            "Int" => Type::Int,
            "Float" => Type::Float,
            "String" => Type::String,
            "Dict" => Type::Dict,
            "Array" => Type::Array(None),
            _ => return None,
        })
    }
    fn reference(&mut self, name: &str, span: TextRange) -> TypedExpr {
        if let Some(v) = self.scope.get(name) {
            let mut value = v.clone();
            value.span = span;
            value.source = self.source.clone();
            return value;
        }
        if let Some(t) = Self::type_name(name) {
            return self.constant(ComptimeValue::Type(t), span);
        }
        self.failed(span, format!("unbound reference `{name}`"))
    }
    pub fn check(&mut self, e: &Expr) -> TypedExpr {
        let span = e.span();
        match e {
            Expr::Fragment { .. } => {
                self.failed(span, "content fragments require a content sequence")
            }
            Expr::Literal(v, _) => self.constant(ComptimeValue::Data(v.clone()), span),
            Expr::Ref { name, .. } => self.reference(name, span),
            Expr::Content {
                forest,
                annotations,
                body,
                ..
            } => {
                let (t, _) = self.content(forest, annotations, *body);
                self.typed(Some(content_type(*body)), ExprKind::Content(t), None, span)
            }
            Expr::Block {
                bindings, value, ..
            } => {
                let old = self.scope.clone();
                let mut checked = Vec::new();
                for (name, e) in bindings {
                    let t = self.check(e);
                    if self.scope.contains_key(name) {
                        self.report(e.span(), format!("duplicate binding `{name}`"));
                    }
                    let binding = self.typed(
                        t.ty.clone(),
                        ExprKind::Input(name.clone()),
                        t.known.clone(),
                        e.span(),
                    );
                    self.scope.insert(name.clone(), binding);
                    if t.ty != Some(Type::Type) {
                        checked.push((name.clone(), t));
                    }
                }
                let val = self.check(value);
                self.scope = old;
                let known = if checked.iter().all(|(_, v)| v.known.is_some()) {
                    val.known.clone()
                } else {
                    None
                };
                self.typed(
                    val.ty.clone(),
                    ExprKind::Block(checked, Box::new(val.clone())),
                    known,
                    span,
                )
            }
            Expr::If {
                condition, yes, no, ..
            } => {
                let c = self.check(condition);
                self.expect(&c, &Type::Bool);
                if let Some(ComptimeValue::Data(Value::Bool(c))) = c.known {
                    return self.check(if c { yes } else { no });
                }
                let y = self.check(yes);
                let n = self.check(no);
                let ty = self.join(&y, &n, span);
                self.typed(
                    ty,
                    ExprKind::If(Box::new(c), Box::new(y), Box::new(n)),
                    None,
                    span,
                )
            }
            Expr::Match { subject, arms, .. } => {
                let subject = self.check(subject);
                if subject.known.is_some() {
                    for (pattern, body) in arms {
                        if matches!(pattern,Expr::Ref{name,..} if name=="_") {
                            return self.check(body);
                        }
                        let p = self.check(pattern);
                        if p.known.is_none() {
                            self.report(
                                pattern.span(),
                                "match patterns must be known at compile time",
                            );
                        }
                        let condition = self.binary("==", subject.clone(), p, pattern.span());
                        if condition.known == Some(ComptimeValue::Data(Value::Bool(true))) {
                            return self.check(body);
                        }
                    }
                    return self.failed(span, "non-exhaustive comptime match");
                }
                let mut result = None;
                let mut wildcard = false;
                for (pattern, body) in arms.iter().rev() {
                    let value = self.check(body);
                    if matches!(pattern,Expr::Ref{name,..} if name=="_") {
                        result = Some(value);
                        wildcard = true;
                        continue;
                    }
                    let p = self.check(pattern);
                    if p.known.is_none() {
                        self.report(
                            pattern.span(),
                            "match patterns must be known at compile time",
                        );
                    }
                    self.expect(&p, subject.ty.as_ref().unwrap_or(&Type::Unit));
                    if let Some(rest) = result {
                        let ty = self.join(&value, &rest, span);
                        let cond = self.binary("==", subject.clone(), p, span);
                        result = Some(self.typed(
                            ty,
                            ExprKind::If(Box::new(cond), Box::new(value), Box::new(rest)),
                            None,
                            span,
                        ));
                    } else {
                        result = Some(value);
                    }
                }
                if !wildcard {
                    self.report(span, "runtime match requires a final `_` arm");
                }
                result.unwrap_or_else(|| self.failed(span, "empty match"))
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                let l = self.check(left);
                let r = self.check(right);
                self.binary(op, l, r, span)
            }
            Expr::Unary { op, value, .. } => {
                let v = self.check(value);
                let ty = if op == "!" {
                    Type::Bool
                } else {
                    v.ty.clone().unwrap_or(Type::Int)
                };
                self.expect(&v, &ty);
                if op == "-" && !matches!(v.ty, Some(Type::Int | Type::Float) | None) {
                    self.report(span, "unary minus requires Int or Float");
                }
                let known = v
                    .known
                    .as_ref()
                    .and_then(|v| super::eval::unary(op, v).ok());
                self.typed(
                    Some(ty),
                    ExprKind::Unary(op.clone(), Box::new(v)),
                    known,
                    span,
                )
            }
            Expr::Comptime { value, .. } => {
                let t = self.check(value);
                if let Some(v) = t.known {
                    return self.constant(v, span);
                }
                match super::eval::constant(&t, &self.program, self.context) {
                    Ok(v) => self.constant(v, span),
                    Err(m) => self.failed(span, format!("comptime expression: {m}")),
                }
            }
            Expr::Array(values, _) => {
                let values: Vec<_> = values.iter().map(|v| self.check(v)).collect();
                let ty = values.first().and_then(|v| v.ty.clone());
                for v in &values {
                    if let Some(t) = &ty {
                        self.expect(v, t);
                    }
                    if v.ty
                        .as_ref()
                        .is_some_and(|t| !t.runtime() || t.level().is_some())
                    {
                        self.report(v.span, "arrays contain data values only");
                    }
                }
                let known = values
                    .iter()
                    .map(|v| match &v.known {
                        Some(ComptimeValue::Data(v)) => Some(v.clone()),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(|v| ComptimeValue::Data(Value::Array(v)));
                self.typed(
                    Some(Type::Array(ty.map(Box::new))),
                    ExprKind::Array(values),
                    known,
                    span,
                )
            }
            Expr::Dict(values, _) => {
                let values: Vec<_> = values
                    .iter()
                    .map(|(k, v)| (k.clone(), self.check(v)))
                    .collect();
                for (_, v) in &values {
                    if v.ty
                        .as_ref()
                        .is_some_and(|t| !t.runtime() || t.level().is_some())
                    {
                        self.report(v.span, "dicts contain data values only");
                    }
                }
                let known = values
                    .iter()
                    .map(|(k, v)| match &v.known {
                        Some(ComptimeValue::Data(v)) => Some((k.clone(), v.clone())),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(|v| ComptimeValue::Data(Value::Dict(v.into_iter().collect())));
                self.typed(Some(Type::Dict), ExprKind::Dict(values), known, span)
            }
            Expr::Call {
                callee,
                args,
                comptime,
                body,
                ..
            } => {
                let Expr::Ref { name, .. } = callee.as_ref() else {
                    return self.failed(span, "function values are not supported");
                };
                if name == "Array"
                    && args.is_empty()
                    && comptime.len() == 1
                    && *body == BodyFlavor::None
                {
                    let t = self.check(comptime[0].value());
                    return match t.known {
                        Some(ComptimeValue::Type(t)) => {
                            self.constant(ComptimeValue::Type(Type::Array(Some(Box::new(t)))), span)
                        }
                        _ => self.failed(span, "Array<T> requires a known Type"),
                    };
                }
                if name == "compile_error" {
                    let message = args.first().map(|a| self.check(a.value()));
                    return match message.and_then(|v| v.known) {
                        Some(ComptimeValue::Data(Value::Str(m))) => self.failed(span, m),
                        _ => self.failed(span, "compile_error requires a known String"),
                    };
                }
                // Direct content calls use exactly the same resolution, mount
                // validation and shaped Items as literal interpolations.
                self.failures.push(Vec::new());
                let r = self.prepare_call(e, false);
                let failures = self.failures.pop().unwrap();
                let r = crate::shape::shape_inline(vec![r]).remove(0);
                let node = finish(&r, &self.source);
                match node.pending.as_deref().cloned() {
                    Some(PendingEvaluation::Call(mut call))
                        if call.definition.returns.level().is_none() =>
                    {
                        if let Some(frame) = self.failures.last_mut() {
                            frame.extend(failures);
                        }
                        call.children = call.definition.children_type.as_ref().map(|ty| {
                            Box::new(self.typed(
                                Some(ty.clone()),
                                ExprKind::Content(node.children),
                                None,
                                span,
                            ))
                        });
                        self.typed(
                            Some(call.definition.returns.clone()),
                            ExprKind::ValueCall(Box::new(call)),
                            None,
                            span,
                        )
                    }
                    Some(PendingEvaluation::Failure { ty, message })
                        if ty.as_ref().is_none_or(|t| t.level().is_none()) =>
                    {
                        if let Some(frame) = self.failures.last_mut() {
                            frame.extend(failures);
                        }
                        self.typed(ty, ExprKind::Failure(message), None, span)
                    }
                    _ => self.typed(
                        Some(if node.level == Level::Inline {
                            Type::InlineContent
                        } else {
                            Type::BlockContent
                        }),
                        ExprKind::Content(vec![node]),
                        None,
                        span,
                    ),
                }
            }
        }
    }
    fn join(&mut self, a: &TypedExpr, b: &TypedExpr, span: TextRange) -> Option<Type> {
        match (&a.ty, &b.ty) {
            (Some(a), Some(b)) if a != b => {
                self.report(
                    span,
                    format!("branches return different types: {a} and {b}"),
                );
                None
            }
            (Some(t), _) | (_, Some(t)) => Some(t.clone()),
            _ => None,
        }
    }
    fn binary(&mut self, op: &str, l: TypedExpr, r: TypedExpr, span: TextRange) -> TypedExpr {
        if let (Some(a), Some(b)) = (&l.ty, &r.ty) {
            if a != b {
                self.report(
                    span,
                    format!("operator `{op}` requires equal operand types"),
                );
            }
        }
        let compare = matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=");
        let allowed = match l.ty.as_ref() {
            Some(Type::Type) => matches!(op, "==" | "!="),
            Some(Type::Bool) => matches!(op, "==" | "!=" | "&&" | "||"),
            Some(Type::Int | Type::Float) => !matches!(op, "&&" | "||"),
            Some(Type::String) => compare || op == "+",
            _ => false,
        };
        if !allowed {
            self.report(
                span,
                format!("operator `{op}` is not defined for this type"),
            );
        }
        let known = l
            .known
            .as_ref()
            .zip(r.known.as_ref())
            .and_then(|(a, b)| super::eval::binary(op, a, b).ok());
        self.typed(
            if compare || matches!(op, "&&" | "||") {
                Some(Type::Bool)
            } else {
                l.ty.clone()
            },
            ExprKind::Binary(op.into(), Box::new(l), Box::new(r)),
            known,
            span,
        )
    }

    fn definition(&self, name: &str) -> Option<FunctionDef> {
        if !name.contains("::") && !self.package.is_empty() {
            self.context
                .registry()
                .function(&self.package, name)
                .or_else(|| self.context.registry().resolve(name).ok())
                .cloned()
        } else {
            self.context.registry().resolve(name).ok().cloned()
        }
    }
    fn invocation(
        &mut self,
        name: &str,
        args: &[CallArg],
        ct: &[CallArg],
        children: Option<Box<TypedExpr>>,
        attrs: Dict,
        span: TextRange,
    ) -> Result<Invocation, CallFailure> {
        self.failures.push(Vec::new());
        let result = self.bind_invocation(name, args, ct, children, attrs, span);
        let failures = self.failures.pop().unwrap();
        if let Some(frame) = self.failures.last_mut() {
            frame.extend(failures.iter().cloned());
        }
        result.map(|mut call| {
            call.invalid = failures.first().map(|d| d.message.clone());
            call
        })
    }
    fn bind_invocation(
        &mut self,
        name: &str,
        args: &[CallArg],
        ct: &[CallArg],
        children: Option<Box<TypedExpr>>,
        attrs: Dict,
        span: TextRange,
    ) -> Result<Invocation, CallFailure> {
        let def = self.definition(name);
        let Some(def) = def else {
            self.report(
                span,
                if name.matches("::").count() > 1 {
                    format!("module paths are not supported yet: `{name}`")
                } else {
                    format!("unknown constructor or function `{name}`")
                },
            );
            return Err(CallFailure {
                definition: None,
                fields: Dict::default(),
                instance: None,
                message: format!("unknown constructor or function `{name}`"),
            });
        };
        let mut bindings = BTreeMap::new();
        let mut pos = 0;
        for arg in ct {
            let param = match arg {
                CallArg::Positional(_) => {
                    let p = def.comptime.get(pos);
                    pos += 1;
                    p
                }
                CallArg::Named { name, .. } => def.comptime.iter().find(|p| p.name == *name),
            };
            let value = self.check(arg.value());
            let known = value
                .known
                .clone()
                .or_else(|| super::eval::constant(&value, &self.program, self.context).ok());
            match (param, known) {
                (Some(p), Some(v)) => {
                    if bindings.insert(p.name.clone(), v).is_some() {
                        self.report(span, "duplicate comptime argument");
                    }
                }
                (None, _) => self.report(span, "unknown or excess comptime argument"),
                (_, None) => self.report(
                    arg.value().span(),
                    "comptime argument depends on a runtime value",
                ),
            }
        }
        let mut arguments = BTreeMap::new();
        let mut pos = 0;
        for arg in args {
            let param = match arg {
                CallArg::Positional(_) => {
                    let p = def.parameters.iter().filter(|p| p.positional).nth(pos);
                    pos += 1;
                    p
                }
                CallArg::Named { name, .. } => def.parameters.iter().find(|p| p.name == *name),
            };
            if self.package.is_empty() && !markup_literal(arg.value()) {
                self.report(
                    arg.value().span(),
                    "standalone Markup arguments must be data literals",
                );
            }
            let value = self.check(arg.value());
            if let Some(p) = param {
                if arguments.insert(p.name.clone(), value).is_some() {
                    self.report(
                        arg.value().span(),
                        format!("argument `{}` supplied more than once", p.name),
                    );
                }
            } else {
                self.report(
                    span,
                    match arg {
                        CallArg::Positional(_) => {
                            format!("too many positional arguments for `{name}`")
                        }
                        CallArg::Named { name: key, .. } => {
                            format!("unknown argument `{key}` for `{name}`")
                        }
                    },
                );
            }
        }
        let mut inference_errors = Vec::new();
        for p in &def.parameters {
            if let Some(v) = arguments.get(&p.name) {
                if let Some(t) = &v.ty {
                    infer(&p.ty, t, &mut bindings, span, &mut inference_errors);
                }
            }
        }
        if let (Some(t), Some(v)) = (&def.children_type, &children) {
            if let Some(actual) = &v.ty {
                infer(t, actual, &mut bindings, span, &mut inference_errors);
            }
        }
        for error in inference_errors {
            self.report(error.span, error.message);
        }
        for p in &def.comptime {
            if !bindings.contains_key(&p.name) {
                if let Some(v) = &p.default {
                    bindings.insert(p.name.clone(), v.clone());
                } else {
                    self.report(
                        span,
                        format!("cannot infer comptime `{}`; supply it in <> ", p.name),
                    );
                    let mut definition = def.clone();
                    definition.returns = def.returns.substitute(&bindings);
                    definition.children_type =
                        def.children_type.as_ref().map(|t| t.substitute(&bindings));
                    return Err(CallFailure {
                        definition: Some(definition),
                        fields: known_fields(arguments.iter().map(|(k, v)| (k.as_str(), v))),
                        instance: None,
                        message: format!("cannot infer comptime `{}`; supply it in <>", p.name),
                    });
                }
            }
        }
        self.check_comptime(&def, &bindings, span);
        let instance = FunctionInstance {
            function: def.id.clone(),
            arguments: def
                .comptime
                .iter()
                .map(|p| bindings[&p.name].clone())
                .collect(),
        };
        let mut concrete = def.clone();
        concrete.returns = def.returns.substitute(&bindings);
        concrete.children_type = def.children_type.as_ref().map(|t| t.substitute(&bindings));
        for p in &mut concrete.parameters {
            p.ty = p.ty.substitute(&bindings);
        }
        if !concrete.returns.runtime()
            || concrete.parameters.iter().any(|p| !p.ty.data())
            || concrete
                .children_type
                .as_ref()
                .is_some_and(|t| !t.runtime() || t.level().is_none())
        {
            self.report(
                span,
                "Type is only permitted in <>; runtime positions require concrete types",
            );
            return Err(CallFailure {
                definition: Some(concrete),
                fields: known_fields(arguments.iter().map(|(k, v)| (k.as_str(), v))),
                instance: Some(instance),
                message: "Type is only permitted in <>; runtime positions require concrete types"
                    .into(),
            });
        }
        if !def.code && def.returns.level().is_none() && concrete.returns.level().is_none() {
            self.report(
                span,
                "native content category must be InlineContent or BlockContent",
            );
            return Err(CallFailure {
                definition: Some(concrete),
                fields: known_fields(arguments.iter().map(|(k, v)| (k.as_str(), v))),
                instance: Some(instance),
                message: "native content category must be InlineContent or BlockContent".into(),
            });
        }
        for p in &concrete.parameters {
            if let Some(v) = arguments.get(&p.name) {
                let type_matches = value_fits(v, &p.ty);
                if v.ty.is_some() && !type_matches {
                    self.report(
                        v.span,
                        format!("`{name}.{}` must be {}", p.name, p.expected()),
                    );
                }
                if let Some(ComptimeValue::Data(v)) = &v.known {
                    if type_matches && !p.accepts(v) {
                        self.report(
                            span,
                            format!("`{name}.{}` must be {}", p.name, p.expected()),
                        );
                    }
                }
            } else {
                match &p.mode {
                    ParameterMode::Default(v) => {
                        let t = self.constant(ComptimeValue::Data(v.clone()), span);
                        self.expect(&t, &p.ty);
                        arguments.insert(p.name.clone(), t);
                    }
                    ParameterMode::Required => {
                        self.report(span, format!("`{name}` requires `{}`", p.name))
                    }
                    ParameterMode::Optional => {}
                }
            }
        }
        match (&concrete.children_type, &children) {
            (None, Some(_)) => self.report(span, format!("`{name}` takes no children")),
            (Some(ty), Some(v)) => self.expect(v, ty),
            _ => {}
        }
        self.ensure_instance(&instance, &concrete, &bindings);
        Ok(Invocation {
            instance,
            definition: concrete,
            arguments: arguments.into_iter().collect(),
            children,
            attrs,
            span,
            source: self.source.clone(),
            invalid: None,
        })
    }

    fn check_comptime(
        &mut self,
        def: &FunctionDef,
        bindings: &BTreeMap<String, ComptimeValue>,
        span: TextRange,
    ) -> bool {
        let start = self.diagnostics.len();
        for p in &def.comptime {
            let expected = p.ty.substitute(bindings);
            let matches = match &bindings[&p.name] {
                ComptimeValue::Data(value) => expected.accepts(value),
                ComptimeValue::Type(_) => expected == Type::Type,
            };
            if !matches {
                self.report(span, format!("comptime `{}` requires {expected}", p.name));
            }
            if let ComptimeValue::Data(v) = &bindings[&p.name] {
                if !p.constraint.accepts(v) {
                    self.report(
                        span,
                        format!(
                            "comptime `{}` must satisfy {}",
                            p.name,
                            match p.constraint {
                                notist_core::definitions::ValueConstraint::PositiveInt =>
                                    "positive integer",
                                _ => "its value constraint",
                            }
                        ),
                    );
                }
            }
        }
        self.diagnostics.len() == start
    }

    fn ensure_instance(
        &mut self,
        instance: &FunctionInstance,
        def: &FunctionDef,
        ct: &BTreeMap<String, ComptimeValue>,
    ) {
        let key = instance.key();
        if self.program.instances.contains_key(&key) {
            return;
        }
        if self.depth >= 32 || self.program.instances.len() >= 4096 {
            self.report(def.span, "function instantiation limit exceeded");
            return;
        }
        self.program.instances.insert(
            key.clone(),
            InstanceRecord {
                definition: def.clone(),
                body: None,
                diagnostics: Vec::new(),
                checking: true,
            },
        );
        if !def.code {
            self.program.instances.get_mut(&key).unwrap().checking = false;
            return;
        }
        let body = self.context.program().bodies.get(&def.id).cloned();
        let Some(body) = body else {
            self.report(def.span, "missing Code implementation");
            return;
        };
        let old_scope = std::mem::take(&mut self.scope);
        let old_package = std::mem::replace(&mut self.package, def.id.package.clone());
        let old_source = std::mem::replace(&mut self.source, body.source.clone());
        for (name, v) in ct {
            let value = self.constant(v.clone(), def.span);
            self.scope.insert(name.clone(), value);
        }
        for p in &def.parameters {
            let value = self.typed(
                Some(p.ty.clone()),
                ExprKind::Input(p.name.clone()),
                None,
                p.span,
            );
            self.scope.insert(p.name.clone(), value);
        }
        if let Some(ty) = &def.children_type {
            let value = self.typed(
                Some(ty.clone()),
                ExprKind::Input("children".into()),
                None,
                def.span,
            );
            self.scope.insert("children".into(), value);
        }
        let start = self.diagnostics.len();
        self.failures.push(body.diagnostics.clone());
        self.diagnostics.extend(body.diagnostics);
        self.depth += 1;
        let checked = body.expression.as_ref().map(|e| self.check(e));
        self.depth -= 1;
        if let Some(t) = &checked {
            self.expect(t, &def.returns);
            if let ExprKind::Failure(m) = &t.kind {
                self.report(t.span, m);
            }
        }
        for d in &mut self.diagnostics[start..] {
            if d.source.is_none() {
                d.source = Some(self.source.clone());
            }
        }
        let diagnostics = self.failures.pop().unwrap();
        let record = self.program.instances.get_mut(&key).unwrap();
        record.body = checked;
        record.diagnostics = diagnostics;
        record.checking = false;
        self.scope = old_scope;
        self.package = old_package;
        self.source = old_source;
    }

    fn content(
        &mut self,
        forest: &[Expr],
        annotations: &[Annotation],
        body: BodyFlavor,
    ) -> (Vec<Item>, Vec<RExpr>) {
        let r: Vec<_> = forest.iter().map(|e| self.prepare(e)).collect();
        let mut shaped = if body == BodyFlavor::Block {
            crate::shape::shape(r)
        } else {
            crate::shape::shape_inline(r)
        };
        if body == BodyFlavor::Inline {
            for r in &shaped {
                if crate::shape::is_block_element(r) {
                    self.report(r.span, "InlineContent literal cannot contain BlockContent");
                }
            }
        }
        crate::attach::attach(&mut shaped, annotations, &mut self.diagnostics);
        (
            shaped.iter().map(|r| finish(r, &self.source)).collect(),
            shaped,
        )
    }
    fn prepare(&mut self, e: &Expr) -> RExpr {
        self.failures.push(Vec::new());
        let result = self.prepare_interpolation(e);
        self.failures.pop();
        result
    }
    fn prepare_interpolation(&mut self, e: &Expr) -> RExpr {
        let span = e.span();
        if let Expr::Fragment { children, .. } = e {
            let mut prepared: Vec<_> = children.iter().map(|e| self.prepare(e)).collect();
            if prepared.len() == 1 && crate::shape::is_block_element(&prepared[0]) {
                return prepared.pop().unwrap();
            }
            let invalid = prepared.iter().any(crate::shape::is_block_element);
            let mut paragraph = RExpr::new(Ctor::Paragraph, span).with_children(prepared);
            if invalid {
                let message = "a content fragment must be all inline or a single block expression; separate blocks with a blank line";
                self.report(span, message);
                paragraph.pending = Some(Box::new(PendingEvaluation::Failure {
                    ty: Some(Type::BlockContent),
                    message: message.into(),
                }));
            }
            return paragraph;
        }
        let Expr::Call {
            callee,
            children,
            body,
            attrs,
            ..
        } = e
        else {
            let value = self.check(e);
            let level = value.ty.as_ref().and_then(Type::level);
            if level.is_none() && value.ty.is_some() {
                self.report(
                    span,
                    "content interpolation requires InlineContent or BlockContent",
                );
            }
            let mut r = RExpr::new(Ctor::Interpolation, span);
            r.pending = Some(Box::new(PendingEvaluation::Splice(value)));
            r.level = level;
            return r;
        };
        let Expr::Ref { .. } = callee.as_ref() else {
            self.report(callee.span(), "content calls require a named function");
            let mut r = RExpr::new(Ctor::Custom("<call>".into()), span)
                .with_children(children.iter().map(|e| self.prepare(e)).collect());
            r.body = *body;
            r.attrs = attrs.clone();
            return r;
        };
        self.prepare_call(e, true)
    }
    fn prepare_call(&mut self, e: &Expr, interpolation: bool) -> RExpr {
        let Expr::Call {
            callee,
            args,
            comptime,
            children,
            body,
            attrs,
            span,
        } = e
        else {
            unreachable!()
        };
        let span = *span;
        let Expr::Ref { name, .. } = callee.as_ref() else {
            unreachable!()
        };
        // Signature inference uses the written mount category, never payload values.
        let definition = self.definition(name);
        let structural = definition
            .as_ref()
            .is_some_and(|d| matches!(d.children, Accepts::Items | Accepts::Rows | Accepts::Cells));
        // Rows, cells and list items form structural axes, not ordinary
        // paragraph/block content. Their separators do not create paragraphs.
        let prepared: Vec<_> = children
            .iter()
            .flat_map(|e| match e {
                Expr::Fragment { children, .. } if structural => children.as_slice(),
                e => std::slice::from_ref(e),
            })
            .map(|e| self.prepare(e))
            .collect();
        let flavor = if structural {
            BodyFlavor::Block
        } else if *body == BodyFlavor::None {
            if definition
                .as_ref()
                .is_some_and(|d| d.children == Accepts::Inline)
            {
                BodyFlavor::Inline
            } else {
                BodyFlavor::Block
            }
        } else {
            *body
        };
        let child = if *body != BodyFlavor::None || !children.is_empty() {
            Some(Box::new(self.typed(
                Some(content_type(flavor)),
                ExprKind::Content(Vec::new()),
                None,
                span,
            )))
        } else {
            None
        };
        let call = self.invocation(name, args, comptime, child, attrs.clone(), span);
        let mut call = match call {
            Ok(call) => call,
            Err(failure) => {
                let ty = failure
                    .definition
                    .as_ref()
                    .map(|d| d.returns.clone())
                    .filter(Type::runtime);
                let ctor = failure
                    .definition
                    .as_ref()
                    .map(|d| resolved_ctor(d))
                    .unwrap_or_else(|| Ctor::Custom(name.clone()));
                let mut r = RExpr::new(ctor, span).with_children(prepared);
                r.fields = failure.fields;
                r.instance = failure.instance;
                r.level = ty.as_ref().and_then(Type::level);
                r.body = *body;
                r.attrs = attrs.clone();
                r.pending = Some(Box::new(PendingEvaluation::Failure {
                    ty,
                    message: failure.message,
                }));
                return r;
            }
        };
        if interpolation && call.definition.returns.level().is_none() {
            let message = "content interpolation requires a content result";
            self.report(span, message);
            call.invalid = Some(message.into());
        }
        call.children = None;
        if call.definition.children_type == Some(Type::InlineContent)
            && prepared.iter().any(contains_block)
        {
            let message = format!("`{name}` takes inline content; block children are not allowed");
            self.report(span, &message);
            call.invalid = Some(message);
        }

        let mut fields = known_fields(call.arguments.iter().map(|(k, v)| (k.as_str(), v)));
        if call.definition.id == notist_core::definitions::FunctionId::new("notist", "heading") {
            if let Some(ComptimeValue::Data(v)) = call.instance.arguments.first() {
                fields.insert("level", v.clone());
            }
        }
        let level = call.definition.returns.level().unwrap_or(Level::Inline);
        let ctor = resolved_ctor(&call.definition);
        let mut r = RExpr::new(ctor, span).with_children(prepared);
        r.fields = fields;
        r.body = *body;
        r.attrs = attrs.clone();
        r.level = Some(level);
        r.instance = Some(call.instance.clone());
        if matches!(
            call.definition.children,
            Accepts::Rows | Accepts::Cells | Accepts::Items
        ) {
            let mut errors = Vec::new();
            crate::resolve::validate_structure(&mut r, &call.definition.id.name, &mut errors);
            if let Some(error) = errors.first() {
                call.invalid = Some(error.message.clone());
            }
            for error in errors {
                self.report(error.span, error.message);
            }
        }

        r.pending = Some(Box::new(PendingEvaluation::Call(call)));
        r
    }
}
fn same_type(actual: &Type, expected: &Type) -> bool {
    match (actual, expected) {
        (Type::Array(_), Type::Array(None)) => true,
        _ => actual == expected,
    }
}
// An erased Array does not prove its element type. Known empty (including
// nested empty) arrays can still be checked against a concrete annotation.
fn value_fits(value: &TypedExpr, expected: &Type) -> bool {
    match &value.known {
        Some(ComptimeValue::Data(v)) => expected.accepts(v),
        _ => value.ty.as_ref().is_some_and(|t| same_type(t, expected)),
    }
}
fn content_type(body: BodyFlavor) -> Type {
    if body == BodyFlavor::Inline {
        Type::InlineContent
    } else {
        Type::BlockContent
    }
}
fn infer(
    pattern: &Type,
    actual: &Type,
    bindings: &mut BTreeMap<String, ComptimeValue>,
    span: TextRange,
    diags: &mut Vec<Diagnostic>,
) {
    match (pattern, actual) {
        (Type::Parameter(name), t) => {
            let v = ComptimeValue::Type(t.clone());
            if let Some(prev) = bindings.get(name) {
                if prev != &v {
                    diags.push(Diagnostic::new(
                        Phase::Type,
                        span,
                        format!("conflicting inference for `{name}`"),
                    ));
                }
            } else {
                bindings.insert(name.clone(), v);
            }
        }
        (Type::Array(Some(p)), Type::Array(Some(a))) => infer(p, a, bindings, span, diags),
        _ => {}
    }
}

fn contains_block(r: &RExpr) -> bool {
    crate::shape::is_block_element(r)
        || r.ctor == Ctor::Node && r.children.iter().any(contains_block)
}

fn markup_literal(e: &Expr) -> bool {
    match e {
        Expr::Literal(..) => true,
        Expr::Array(v, _) => v.iter().all(markup_literal),
        Expr::Dict(v, _) => v.iter().all(|(_, v)| markup_literal(v)),
        _ => false,
    }
}

fn finish(r: &RExpr, source: &str) -> Item {
    fn attach_origin(item: &mut Item, source: &str) {
        item.origin = Some(Origin {
            source: source.into(),
            start: item.span.start().into(),
            end: item.span.end().into(),
            expansion: Vec::new(),
        });
        for child in &mut item.children {
            attach_origin(child, source);
        }
    }
    let mut item = crate::materialize::materialize(r);
    attach_origin(&mut item, source);
    item
}

fn resolved_ctor(definition: &FunctionDef) -> Ctor {
    if definition.id.package == "notist" {
        return Ctor::from_name(&definition.id.name).unwrap();
    }
    let accepts = match definition.children {
        Accepts::Any => match definition.children_type.as_ref().and_then(Type::level) {
            Some(Level::Inline) => Accepts::Inline,
            _ => Accepts::Block,
        },
        a => a,
    };
    Ctor::Extension(ExtensionCtor {
        id: definition.id.clone(),
        accepts,
        level: definition.returns.level().unwrap_or(Level::Inline),
    })
}

fn known_fields<'a>(arguments: impl Iterator<Item = (&'a str, &'a TypedExpr)>) -> Dict {
    arguments
        .filter_map(|(name, value)| match &value.known {
            Some(ComptimeValue::Data(v)) => Some((name.to_owned(), v.clone())),
            _ => None,
        })
        .collect()
}
