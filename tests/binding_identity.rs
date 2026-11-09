use notist::{Context, Pipeline, Type};
use notist_core::evaluation::{ExprKind, TypedExpr};

fn visit(expression: &TypedExpr, ids: &mut Vec<notist::BindingId>) {
    match &expression.kind {
        ExprKind::Input(id) => ids.push(*id),
        ExprKind::Block(bindings, tail) => {
            for (id, value) in bindings {
                ids.push(*id);
                visit(value, ids);
            }
            visit(tail, ids);
        }
        ExprKind::For {
            binding,
            subject,
            body,
        } => {
            ids.push(*binding);
            visit(subject, ids);
            visit(body, ids);
        }
        ExprKind::Content(nodes) => {
            for node in nodes.iter() {
                for child in node.descendants() {
                    if let Some(expr) = child.as_expr() {
                        visit(expr, ids);
                    }
                }
            }
        }
        ExprKind::Call(call) => {
            for (_, value) in call.arguments() {
                visit(value, ids);
            }
            if let Some(children) = &call.children {
                visit(children, ids);
            }
        }
        ExprKind::Array(values) => {
            for value in values {
                visit(value, ids);
            }
        }
        ExprKind::Dict(values) => {
            for (_, value) in values {
                visit(value, ids);
            }
        }
        ExprKind::Unary(_, value)
        | ExprKind::Some(value)
        | ExprKind::Unwrap(value)
        | ExprKind::Ok(value)
        | ExprKind::Err(value) => visit(value, ids),
        ExprKind::Binary(_, a, b) => {
            visit(a, ids);
            visit(b, ids);
        }
        ExprKind::If(a, b, c) => {
            visit(a, ids);
            visit(b, ids);
            visit(c, ids);
        }
        ExprKind::MatchResult { subject, arms } => {
            visit(subject, ids);
            for arm in arms {
                visit(&arm.body, ids);
            }
        }
        ExprKind::Constant(_) | ExprKind::Failure(_) => {}
    }
}

