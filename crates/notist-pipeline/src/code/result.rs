//! Result constructors and exhaustive, single-evaluation pattern matching.
use super::{Checker, ComptimeValue, ExprKind, Type, TypedExpr, check::needs_hint};
use notist_core::{
    evaluation::{ResultArm, ResultPattern},
    expr::{BodyFlavor, CallArg, Expr},
    item::Value,
};
use rowan::TextRange;

impl Checker<'_> {
    fn data_type_arguments(
        &mut self,
        name: &str,
        args: &[CallArg],
        arity: usize,
    ) -> Result<Vec<Type>, String> {
        if args.len() != arity || args.iter().any(|a| !matches!(a, CallArg::Positional(_))) {
            return Err(format!("{name} requires {arity} positional type arguments"));
        }
        args.iter()
            .map(|arg| match self.check(arg.value()).known {
                Some(ComptimeValue::Type(t)) if t.data() => Ok(t),
                _ => Err(format!("{name} requires known data types in <>")),
            })
            .collect()
    }

    pub(super) fn check_container_type(&mut self, name: &str, e: &Expr) -> TypedExpr {
        let Expr::Call {
            args,
            comptime,
            children,
            body,
            ..
        } = e
        else {
            unreachable!()
        };
        let span = e.span();
        if !args.is_empty() || !children.is_empty() || *body != BodyFlavor::None {
            return self.failed(span, "type constructors take only type arguments in <>");
        }
        let types =
            match self.data_type_arguments(name, comptime, if name == "Result" { 2 } else { 1 }) {
                Ok(types) => types,
                Err(message) => return self.failed(span, message),
            };
        let t = match (name, types.as_slice()) {
            ("Array", [t]) => Type::Array(Some(Box::new(t.clone()))),
            ("Option", [t]) => Type::Option(Box::new(t.clone())),
            ("Result", [ok, err]) => Type::Result(Box::new(ok.clone()), Box::new(err.clone())),
            _ => unreachable!(),
        };
        let v = ComptimeValue::Type(t);
        self.typed(
            Some(Type::Type),
            ExprKind::Constant(v.clone()),
            Some(v),
            span,
        )
    }

    pub(super) fn check_result_constructor(
        &mut self,
        name: &str,
        e: &Expr,
        hint: Option<Type>,
    ) -> TypedExpr {
        let Expr::Call {
            args,
            comptime,
            children,
            body,
            ..
        } = e
        else {
            unreachable!()
        };
        let span = e.span();
        let [CallArg::Positional(arg)] = args.as_slice() else {
            return self.failed(
                span,
                format!("{name} requires exactly one positional argument"),
            );
        };
        if !children.is_empty() || *body != BodyFlavor::None {
            return self.failed(span, format!("{name} does not take content children"));
        }
        let ty = if comptime.is_empty() {
            match hint {
                Some(t @ Type::Result(..)) => t,
                _ => return self.failed(span, format!(
                    "cannot determine the {} type of `{name}`; use {name}<T, E>(value) or a Result<T, E> context",
                    if name == "ok" { "error" } else { "success" },
                )),
            }
        } else {
            match self.data_type_arguments(name, comptime, 2) {
                Ok(types) => Type::Result(Box::new(types[0].clone()), Box::new(types[1].clone())),
                Err(message) => return self.failed(span, message),
            }
        };
        let Type::Result(ok, err) = &ty else {
            unreachable!()
        };
        let payload = if name == "ok" { ok } else { err };
        let value = self.check_with_hint(arg, Some((**payload).clone()));
        self.expect(&value, payload);
        let known = value.known.clone().and_then(|v| match v {
            ComptimeValue::Data(v) => Some(ComptimeValue::Data(if name == "ok" {
                Value::Ok(Box::new(v))
            } else {
                Value::Err(Box::new(v))
            })),
            _ => None,
        });
        let kind = if name == "ok" {
            ExprKind::Ok(Box::new(value))
        } else {
            ExprKind::Err(Box::new(value))
        };
        let ty = if matches!(&kind, ExprKind::Ok(v) | ExprKind::Err(v) if v.ty == Some(Type::Never))
        {
            Type::Never
        } else {
            ty
        };
        self.typed(Some(ty), kind, known, span)
    }

    fn result_arm_body(
        &mut self,
        pattern: &ResultPattern,
        subject_type: &Type,
        body: &Expr,
        known_payload: Option<Value>,
        hint: Option<Type>,
    ) -> TypedExpr {
        let Type::Result(ok, err) = subject_type else {
            unreachable!()
        };
        let binding = match pattern {
            ResultPattern::Ok(name) => Some((name, ok)),
            ResultPattern::Err(name) => Some((name, err)),
            ResultPattern::Wildcard => None,
        }
        .filter(|(id, _)| {
            self.program
                .sources
                .bindings()
                .get(**id)
                .expect("checked pattern binding")
                .name
                != "_"
        });
        let previous = binding.map(|(id, ty)| {
            let name = self
                .program
                .sources
                .bindings()
                .get(*id)
                .expect("checked pattern binding")
                .name
                .clone();
            let known = known_payload.map(ComptimeValue::Data);
            let kind = known
                .clone()
                .map(ExprKind::Constant)
                .unwrap_or_else(|| ExprKind::Input(*id));
            let input = self.typed(Some((**ty).clone()), kind, known, body.span());
            (name.clone(), self.scope.insert(name, input))
        });
        let checked = self.check_with_hint(body, hint);
        if let Some((name, previous)) = previous {
            match previous {
                Some(value) => {
                    self.scope.insert(name.clone(), value);
                }
                None => {
                    self.scope.remove(&name);
                }
            }
        }
        checked
    }

    pub(super) fn check_result_match(
        &mut self,
        subject: TypedExpr,
        arms: &[(Expr, Expr)],
        hint: Option<Type>,
        span: TextRange,
    ) -> TypedExpr {
        let subject_type = subject.ty.as_ref().expect("Result subject");
        let mut patterns = Vec::new();
        let mut covered_ok = false;
        let mut covered_err = false;
        let mut wildcard = false;
        for (pattern, body) in arms {
            let Some(parsed) = result_pattern(pattern) else {
                return self.failed(
                    pattern.span(),
                    "Result patterns must be ok(name), err(name) or `_`",
                );
            };
            let duplicate = wildcard
                || match &parsed {
                    NamePattern::Ok(_) => std::mem::replace(&mut covered_ok, true),
                    NamePattern::Err(_) => std::mem::replace(&mut covered_err, true),
                    NamePattern::Wildcard => std::mem::replace(&mut wildcard, true),
                };
            if duplicate {
                self.report(pattern.span(), "unreachable Result match arm");
            }
            let resolved = match parsed {
                NamePattern::Ok(name) | NamePattern::Err(name) => {
                    let Some(id) = self.declare_in(&name, body.span(), body.span()) else {
                        return self.failed(span, "cannot allocate match binding");
                    };
                    if matches!(result_pattern(pattern), Some(NamePattern::Ok(_))) {
                        ResultPattern::Ok(id)
                    } else {
                        ResultPattern::Err(id)
                    }
                }
                NamePattern::Wildcard => ResultPattern::Wildcard,
            };
            patterns.push(resolved);
        }
        if arms.is_empty() {
            return self.failed(span, "empty match");
        }
        if !wildcard && !(covered_ok && covered_err) {
            self.report(
                span,
                "Result match requires both `ok(value)` and `err(error)` arms or a final `_` arm",
            );
        }
        // Preserve comptime branch pruning without deriving a payload type from
        // its raw Value: nested Results must retain both nominal parameters.
        let known = match &subject.known {
            Some(ComptimeValue::Data(Value::Ok(v))) => Some((true, &**v)),
            Some(ComptimeValue::Data(Value::Err(v))) => Some((false, &**v)),
            _ => None,
        };
        if let Some((ok, payload)) = known {
            for (pattern, (_, body)) in patterns.iter().zip(arms) {
                if matches!(pattern, ResultPattern::Wildcard)
                    || ok && matches!(pattern, ResultPattern::Ok(_))
                    || !ok && matches!(pattern, ResultPattern::Err(_))
                {
                    return self.result_arm_body(
                        pattern,
                        subject_type,
                        body,
                        Some(payload.clone()),
                        hint,
                    );
                }
            }
        }
        // With no outer annotation, a fully typed sibling may complete the
        // constructors in the remaining arms. No general unification is added.
        let mut checked: Vec<_> = patterns
            .iter()
            .zip(arms)
            .map(|(pattern, (_, body))| {
                if hint.is_none() && needs_hint(body) {
                    None
                } else {
                    Some(self.result_arm_body(pattern, subject_type, body, None, hint.clone()))
                }
            })
            .collect();
        let sibling = checked.iter().find_map(|body| {
            body.as_ref()
                .and_then(|b| b.ty.clone())
                .filter(|t| *t != Type::Never)
        });
        for ((checked, pattern), (_, body)) in checked.iter_mut().zip(&patterns).zip(arms) {
            if checked.is_none() {
                *checked =
                    Some(self.result_arm_body(pattern, subject_type, body, None, sibling.clone()));
            }
        }
        let mut joined = checked[0].as_ref().expect("checked Result arm").clone();
        let mut invalid = joined.ty.is_none();
        for body in checked.iter().skip(1).flatten() {
            joined.ty = self.join(&joined, body, span);
            invalid |= joined.ty.is_none();
        }
        let arms = patterns
            .into_iter()
            .zip(checked)
            .map(|(pattern, body)| ResultArm {
                pattern,
                body: body.expect("checked Result arm"),
            })
            .collect();
        self.typed(
            if invalid { None } else { joined.ty },
            ExprKind::MatchResult {
                subject: Box::new(subject),
                arms,
            },
            None,
            span,
        )
    }
}

enum NamePattern {
    Ok(String),
    Err(String),
    Wildcard,
}
fn result_pattern(e: &Expr) -> Option<NamePattern> {
    if matches!(e, Expr::Ref { name, .. } if name == "_") {
        return Some(NamePattern::Wildcard);
    }
    let Expr::Call {
        callee,
        args,
        comptime,
        children,
        body,
        ..
    } = e
    else {
        return None;
    };
    if !comptime.is_empty() || !children.is_empty() || *body != BodyFlavor::None {
        return None;
    }
    let [CallArg::Positional(Expr::Ref { name, .. })] = args.as_slice() else {
        return None;
    };
    match callee.name_path().as_deref()? {
        "ok" => Some(NamePattern::Ok(name.clone())),
        "err" => Some(NamePattern::Err(name.clone())),
        _ => None,
    }
}
