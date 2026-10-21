use super::content::known_fields;
use super::*;
use crate::Context;
use notist_core::{
    annotation::Annotation,
    builtins::Level,
    definitions::ParameterMode,
    diag::Phase,
    expr::{BodyFlavor, CallArg, Expr},
    item::{Ctor, Origin, Value},
};
use rowan::TextRange;

type Scope = BTreeMap<String, TypedExpr>;
pub(super) struct CallFailure {
    pub(super) definition: Option<FunctionDef>,
    pub(super) fields: Dict,
    pub(super) instance: Option<FunctionInstance>,
    pub(super) message: String,
}

pub(crate) struct Checker<'a> {
    pub(super) context: &'a Context,
    pub program: CheckedProgram,
    pub diagnostics: Vec<Diagnostic>,
    pub(super) scope: Scope,
    pub(super) module: String,
    pub(super) source: String,
    pub(super) depth: usize,
    // A function owns direct checking failures; interpolation scopes recover locally.
    pub(super) failures: Vec<Vec<Diagnostic>>,
    pub(super) content_annotations: Vec<Annotation>,
    /// Expected Option<T> for a bare `none` in one of its three typed positions.
    pub(super) none_hint: Option<Type>,
}
impl<'a> Checker<'a> {
    pub fn new(context: &'a Context, source: &str) -> Self {
        Self {
            context,
            program: Default::default(),
            diagnostics: Vec::new(),
            scope: BTreeMap::new(),
            module: String::new(),
            source: source.into(),
            depth: 0,
            failures: Vec::new(),
            content_annotations: Vec::new(),
            none_hint: None,
        }
    }
    pub(crate) fn for_content(context: &'a Context, source: &str) -> Self {
        let mut checker = Self::new(context, source);
        checker.module = context.root().unwrap_or("<content>").to_owned();
        checker
    }
    pub(super) fn report(&mut self, span: TextRange, message: impl Into<String>) {
        let mut d = Diagnostic::new(Phase::Type, span, message);
        d.source = Some(self.source.clone());
        if let Some(frame) = self.failures.last_mut() {
            frame.push(d.clone());
        }
        self.diagnostics.push(d);
    }
    pub(super) fn typed(
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
        if let Some(actual) = &t.ty
            && !value_fits(t, expected)
        {
            self.report(t.span, format!("expected {expected}, found {actual}"));
        }
    }
    /// Check with an expected Option<T> available for a bare `none`.
    fn check_with_hint(&mut self, e: &Expr, hint: Option<Type>) -> TypedExpr {
        let previous = std::mem::replace(&mut self.none_hint, hint);
        let result = self.check(e);
        self.none_hint = previous;
        result
    }
    /// Check a match arm body; a `some(name)` arm binds the payload first.
    fn check_arm_body(
        &mut self,
        body: &Expr,
        binding: Option<&str>,
        payload: Option<&Type>,
        subject: &TypedExpr,
        span: TextRange,
        hint: Option<Type>,
    ) -> TypedExpr {
        let bound = binding.map(|name| {
            let value = self.typed(
                payload.cloned(),
                ExprKind::Unwrap(Box::new(subject.clone())),
                None,
                span,
            );
            (name.to_owned(), self.scope.insert(name.into(), value))
        });
        let result = self.check_with_hint(body, hint);
        if let Some((name, previous)) = bound {
            match previous {
                Some(previous) => {
                    self.scope.insert(name, previous);
                }
                None => {
                    self.scope.remove(&name);
                }
            }
        }
        result
    }
    fn none_constant(&self, subject: &TypedExpr, span: TextRange) -> TypedExpr {
        self.typed(
            subject.ty.clone(),
            ExprKind::Constant(ComptimeValue::Data(Value::None)),
            Some(ComptimeValue::Data(Value::None)),
            span,
        )
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
    pub fn check_module(mut self, module: &str) -> Vec<Diagnostic> {
        for (id, body) in &self.context.program().bodies {
            if id.module == module {
                self.diagnostics.extend(body.diagnostics.iter().cloned());
            }
        }
        self.check_selected(Some(module));
        let mut unique = Vec::new();
        for diagnostic in self.diagnostics {
            if !unique.contains(&diagnostic) {
                unique.push(diagnostic);
            }
        }
        unique
    }
    fn check_selected(&mut self, module: Option<&str>) {
        let definitions: Vec<_> = self
            .context
            .registry()
            .functions()
            .filter(|d| d.code && module.is_none_or(|p| d.id.module == p))
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
    pub fn with_program(context: &'a Context, program: CheckedProgram) -> Self {
        let mut s = Self::new(context, "<evaluation>");
        s.program = program;
        s
    }
    pub fn install(&mut self, instance: &FunctionInstance) {
        let source = self
            .context
            .program()
            .source_path(&instance.function.module)
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
        // The none hint is consumed here; only the three typed positions
        // (optional argument, typed sibling branch, Option tail) reinstall it.
        let none_hint = self.none_hint.take();
        match e {
            Expr::Fragment { .. } => {
                self.failed(span, "content fragments require a content sequence")
            }
            Expr::Literal(v, _) => self.constant(ComptimeValue::Data(v.clone()), span),
            Expr::Ref { name, .. } if name == "none" && !self.scope.contains_key("none") => {
                match none_hint {
                    Some(ty @ Type::Option(_)) => self.typed(
                        Some(ty),
                        ExprKind::Constant(ComptimeValue::Data(Value::None)),
                        Some(ComptimeValue::Data(Value::None)),
                        span,
                    ),
                    _ => self.failed(span, "cannot determine the type of `none`"),
                }
            }
            Expr::Ref { name, .. } => self.reference(name, span),
            Expr::Member { .. } => self.failed(span, "module members must be called"),
            Expr::Content {
                forest,
                annotations,
                body,
                ..
            } => {
                let (forest, _) = self.content(forest, annotations, *body);
                self.typed(
                    Some(content_type(*body)),
                    ExprKind::Content(forest),
                    None,
                    span,
                )
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
                if none_literal(value) {
                    self.none_hint = none_hint;
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
                    return self.check_with_hint(if c { yes } else { no }, none_hint);
                }
                let (y, n) = if none_literal(yes) && !none_literal(no) {
                    let n = self.check(no);
                    let y = self.check_with_hint(yes, n.ty.clone());
                    (y, n)
                } else {
                    let y = self.check(yes);
                    let n = if none_literal(no) {
                        self.check_with_hint(no, y.ty.clone())
                    } else {
                        self.check(no)
                    };
                    (y, n)
                };
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
                let payload = match &subject.ty {
                    Some(Type::Option(payload)) => Some((**payload).clone()),
                    _ => None,
                };
                // Constructor patterns exist only under an Option subject.
                let some_binding = |pattern: &Expr| -> Option<String> {
                    payload.as_ref()?;
                    let Expr::Call {
                        callee,
                        args,
                        comptime,
                        children,
                        body,
                        ..
                    } = pattern
                    else {
                        return None;
                    };
                    if callee.name_path().as_deref() != Some("some")
                        || !comptime.is_empty()
                        || !children.is_empty()
                        || *body != BodyFlavor::None
                    {
                        return None;
                    }
                    match args.as_slice() {
                        [CallArg::Positional(Expr::Ref { name, .. })] => Some(name.clone()),
                        _ => None,
                    }
                };
                let none_pattern = |pattern: &Expr| {
                    payload.is_some() && matches!(pattern, Expr::Ref { name, .. } if name == "none")
                };
                if subject.known.is_some() {
                    for (pattern, body) in arms {
                        if matches!(pattern,Expr::Ref{name,..} if name=="_") {
                            return self.check_with_hint(body, none_hint);
                        }
                        if let Some(name) = some_binding(pattern) {
                            if let Some(ComptimeValue::Data(Value::Some(value))) = &subject.known {
                                let binding = self.constant(
                                    ComptimeValue::Data((**value).clone()),
                                    pattern.span(),
                                );
                                let old = self.scope.insert(name.clone(), binding);
                                let result = self.check_with_hint(body, none_hint);
                                match old {
                                    Some(previous) => {
                                        self.scope.insert(name, previous);
                                    }
                                    None => {
                                        self.scope.remove(&name);
                                    }
                                }
                                return result;
                            }
                            continue;
                        }
                        if none_pattern(pattern) {
                            if matches!(&subject.known, Some(ComptimeValue::Data(Value::None))) {
                                return self.check_with_hint(body, none_hint);
                            }
                            continue;
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
                            return self.check_with_hint(body, none_hint);
                        }
                    }
                    return self.failed(span, "non-exhaustive comptime match");
                }
                let mut wildcard = false;
                let mut covered_some = false;
                let mut covered_none = false;
                // Bare none bodies wait for a typed sibling branch; everything
                // else keeps the source-diagnostic order of plain matches.
                let mut staged: Vec<(Option<TypedExpr>, Option<TypedExpr>)> = Vec::new();
                let mut deferred: Vec<(usize, &Expr, Option<String>)> = Vec::new();
                for (pattern, body) in arms.iter().rev() {
                    let binding = some_binding(pattern);
                    let checked = if none_literal(body) {
                        deferred.push((staged.len(), body, binding.clone()));
                        None
                    } else {
                        Some(self.check_arm_body(
                            body,
                            binding.as_deref(),
                            payload.as_ref(),
                            &subject,
                            pattern.span(),
                            None,
                        ))
                    };
                    let cond = if matches!(pattern,Expr::Ref{name,..} if name=="_") {
                        wildcard = true;
                        None
                    } else if binding.is_some() {
                        covered_some = true;
                        Some(self.binary(
                            "!=",
                            subject.clone(),
                            self.none_constant(&subject, pattern.span()),
                            span,
                        ))
                    } else if none_pattern(pattern) {
                        covered_none = true;
                        Some(self.binary(
                            "==",
                            subject.clone(),
                            self.none_constant(&subject, pattern.span()),
                            span,
                        ))
                    } else {
                        let p = self.check(pattern);
                        if p.known.is_none() {
                            self.report(
                                pattern.span(),
                                "match patterns must be known at compile time",
                            );
                        }
                        self.expect(&p, subject.ty.as_ref().unwrap_or(&Type::Unit));
                        Some(self.binary("==", subject.clone(), p, span))
                    };
                    staged.push((cond, checked));
                }
                let sibling = staged.iter().find_map(|(_, body)| {
                    body.as_ref()
                        .and_then(|b| b.ty.clone())
                        .filter(|t| matches!(t, Type::Option(_)))
                });
                for (index, body, binding) in deferred {
                    let checked = self.check_arm_body(
                        body,
                        binding.as_deref(),
                        payload.as_ref(),
                        &subject,
                        body.span(),
                        sibling.clone(),
                    );
                    staged[index].1 = Some(checked);
                }
                let mut result = None;
                for (cond, body) in staged {
                    let body = body.expect("checked arm body");
                    result = Some(match cond {
                        None => body,
                        Some(cond) => match result {
                            None => body,
                            Some(rest) => {
                                let ty = self.join(&body, &rest, span);
                                self.typed(
                                    ty,
                                    ExprKind::If(Box::new(cond), Box::new(body), Box::new(rest)),
                                    None,
                                    span,
                                )
                            }
                        },
                    });
                }
                if !wildcard && !(covered_some && covered_none) {
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
                children,
                body,
                ..
            } => {
                let Some(name) = callee.name_path() else {
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
                if name == "some" {
                    let [arg] = args.as_slice() else {
                        return self.failed(span, "some requires exactly one argument");
                    };
                    if !comptime.is_empty() || !children.is_empty() || *body != BodyFlavor::None {
                        return self.failed(span, "some requires exactly one argument");
                    }
                    let value = self.check(arg.value());
                    if let Some(t) = &value.ty
                        && !t.data()
                    {
                        self.report(value.span, "some requires a data value");
                    }
                    let ty = value.ty.clone().map(|t| Type::Option(Box::new(t)));
                    let known = value.known.clone().and_then(|v| match v {
                        ComptimeValue::Data(v) => {
                            Some(ComptimeValue::Data(Value::Some(Box::new(v))))
                        }
                        _ => None,
                    });
                    return self.typed(ty, ExprKind::Some(Box::new(value)), known, span);
                }
                // Direct content calls use exactly the same resolution, mount
                // validation and shaped Items as literal interpolations.
                self.failures.push(Vec::new());
                let node = self.compile_call(e);
                let failures = self.failures.pop().unwrap();
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
        if let (Some(a), Some(b)) = (&l.ty, &r.ty)
            && a != b
        {
            self.report(
                span,
                format!("operator `{op}` requires equal operand types"),
            );
        }
        let compare = matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=");
        let allowed = match l.ty.as_ref() {
            Some(Type::Type) => matches!(op, "==" | "!="),
            Some(Type::Bool) => matches!(op, "==" | "!=" | "&&" | "||"),
            Some(Type::Int | Type::Float) => !matches!(op, "&&" | "||"),
            Some(Type::String) => compare || op == "+",
            Some(Type::Option(_)) => matches!(op, "==" | "!="),
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

    pub(super) fn definition(&self, name: &str) -> Option<FunctionDef> {
        let root = name.split('.').next().unwrap_or_default();
        let reserved_namespace = name.contains('.') && matches!(root, "notist" | "self");
        if !reserved_namespace && self.scope.contains_key(root) {
            return None;
        }
        self.context
            .resolve_in(
                (!self.module.is_empty()).then_some(self.module.as_str()),
                name,
            )
            .cloned()
    }
    pub(super) fn invocation(
        &mut self,
        name: &str,
        args: &[CallArg],
        ct: &[CallArg],
        children: Option<Box<TypedExpr>>,
        attrs: Dict,
        span: TextRange,
    ) -> Result<Invocation, Box<CallFailure>> {
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
    ) -> Result<Invocation, Box<CallFailure>> {
        let def = self.definition(name);
        let Some(def) = def else {
            self.report(span, format!("unknown constructor or function `{name}`"));
            return Err(Box::new(CallFailure {
                definition: None,
                fields: Dict::default(),
                instance: None,
                message: format!("unknown constructor or function `{name}`"),
            }));
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
            if self.module.is_empty() && !markup_literal(arg.value()) {
                self.report(
                    arg.value().span(),
                    "standalone Markup arguments must be data literals",
                );
            }
            // A bare none at a `?: T` boundary takes its type from the parameter.
            let value = match param {
                Some(p) if p.mode == ParameterMode::Optional && p.ty.concrete() => {
                    self.check_with_hint(arg.value(), Some(Type::Option(Box::new(p.ty.clone()))))
                }
                _ => self.check(arg.value()),
            };
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
            if let Some(v) = arguments.get(&p.name)
                && let Some(t) = &v.ty
            {
                infer(&p.ty, t, &mut bindings, span, &mut inference_errors);
            }
        }
        if let (Some(t), Some(v)) = (&def.children_type, &children)
            && let Some(actual) = &v.ty
        {
            infer(t, actual, &mut bindings, span, &mut inference_errors);
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
                    return Err(Box::new(CallFailure {
                        fields: known_fields(
                            &definition,
                            arguments.iter().map(|(k, v)| (k.as_str(), v)),
                        ),
                        definition: Some(definition),
                        instance: None,
                        message: format!("cannot infer comptime `{}`; supply it in <>", p.name),
                    }));
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
            return Err(Box::new(CallFailure {
                fields: known_fields(&concrete, arguments.iter().map(|(k, v)| (k.as_str(), v))),
                definition: Some(concrete),
                instance: Some(instance),
                message: "Type is only permitted in <>; runtime positions require concrete types"
                    .into(),
            }));
        }
        if !def.code && def.returns.level().is_none() && concrete.returns.level().is_none() {
            self.report(
                span,
                "native content category must be InlineContent or BlockContent",
            );
            return Err(Box::new(CallFailure {
                fields: known_fields(&concrete, arguments.iter().map(|(k, v)| (k.as_str(), v))),
                definition: Some(concrete),
                instance: Some(instance),
                message: "native content category must be InlineContent or BlockContent".into(),
            }));
        }
        for p in &concrete.parameters {
            if let Some(v) = arguments.get(&p.name) {
                // Forwarding: an Option<T> argument into a `?: T` parameter.
                let forwarded = p.mode == ParameterMode::Optional
                    && matches!(&v.ty, Some(Type::Option(t)) if same_type(t, &p.ty));
                let type_matches = forwarded || value_fits(v, &p.ty);
                if v.ty.is_some() && !type_matches {
                    self.report(
                        v.span,
                        format!("`{name}.{}` must be {}", p.name, p.expected()),
                    );
                }
                if let Some(ComptimeValue::Data(v)) = &v.known
                    && type_matches
                {
                    let payload = if forwarded {
                        match v {
                            Value::Some(payload) => Some(&**payload),
                            Value::None => None,
                            _ => Some(v),
                        }
                    } else {
                        Some(v)
                    };
                    if let Some(payload) = payload
                        && !p.accepts(payload)
                    {
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
            definition: Box::new(concrete),
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
            if let ComptimeValue::Data(v) = &bindings[&p.name]
                && !p.constraint.accepts(v)
            {
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
        let old_module = std::mem::replace(&mut self.module, def.id.module.clone());
        let old_source = std::mem::replace(&mut self.source, body.source.clone());
        for (name, v) in ct {
            let value = self.constant(v.clone(), def.span);
            self.scope.insert(name.clone(), value);
        }
        for p in &def.parameters {
            // Inside the body a `?: T` parameter is an Option<T> input.
            let ty = match p.mode {
                ParameterMode::Optional => Type::Option(Box::new(p.ty.clone())),
                _ => p.ty.clone(),
            };
            let value = self.typed(Some(ty), ExprKind::Input(p.name.clone()), None, p.span);
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
        let checked = body.expression.as_ref().map(|e| {
            let hint = match &def.returns {
                Type::Option(_) => Some(def.returns.clone()),
                _ => None,
            };
            self.check_with_hint(e, hint)
        });
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
        self.module = old_module;
        self.source = old_source;
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
pub(super) fn content_type(body: BodyFlavor) -> Type {
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
        (Type::Option(p), Type::Option(a)) => infer(p, a, bindings, span, diags),
        _ => {}
    }
}

fn markup_literal(e: &Expr) -> bool {
    match e {
        Expr::Literal(..) => true,
        Expr::Array(v, _) => v.iter().all(markup_literal),
        Expr::Dict(v, _) => v.iter().all(|(_, v)| markup_literal(v)),
        _ => false,
    }
}

/// A bare `none` reference, looking through block tails.
fn none_literal(e: &Expr) -> bool {
    match e {
        Expr::Ref { name, .. } => name == "none",
        Expr::Block { value, .. } => none_literal(value),
        _ => false,
    }
}