#[test]
fn nested_loop_shadowing_has_distinct_numeric_binders_and_retains_outer_values() {
    let pipeline = Pipeline::default();
    let code = pipeline.compile_module("doc.not", "binding-loops", "loops.notc", "fn show(x: Int) -> InlineContent { [#text(text: str(x)) #for x in (2,3) [#text(text: str(x)) #for x in (4,5) [#text(text: str(x))] #text(text: str(x))] #text(text: str(x))] }").unwrap();
    let context = Context::default().with_module(code).unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.show(1)", &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let record = analysis
        .checked_program()
        .instances
        .values()
        .find(|record| record.definition().id.module == "binding-loops")
        .unwrap();
    let mut ids = Vec::new();
    visit(record.body.as_ref().unwrap(), &mut ids);
    let unique: std::collections::BTreeSet<_> = ids
        .into_iter()
        .filter(|id| analysis.sources().bindings().get(*id).unwrap().name == "x")
        .collect();
    assert_eq!(unique.len(), 3);
    for id in unique {
        assert_eq!(
            analysis
                .sources()
                .bindings()
                .get(id)
                .unwrap()
                .owner
                .as_ref(),
            Some(&record.definition().id)
        );
    }
    let result = pipeline.evaluate(&analysis, &context);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let text: String = result
        .root
        .descendants()
        .filter_map(|node| match node.field("text") {
            Some(notist::Value::Str(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text.replace(' ', ""), "1245234531");
}

#[test]
fn repeated_analysis_and_specialization_keep_live_lexical_identities() {
    let pipeline = Pipeline::default();
    let module = pipeline.compile_module("doc.not", "binding-repeat", "repeat.notc", "fn value<T: Type>(x: T) -> T { x } fn show() -> InlineContent { [#text(text: str(value(7)))] }").unwrap();
    let context = Context::default().with_module(module).unwrap();
    let a = pipeline
        .analyze("doc.not", "#self.show()", &context)
        .unwrap();
    let b = pipeline
        .analyze("doc.not", "#self.show()", &context)
        .unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    assert_eq!(a, b);
    let signature = context
        .registry()
        .signature(context.resolve("self.value").unwrap().id.def_id())
        .unwrap();
    assert!(signature.type_parameter("T").is_some());
    assert_eq!(size_of::<notist::BindingId>(), 4);
    assert_eq!(size_of::<notist::TypeParamId>(), 4);
}

#[test]
fn identical_source_addresses_in_different_functions_keep_owned_wire_identities() {
    use notist_core::{definitions::FunctionId, evaluation::ContentShape};
    let source = notist::Source::metadata("binding-owner.notc").unwrap();
    let mut sources = notist::Sources::from(source.clone());
    let a = FunctionId::new("binding-owner-a", "value");
    let b = FunctionId::new("binding-owner-b", "value");
    let span = notist::TextRange::new(1.into(), 2.into());
    let scope = notist::TextRange::new(0.into(), 20.into());
    let x = sources
        .bindings_mut()
        .declare_with_owner("x", &source, span, scope, Some(&a))
        .unwrap();
    let y = sources
        .bindings_mut()
        .declare_with_owner("x", &source, span, scope, Some(&b))
        .unwrap();
    assert_ne!(x, y);
    assert_eq!(
        sources
            .bindings_mut()
            .declare_with_owner("x", &source, span, scope, Some(&a))
            .unwrap(),
        x
    );
    let expression = TypedExpr {
        ty: Some(Type::Int),
        known: Some(notist::ComptimeValue::Data(notist::Value::Int(7))),
        kind: ExprKind::Input(x),
        span,
        source: source.id(),
        shape: ContentShape::Other,
    };
    let wire = serde_json::to_value(sources.wire(&expression)).unwrap();
    assert_eq!(wire["kind"]["Input"]["owner"]["module"], "binding-owner-a");
    assert_eq!(
        sources.decode::<TypedExpr>(wire.clone()).unwrap(),
        expression
    );
    let mut wrong_owner = wire.clone();
    wrong_owner["kind"]["Input"]["owner"]["module"] = "binding-owner-unknown".into();
    assert!(sources.decode::<TypedExpr>(wrong_owner).is_err());
    let imported = notist::Sources::import::<TypedExpr>(
        wire,
        notist::SourceNamespace::named("binding-owner-import"),
    )
    .unwrap();
    let ExprKind::Input(id) = imported.value.kind else {
        panic!("expected input")
    };
    assert_eq!(
        imported
            .sources
            .bindings()
            .get(id)
            .unwrap()
            .owner
            .as_ref()
            .unwrap()
            .module,
        "binding-owner-a"
    );
    let escaped = sources.clone();
    drop(sources);
    drop(a);
    drop(b);
    assert_eq!(
        escaped
            .bindings()
            .get(x)
            .unwrap()
            .owner
            .as_ref()
            .unwrap()
            .module,
        "binding-owner-a"
    );
}

#[test]
fn one_source_compiled_under_two_modules_has_distinct_checked_inputs() {
    let pipeline = Pipeline::default();
    let text = "fn value(x: Int) -> Int { let y = x; y }";
    let a = pipeline
        .compile_module("doc.not", "binding-module-a", "binding-shared.notc", text)
        .unwrap();
    let b = pipeline
        .compile_module("doc.not", "binding-module-b", "binding-shared.notc", text)
        .unwrap();
    let context = Context::default()
        .with_module(a)
        .unwrap()
        .with_module(b)
        .unwrap();
    let show = pipeline.compile_module("doc.not", "binding-module-caller", "binding-caller.notc", "import \"binding-shared-a.notc\": value as a; import \"binding-shared-b.notc\": value as b; fn show() -> InlineContent { [#text(text: str(a(1) + b(2)))] }").unwrap();
    let linked = context
        .link_module(show, ["binding-module-a".into(), "binding-module-b".into()])
        .unwrap();
    let context = context.with_linked_module(linked).unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.show()", &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let ids: Vec<_> = analysis
        .checked_program()
        .instances
        .values()
        .filter_map(|record| {
            record
                .named_inputs()
                .find(|(name, _)| *name == "x")
                .map(|(_, id)| id)
        })
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn lexical_wire_preserves_names_and_restores_shadowed_scopes() {
    use notist_core::evaluation::{ContentShape, TypedExpr};
    let source = notist::Source::metadata("wire-bindings.notc").unwrap();
    let mut sources = notist::Sources::from(source.clone());
    let span = |start: u32, end: u32| notist::TextRange::new(start.into(), end.into());
    let outer = sources
        .bindings_mut()
        .declare_scoped("x", &source, span(1, 2), span(0, 10))
        .unwrap();
    let inner = sources
        .bindings_mut()
        .declare_scoped("x", &source, span(4, 5), span(3, 9))
        .unwrap();
    let expr = |kind, span| TypedExpr {
        ty: Some(Type::Int),
        known: None,
        kind,
        span,
        source: source.id(),
        shape: ContentShape::Other,
    };
    let constant = |n| {
        expr(
            ExprKind::Constant(notist::ComptimeValue::Data(notist::Value::Int(n))),
            if n == 1 { span(1, 2) } else { span(4, 5) },
        )
    };
    let tree = expr(
        ExprKind::Block(
            vec![(outer, constant(1))],
            Box::new(expr(
                ExprKind::Block(
                    vec![(inner, constant(2))],
                    Box::new(expr(ExprKind::Input(inner), span(6, 7))),
                ),
                span(3, 9),
            )),
        ),
        span(0, 10),
    );
    let wire = serde_json::to_value(sources.wire(&tree)).unwrap();
    assert!(!wire.to_string().contains("BindingId"));
    assert_eq!(sources.decode::<TypedExpr>(wire.clone()).unwrap(), tree);
    let mut escaped = wire.clone();
    escaped["kind"]["Block"][1]["kind"]["Block"][1]["kind"]["Input"] =
        serde_json::to_value(sources.wire(&outer)).unwrap();
    assert!(
        sources.decode::<TypedExpr>(escaped).is_err(),
        "an inner reference must not escape to the shadowed outer declaration"
    );
    let imported = notist::Sources::import::<TypedExpr>(
        wire.clone(),
        notist::SourceNamespace::named("wire-bindings-import"),
    )
    .unwrap();
    imported
        .sources
        .validate_expression(&imported.value)
        .unwrap();
    assert_eq!(
        serde_json::to_value(imported.sources.wire(&imported.value)).unwrap(),
        wire
    );
    let free = expr(ExprKind::Input(outer), span(6, 7));
    assert!(
        sources
            .decode::<TypedExpr>(serde_json::to_value(sources.wire(&free)).unwrap())
            .is_err()
    );
    assert!(
        notist::Sources::from(source)
            .validate_expression(&tree)
            .is_err()
    );
}

#[test]
fn free_wire_references_require_exact_source_scope_and_explicit_environment() {
    use notist_core::evaluation::ContentShape;
    let source = notist::Source::metadata("binding-explicit.notc").unwrap();
    let other = notist::Source::metadata("binding-unrelated.notc").unwrap();
    let mut sources = notist::Sources::from(source.clone());
    sources.insert(other.clone()).unwrap();
    let span = |start: u32, end: u32| notist::TextRange::new(start.into(), end.into());
    let id = sources
        .bindings_mut()
        .declare_scoped("x", &source, span(1, 2), span(0, 20))
        .unwrap();
    let mut value = TypedExpr {
        ty: Some(Type::Int),
        known: None,
        kind: ExprKind::Input(id),
        source: source.id(),
        span: span(10, 11),
        shape: ContentShape::Other,
    };
    let environment = std::collections::BTreeMap::from([("x".to_string(), id)]);
    let wire = serde_json::to_value(sources.wire(&value)).unwrap();
    assert!(
        sources.decode::<TypedExpr>(wire.clone()).is_err(),
        "unique spelling is not a lexical environment"
    );
    assert_eq!(
        sources
            .decode_with_bindings::<TypedExpr>(wire.clone(), &environment)
            .unwrap(),
        value
    );
    assert!(
        notist::Sources::import::<TypedExpr>(
            wire,
            notist::SourceNamespace::named("unknown-free-input")
        )
        .is_err()
    );
    value.source = other.id();
    let wire = serde_json::to_value(sources.wire(&value)).unwrap();
    assert!(
        sources
            .decode_with_bindings::<TypedExpr>(wire, &environment)
            .is_err(),
        "an explicit environment does not bypass declaration source ownership"
    );
    value.source = source.id();
    value.span = span(30, 31);
    let wire = serde_json::to_value(sources.wire(&value)).unwrap();
    assert!(
        sources
            .decode_with_bindings::<TypedExpr>(wire, &environment)
            .is_err(),
        "an explicit environment does not bypass lexical scope"
    );
}

#[test]
fn result_arm_payloads_shadow_parameters_and_wire_uses_explicit_free_bindings() {
    let pipeline = Pipeline::default();
    let module = pipeline.compile_module("doc.not", "binding-match", "match.notc", "fn show(x: Int, flag: Bool) -> InlineContent { let result = if flag { ok<Int, Int>(2) } else { err<Int, Int>(3) }; let value = match result { ok(x) => if true { x } else { 0 }, err(x) => x }; [#text(text: str(value))#text(text: str(x))] }").unwrap();
    let context = Context::default().with_module(module).unwrap();
    for (flag, expected) in [(true, "21"), (false, "31")] {
        let analysis = pipeline
            .analyze("doc.not", &format!("#self.show(1, {flag})"), &context)
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let record = analysis
            .checked_program()
            .instances
            .values()
            .find(|record| record.definition().id.module == "binding-match")
            .unwrap();
        let body = record.body.as_ref().unwrap();
        let wire = serde_json::to_value(analysis.sources().wire(body)).unwrap();
        assert_eq!(
            analysis
                .sources()
                .decode_with_bindings::<TypedExpr>(
                    wire,
                    &record
                        .named_inputs()
                        .map(|(name, id)| (name.to_owned(), id))
                        .collect()
                )
                .unwrap(),
            *body
        );
        let result = pipeline.evaluate(&analysis, &context);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let text: String = result
            .root
            .descendants()
            .filter_map(|node| match node.field("text") {
                Some(notist::Value::Str(text)) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, expected);
    }
}
