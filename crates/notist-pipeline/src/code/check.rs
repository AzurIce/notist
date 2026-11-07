use super::content::known_fields;
use super::*;
use crate::Context;
use notist_core::{
    annotation::{Annotation, LiteralDict, ResolvedAnnotation},
    definitions::{FunctionImpl, NativeFunction, ParameterMode},
    diag::Phase,
    evaluation::CallSite,
    expr::{BodyFlavor, CallArg, Expr},
    item::{Ctor, Origin, Value},
};
use rowan::TextRange;

type Scope = BTreeMap<String, TypedExpr>;
pub(super) struct CallTarget {
    pub(super) name: String,
    pub(super) definition: Option<FunctionDef>,
    pub(super) receiver: Option<TypedExpr>,
}
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
    resource_root: String,
    pub(super) site: CallSite,
    pub(super) depth: usize,
    // A function owns direct checking failures; interpolation scopes recover locally.
    pub(super) failures: Vec<Vec<Diagnostic>>,
    pub(super) content_annotations: Vec<Annotation>,
    /// Context for constructors whose complete type cannot be inferred locally.
    pub(super) type_hint: Option<Type>,
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
            resource_root: context.resource_root().into(),
            site: CallSite::Markup,
            depth: 0,
            failures: Vec::new(),
            content_annotations: Vec::new(),
            type_hint: None,
        }
    }
    pub(crate) fn for_content(context: &'a Context, source: &str) -> Self {
        let mut checker = Self::new(context, source);
        checker.module = context.root().unwrap_or("<content>").to_owned();
        checker.site = CallSite::Code;
        checker
    }
    pub(super) fn report(&mut self, span: TextRange, message: impl Into<String>) {
        let mut d = Diagnostic::new(Phase::Type, span, message);
        d.source = Some(self.source.clone());
        self.report_diagnostic(d);
    }
    fn report_diagnostic(&mut self, d: Diagnostic) {
        if let Some(frame) = self.failures.last_mut() {
            frame.push(d.clone());
        }
        self.diagnostics.push(d);
    }
    fn comptime_failed(
        &mut self,
        span: TextRange,
        label: &str,
        failure: notist_eval::Failure,
    ) -> TypedExpr {
        let message = format!("{label}: {}", failure.message);
        let (span, source) = failure.location.map_or_else(
            || (span, self.source.clone()),
            |location| {
                (
                    TextRange::new(location.start.into(), location.end.into()),
                    location.source.to_string(),
                )
            },
        );
        let mut diagnostic = Diagnostic::new(Phase::Type, span, &message);
        diagnostic.source = Some(source.clone());
        self.report_diagnostic(diagnostic);
        let mut expr = self.typed(None, ExprKind::Failure(message), None, span);
        expr.source = source;
        expr
    }
    pub(super) fn typed(
        &self,
        ty: Option<Type>,
        kind: ExprKind,
        known: Option<ComptimeValue>,
        span: TextRange,
    ) -> TypedExpr {
        TypedExpr {
            shape: notist_core::evaluation::ContentShape::of_expr(&kind),
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
    pub(super) fn failed(&mut self, span: TextRange, message: impl Into<String>) -> TypedExpr {
        let message = message.into();
        self.report(span, &message);
        self.typed(None, ExprKind::Failure(message), None, span)
    }
    pub(super) fn expect(&mut self, t: &TypedExpr, expected: &Type) {
        if let Some(actual) = &t.ty
            && !value_fits(t, expected)
        {
            self.report(t.span, format!("expected {expected}, found {actual}"));
        }
    }
    /// Hints never convert a value; they only complete constructor types.
    pub(super) fn check_with_hint(&mut self, e: &Expr, hint: Option<Type>) -> TypedExpr {
        let previous = std::mem::replace(&mut self.type_hint, hint);
        let result = self.check(e);
        self.type_hint = previous;
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
        attrs: LiteralDict,
    ) -> (Item, CheckedProgram, Vec<Diagnostic>, Vec<RExpr>) {
        let (children, shaped) = self.content(forest, annotations, BodyFlavor::Block);
        let mut doc = Item::new(Ctor::Doc, span);
        doc.attrs = self.resolve_attributes(&attrs);
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
        if let Some(linked) = self.context.program().module(module) {
            for body in linked.code().bodies() {
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
            .filter(|d| d.implementation.is_code() && module.is_none_or(|p| d.id.module == p))
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
            source: self.source.as_str().into(),
            start: span.start().into(),
            end: span.end().into(),
            expansion: Default::default(),
        }
    }
    pub(super) fn resolve_attributes(&mut self, attrs: &LiteralDict) -> Dict {
        let root = &self.resource_root;
        let mut diagnostics = Vec::new();
        let dict = super::literal::resolve_dict(attrs, root, &self.source, &mut diagnostics);
        for diagnostic in diagnostics {
            self.report(diagnostic.span, diagnostic.message);
        }
        dict
    }
    pub(super) fn resolve_annotations(
        &mut self,
        annotations: &[Annotation],
    ) -> Vec<ResolvedAnnotation> {
        annotations
            .iter()
            .map(|annotation| ResolvedAnnotation {
                dict: self.resolve_attributes(&annotation.dict),
                span: annotation.span,
                kind: annotation.kind,
            })
            .collect()
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
            "Never" => Type::Never,
            "Data" => Type::Data,
            "DataError" => Type::DataError,
            "InlineContent" => Type::InlineContent,
            "BlockContent" => Type::BlockContent,
            "Unit" => Type::Unit,
            "Bool" => Type::Bool,
            "Int" => Type::Int,
            "Float" => Type::Float,
            "String" => Type::String,
            "Path" => Type::Path,
            "ResourceTarget" => Type::ResourceTarget,
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
        // Only value-producing positions propagate the hint; conditions,
        // let initializers and unrelated calls cannot borrow a result type.
        let type_hint = self.type_hint.take();
        match e {
            Expr::Fragment { .. } => {
                self.failed(span, "content fragments require a content sequence")
            }
            Expr::Literal(v, _) => self.constant(ComptimeValue::Data(v.clone()), span),
            Expr::PathLiteral { .. } => {
                match super::literal::resolve_literal(e, &self.resource_root, &self.source) {
                    Ok(value) => self.constant(ComptimeValue::Data(value), span),
                    Err(error) => self.failed(error.span, error.message),
                }
            }
            Expr::Ref { name, .. } if name == "none" && !self.scope.contains_key("none") => {
                match type_hint {
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
                    let mut binding = self.typed(
                        t.ty.clone(),
                        ExprKind::Input(name.clone()),
                        t.known.clone(),
                        e.span(),
                    );
                    binding.shape = t.shape;
                    self.scope.insert(name.clone(), binding);
                    if t.ty != Some(Type::Type) {
                        checked.push((name.clone(), t));
                    }
                }
                let val = self.check_with_hint(value, type_hint);
                self.scope = old;
                let ty = if checked.iter().any(|(_, v)| v.ty == Some(Type::Never)) {
                    Some(Type::Never)
                } else {
                    val.ty.clone()
                };
                let known = if checked.iter().all(|(_, v)| v.known.is_some()) {
                    val.known.clone()
                } else {
                    None
                };
                self.typed(
                    ty,
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
                    return self.check_with_hint(if c { yes } else { no }, type_hint);
                }
                let (y, n) = if type_hint.is_some() {
                    (
                        self.check_with_hint(yes, type_hint.clone()),
                        self.check_with_hint(no, type_hint),
                    )
                } else if needs_hint(yes) && !needs_hint(no) {
                    let n = self.check(no);
                    let y = self.check_with_hint(yes, n.ty.clone());
                    (y, n)
                } else {
                    let y = self.check(yes);
                    let n = if needs_hint(no) {
                        self.check_with_hint(no, y.ty.clone())
                    } else {
                        self.check(no)
                    };
                    (y, n)
                };
                let joined = self.join(&y, &n, span);
                let ty = if c.ty == Some(Type::Never) {
                    Some(Type::Never)
                } else {
                    joined
                };
                self.typed(
                    ty,
                    ExprKind::If(Box::new(c), Box::new(y), Box::new(n)),
                    None,
                    span,
                )
            }
            Expr::Match { subject, arms, .. } => {
                let subject = self.check(subject);
                if subject.ty == Some(Type::Never) {
                    // No arm can be reached, including an empty elimination.
                    return subject;
                }
                if matches!(subject.ty, Some(Type::Result(..))) {
                    return self.check_result_match(subject, arms, type_hint, span);
                }
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
                            return self.check_with_hint(body, type_hint);
                        }
                        if let Some(name) = some_binding(pattern) {
                            if let Some(ComptimeValue::Data(Value::Some(value))) = &subject.known {
                                let value = ComptimeValue::Data((**value).clone());
                                let binding = self.typed(
                                    payload.clone(),
                                    ExprKind::Constant(value.clone()),
                                    Some(value),
                                    pattern.span(),
                                );
                                let old = self.scope.insert(name.clone(), binding);
                                let result = self.check_with_hint(body, type_hint);
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
                                return self.check_with_hint(body, type_hint);
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
                            return self.check_with_hint(body, type_hint);
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
                    let checked = if type_hint.is_none() && needs_hint(body) {
                        deferred.push((staged.len(), body, binding.clone()));
                        None
                    } else {
                        Some(self.check_arm_body(
                            body,
                            binding.as_deref(),
                            payload.as_ref(),
                            &subject,
                            pattern.span(),
                            type_hint.clone(),
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
                        .filter(|t| matches!(t, Type::Option(_) | Type::Result(..)))
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
            Expr::For {
                binding,
                subject,
                body,
                ..
            } => {
                let old_site = std::mem::replace(&mut self.site, CallSite::Code);
                let subject = self.check(subject);
                let element = match &subject.ty {
                    Some(Type::Array(Some(element))) => Some((**element).clone()),
                    _ => {
                        self.report(subject.span, "for requires a statically typed Array<E>; erased Array and Data are not iterable");
                        None
                    }
                };
                let previous = self.scope.insert(
                    binding.clone(),
                    self.typed(element, ExprKind::Input(binding.clone()), None, span),
                );
                let body = self.check_with_hint(body, type_hint.clone());
                match previous {
                    Some(value) => {
                        self.scope.insert(binding.clone(), value);
                    }
                    None => {
                        self.scope.remove(binding);
                    }
                }
                self.site = old_site;
                let ty = body.ty.clone().filter(|t| t.level().is_some()).or_else(|| {
                    (body.ty == Some(Type::Never))
                        .then_some(type_hint)
                        .flatten()
                        .filter(|t| t.level().is_some())
                });
                if ty.is_none() {
                    self.report(body.span, "for body must return InlineContent or BlockContent (including for empty input)");
                }
                self.typed(
                    ty,
                    ExprKind::For {
                        binding: binding.clone(),
                        subject: Box::new(subject),
                        body: Box::new(body),
                    },
                    None,
                    span,
                )
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
                if v.ty == Some(Type::Never) {
                    return self.typed(
                        Some(Type::Never),
                        ExprKind::Unary(op.clone(), Box::new(v)),
                        None,
                        span,
                    );
                }
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
                    .and_then(|v| notist_eval::unary(op, v).ok());
                self.typed(
                    Some(ty),
                    ExprKind::Unary(op.clone(), Box::new(v)),
                    known,
                    span,
                )
            }
            Expr::Comptime { value, .. } => {
                let t = self.check_with_hint(value, type_hint);
                let known = t
                    .known
                    .clone()
                    .map(Ok)
                    .unwrap_or_else(|| notist_eval::constant(&t, &self.program, self.context));
                match known {
                    Ok(v) => {
                        if let ComptimeValue::Data(value) = &v
                            && let Err(failure) = self.program.data_retention.observe(
                                value,
                                notist_core::data::DataLimits::default().working_bytes,
                            )
                        {
                            return self.failed(span, failure.to_string());
                        }
                        self.typed(t.ty, ExprKind::Constant(v.clone()), Some(v), span)
                    }
                    Err(failure) => self.comptime_failed(span, "comptime expression", failure),
                }
            }
            Expr::Array(values, _) => {
                let element_hint = match type_hint {
                    Some(Type::Array(element)) => element.map(|t| *t),
                    _ => None,
                };
                let values: Vec<_> = values
                    .iter()
                    .map(|v| self.check_with_hint(v, element_hint.clone()))
                    .collect();
                let ty = values
                    .iter()
                    .find_map(|v| v.ty.clone().filter(|t| *t != Type::Never))
                    .or(element_hint);
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
                    .map(|v| ComptimeValue::Data(Value::Array(v.into())));
                self.typed(
                    Some(if values.iter().any(|v| v.ty == Some(Type::Never)) {
                        Type::Never
                    } else {
                        Type::Array(ty.map(Box::new))
                    }),
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
                let ty = if values.iter().any(|(_, v)| v.ty == Some(Type::Never)) {
                    Type::Never
                } else {
                    Type::Dict
                };
                self.typed(Some(ty), ExprKind::Dict(values), known, span)
            }
            Expr::Call {
                callee,
                args,
                comptime,
                children,
                body,
                ..
            } => {
                let name = callee.name_path().unwrap_or_default();
                if matches!(name.as_str(), "Array" | "Option" | "Result") {
                    return self.check_container_type(&name, e);
                }
                if matches!(name.as_str(), "ok" | "err") {
                    return self.check_result_constructor(&name, e, type_hint);
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
                    let hint = match type_hint {
                        Some(Type::Option(t)) => Some(*t),
                        _ => None,
                    };
                    let value = self.check_with_hint(arg.value(), hint);
                    if let Some(t) = &value.ty
                        && !t.data()
                    {
                        self.report(value.span, "some requires a data value");
                    }
                    let ty = value.ty.clone().map(|t| {
                        if t == Type::Never {
                            Type::Never
                        } else {
                            Type::Option(Box::new(t))
                        }
                    });
                    let known = value.known.clone().and_then(|v| match v {
                        ComptimeValue::Data(v) => {
                            Some(ComptimeValue::Data(Value::Some(Box::new(v))))
                        }
                        _ => None,
                    });
                    return self.typed(ty, ExprKind::Some(Box::new(value)), known, span);
                }
                // Bind every function identically; only a Content result needs
                // shaping and an Item boundary. Data calls never become Ctors.
                self.failures.push(Vec::new());
                let result = self.compile_call(e);
                let failures = self.failures.pop().unwrap();
                if result.ty.as_ref().is_none_or(|ty| ty.level().is_none())
                    && let Some(frame) = self.failures.last_mut()
                {
                    frame.extend(failures);
                }
                result
            }
        }
    }
    pub(super) fn join(&mut self, a: &TypedExpr, b: &TypedExpr, span: TextRange) -> Option<Type> {
        match (&a.ty, &b.ty) {
            (Some(Type::Never), Some(t)) | (Some(t), Some(Type::Never)) => Some(t.clone()),
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
        let left_never = l.ty == Some(Type::Never);
        let right_never = r.ty == Some(Type::Never);
        if let (Some(a), Some(b)) = (&l.ty, &r.ty)
            && a != b
            && !left_never
            && !right_never
        {
            self.report(
                span,
                format!("operator `{op}` requires equal operand types"),
            );
        }
        let compare = matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=");
        let operand =
            l.ty.as_ref()
                .filter(|t| **t != Type::Never)
                .or_else(|| r.ty.as_ref().filter(|t| **t != Type::Never));
        let allowed = match operand {
            Some(Type::Type) => matches!(op, "==" | "!="),
            Some(Type::Bool) => matches!(op, "==" | "!=" | "&&" | "||"),
            Some(Type::Int | Type::Float) => !matches!(op, "&&" | "||"),
            Some(Type::String) => compare || op == "+",
            Some(
                Type::Option(_) | Type::Result(..) | Type::Path | Type::Data | Type::DataError,
            ) => matches!(op, "==" | "!="),
            None if left_never || right_never => true,
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
            .and_then(|(a, b)| notist_eval::binary(op, a, b).ok());
        self.typed(
            if left_never || right_never && !matches!(op, "&&" | "||") {
                Some(Type::Never)
            } else if compare || matches!(op, "&&" | "||") {
                Some(Type::Bool)
            } else {
                operand.cloned()
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
    pub(super) fn call_target(&mut self, callee: &Expr) -> CallTarget {
        let name = callee.name_path().unwrap_or_else(|| "<method>".into());
        if let Some(definition) = self.definition(&name) {
            return CallTarget {
                name,
                definition: Some(definition),
                receiver: None,
            };
        }
        if let Expr::Member {
            base, name: method, ..
        } = callee
        {
            let receiver = self.check(base);
            let definition = match (&receiver.ty, method.as_str()) {
                (Some(Type::Array(_)), "len") => self
                    .context
                    .registry()
                    .function("notist.array", "len")
                    .cloned(),
                _ => None,
            };
            if definition.is_none() {
                self.report(
                    callee.span(),
                    format!("no method `{method}` for this receiver type"),
                );
            }
            return CallTarget {
                name,
                definition,
                receiver: Some(receiver),
            };
        }
        CallTarget {
            name,
            definition: None,
            receiver: None,
        }
    }
    pub(super) fn invocation(
        &mut self,
        target: CallTarget,
        args: &[CallArg],
        ct: &[CallArg],
        children: Option<Box<TypedExpr>>,
        attrs: Dict,
        span: TextRange,
    ) -> Result<Invocation, Box<CallFailure>> {
        self.failures.push(Vec::new());
        let result = self.bind_invocation(target, args, ct, children, attrs, span);
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
        target: CallTarget,
        args: &[CallArg],
        ct: &[CallArg],
        children: Option<Box<TypedExpr>>,
        attrs: Dict,
        span: TextRange,
    ) -> Result<Invocation, Box<CallFailure>> {
        let CallTarget {
            name,
            definition: def,
            receiver,
        } = target;
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
            let hint = param
                .map(|p| p.ty.substitute(&bindings))
                .filter(Type::concrete);
            let value = self.check_with_hint(arg.value(), hint.clone());
            if let Some(hint) = &hint {
                self.expect(&value, hint);
            }
            let mut failed = false;
            let known = match value.known.clone() {
                Some(value) => Some(value),
                None => match notist_eval::constant(&value, &self.program, self.context) {
                    Ok(value) => Some(value),
                    Err(failure) => {
                        failed = true;
                        self.comptime_failed(arg.value().span(), "comptime argument", failure);
                        None
                    }
                },
            };
            match (param, known) {
                (Some(p), Some(v)) => {
                    if matches!(&v, ComptimeValue::Data(v) if v.contains_data_views()) {
                        self.report(span, "Data and DataError views cannot be comptime value arguments; decode ordinary values first");
                        return Err(Box::new(CallFailure {
                            definition: Some(def.clone()),
                            fields: Dict::default(),
                            instance: None,
                            message: "Data views are not comptime value arguments".into(),
                        }));
                    }
                    if bindings.insert(p.name.clone(), v).is_some() {
                        self.report(span, "duplicate comptime argument");
                    }
                }
                (None, _) => self.report(span, "unknown or excess comptime argument"),
                (_, None) if !failed => self.report(
                    arg.value().span(),
                    "comptime argument depends on a runtime value",
                ),
                _ => {}
            }
        }
        let mut arguments = BTreeMap::new();
        let has_receiver = receiver.is_some();
        if let Some(receiver) = receiver {
            arguments.insert("self".into(), receiver);
        }
        let mut pos = 0;
        for arg in args {
            let param = match arg {
                CallArg::Positional(_) => {
                    let p = def
                        .parameters
                        .iter()
                        .filter(|p| p.positional && !(has_receiver && p.name == "self"))
                        .nth(pos);
                    pos += 1;
                    p
                }
                CallArg::Named { name, .. } => def.parameters.iter().find(|p| p.name == *name),
            };
            if self.site == CallSite::Markup && !markup_literal(arg.value()) {
                self.report(
                    arg.value().span(),
                    "standalone Markup arguments must be data literals",
                );
            }
            // Optional arguments also accept an already wrapped Option<T>.
            let value = match param {
                Some(p) => {
                    let ty = p.ty.substitute(&bindings);
                    let hint = if p.mode == ParameterMode::Optional && !result_literal(arg.value())
                    {
                        Type::Option(Box::new(ty))
                    } else {
                        ty
                    };
                    self.check_with_hint(arg.value(), hint.concrete().then_some(hint))
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
        if def.implementation == FunctionImpl::Native(NativeFunction::Str)
            && !matches!(
                bindings.get("T"),
                Some(ComptimeValue::Type(
                    Type::Unit | Type::Bool | Type::Int | Type::Float | Type::String
                ))
            )
        {
            self.report(span, "str accepts only Unit, Bool, Int, Float or String");
        }
        if let FunctionImpl::Native(NativeFunction::Data(kind)) = def.implementation
            && kind.decodes()
        {
            if ct.is_empty() {
                self.report(span, "data decoding requires an explicit target type in <>");
            }
            if let Some(ComptimeValue::Type(t)) = bindings.get("T")
                && !notist_core::data::decode_target(t)
            {
                self.report(span, format!("unsupported Data decode target: {t}"));
            }
        }
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
        if matches!(def.implementation, FunctionImpl::Native(native) if native.reads_resources())
            && self.site != CallSite::Code
        {
            self.report(
                span,
                format!("{} can only be called from notc Code", def.id.name),
            );
        }
        if def.implementation == FunctionImpl::Native(NativeFunction::Construct)
            && def.returns.level().is_none()
            && concrete.returns.level().is_none()
        {
            self.report(span, "native value declaration has no implementation");
            return Err(Box::new(CallFailure {
                fields: known_fields(&concrete, arguments.iter().map(|(k, v)| (k.as_str(), v))),
                definition: Some(concrete),
                instance: Some(instance),
                message: "native value declaration has no implementation".into(),
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
                        // Raw variants cannot recover the absent Result type;
                        // registered defaults already have a declared type.
                        let v = ComptimeValue::Data(v.clone());
                        let t = self.typed(
                            Some(p.ty.clone()),
                            ExprKind::Constant(v.clone()),
                            Some(v),
                            span,
                        );
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
        if def.id == notist_core::definitions::FunctionId::new("notist", "embed")
            && arguments.get("target").is_some_and(|v| v.known.is_some())
            && ["query", "fragment"]
                .iter()
                .all(|name| arguments.get(*name).is_none_or(|v| v.known.is_some()))
        {
            let mut fields =
                known_fields(&concrete, arguments.iter().map(|(k, v)| (k.as_str(), v)));
            match notist_core::embed::normalize(&mut fields, &self.resource_root, &self.source) {
                Ok(()) => {
                    let target_span = arguments["target"].span;
                    for name in ["target", "query", "fragment"] {
                        if let Some(value) = fields.get(name) {
                            let span = arguments.get(name).map_or(target_span, |v| v.span);
                            arguments.insert(
                                name.into(),
                                self.constant(ComptimeValue::Data(value.clone()), span),
                            );
                        }
                    }
                }
                Err(message) => self.report(arguments["target"].span, message),
            }
        }
        let resource_root = concrete
            .parameters
            .iter()
            .any(|p| p.ty == Type::ResourceTarget)
            .then(|| self.resource_root.clone());
        self.ensure_instance(&instance, &concrete, &bindings);
        Ok(Invocation {
            instance,
            definition: concrete.into(),
            arguments: arguments.into_iter().collect(),
            children,
            attrs,
            span,
            source: self.source.clone(),
            site: self.site,
            resource_root,
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
            if matches!(&bindings[&p.name], ComptimeValue::Data(v) if v.contains_data_views()) {
                self.report(
                    span,
                    "Data and DataError views cannot be comptime value arguments",
                );
                continue;
            }
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
        // Only whole, successful plans starting from an empty program are
        // reused. This preserves instance/depth limits and avoids merging extra
        // instances from another document into a partially checked program.
        let cacheable = self.depth == 0
            && self.program.instances.is_empty()
            && self.program.data_retention.is_empty()
            && def.implementation.is_code();
        if cacheable && let Some(program) = self.context.checked_program(&key) {
            self.program = program;
            return;
        }
        let diagnostics_start = self.diagnostics.len();
        std::sync::Arc::make_mut(&mut self.program.instances).insert(
            key.clone(),
            std::sync::Arc::new(InstanceRecord {
                definition: def.clone(),
                body: None,
                diagnostics: Vec::new(),
                checking: true,
            }),
        );
        if !def.implementation.is_code() {
            std::sync::Arc::make_mut(
                std::sync::Arc::make_mut(&mut self.program.instances)
                    .get_mut(&key)
                    .unwrap(),
            )
            .checking = false;
            return;
        }
        let body = self.context.program().body(&def.id);
        let Some(body) = body else {
            self.report(def.span, "missing Code implementation");
            return;
        };
        let old_scope = std::mem::take(&mut self.scope);
        let old_module = std::mem::replace(&mut self.module, def.id.module.clone());
        let old_source = std::mem::replace(&mut self.source, body.source.clone());
        let old_site = std::mem::replace(&mut self.site, CallSite::Code);
        let old_resource_root =
            std::mem::replace(&mut self.resource_root, body.resource_root.clone());
        for (name, v) in ct {
            let mut value = self.constant(v.clone(), def.span);
            if let Some(p) = def.comptime.iter().find(|p| p.name == *name)
                && matches!(v, ComptimeValue::Data(_))
            {
                value.ty = Some(p.ty.substitute(ct));
            }
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
        self.diagnostics.extend(body.diagnostics.iter().cloned());
        self.depth += 1;
        let checked = body
            .expression
            .as_ref()
            .map(|e| self.check_with_hint(e, Some(def.returns.clone())));
        self.depth -= 1;
        if let Some(t) = &checked {
            self.expect(t, &def.returns);
        }
        for d in &mut self.diagnostics[start..] {
            if d.source.is_none() {
                d.source = Some(self.source.clone());
            }
        }
        let diagnostics = self.failures.pop().unwrap();
        let record = std::sync::Arc::make_mut(
            std::sync::Arc::make_mut(&mut self.program.instances)
                .get_mut(&key)
                .unwrap(),
        );
        record.body = checked;
        record.diagnostics = diagnostics;
        record.checking = false;
        self.scope = old_scope;
        self.module = old_module;
        self.source = old_source;
        self.site = old_site;
        self.resource_root = old_resource_root;
        if cacheable
            && self.diagnostics.len() == diagnostics_start
            && self.program.data_retention.is_empty()
            && self
                .program
                .instances
                .values()
                .all(|record| !record.checking && record.diagnostics.is_empty())
        {
            self.context.remember_checked_program(key, &self.program);
        }
    }
}
fn same_type(actual: &Type, expected: &Type) -> bool {
    match (actual, expected) {
        (Type::Path | Type::String, Type::ResourceTarget) => true,
        (Type::Array(_), Type::Array(None)) => true,
        _ => actual == expected,
    }
}
// An erased Array does not prove its element type. Known empty (including
// nested empty) arrays can still be checked against a concrete annotation.
fn value_fits(value: &TypedExpr, expected: &Type) -> bool {
    // Ex falso is an expression rule, not a conversion between inhabited
    // nominal containers. No Value can ever satisfy the Never type itself.
    if value.ty == Some(Type::Never) {
        return true;
    }
    // A known ok/err payload does not erase the other half of its nominal type.
    // In particular, an ok<Int, String> is not an ok<Int, Bool> just because
    // no error payload happens to be present in this value.
    fn result_identity(actual: &Type, expected: &Type) -> bool {
        match (actual, expected) {
            (Type::Result(..), Type::Result(..)) => actual == expected,
            (Type::Result(..), _) | (_, Type::Result(..)) => false,
            (Type::Option(a), Type::Option(b)) | (Type::Array(Some(a)), Type::Array(Some(b))) => {
                result_identity(a, b)
            }
            _ => true,
        }
    }
    if value
        .ty
        .as_ref()
        .is_some_and(|t| !result_identity(t, expected))
    {
        return false;
    }
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
    // A non-returning argument supplies no normal-value evidence for a type
    // parameter. Other arguments or explicit <> must determine it.
    if *actual == Type::Never {
        return;
    }
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
        (Type::Result(p_ok, p_err), Type::Result(a_ok, a_err)) => {
            infer(p_ok, a_ok, bindings, span, diags);
            infer(p_err, a_err, bindings, span, diags);
        }
        _ => {}
    }
}

fn markup_literal(e: &Expr) -> bool {
    match e {
        Expr::Literal(..) | Expr::PathLiteral { .. } => true,
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

pub(super) fn result_literal(e: &Expr) -> bool {
    match e {
        Expr::Call {
            callee, comptime, ..
        } => matches!(callee.name_path().as_deref(), Some("ok" | "err")) && comptime.is_empty(),
        Expr::Block { value, .. } | Expr::Comptime { value, .. } => result_literal(value),
        Expr::If { yes, no, .. } => result_literal(yes) || result_literal(no),
        _ => false,
    }
}

pub(super) fn needs_hint(e: &Expr) -> bool {
    none_literal(e) || result_literal(e)
}
