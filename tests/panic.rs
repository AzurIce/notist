use notist::{Context, Diagnostic, Evaluation, Item, Pipeline, Type, Value};

fn context(module: &str) -> Context {
    Context::default()
        .with_module(
            Pipeline::default()
                .compile_module("content.not", "abort", "abort/lib.notc", module)
                .unwrap(),
        )
        .unwrap()
}

fn diagnostics(module: &str) -> Vec<Diagnostic> {
    let pipeline = Pipeline::default();
    let module = match pipeline.compile_module("content.not", "abort", "abort/lib.notc", module) {
        Ok(module) => module,
        Err(errors) => return errors,
    };
    let context = match Context::default().with_module(module) {
        Ok(context) => context,
        Err(errors) => return errors,
    };
    pipeline.check_module("abort", &context)
}

fn text(root: &Item) -> String {
    root.descendants()
        .filter_map(|item| match item.fields.get("text") {
            Some(Value::Str(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

fn evaluate(module: &str, source: &str) -> Evaluation {
    let context = context(module);
    let pipeline = Pipeline::default();
    let errors = pipeline.check_module("abort", &context);
    assert!(errors.is_empty(), "{module}: {errors:?}");
    let analysis = pipeline.analyze("doc.not", source, &context).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{source}: {:?}",
        analysis.diagnostics()
    );
    pipeline.evaluate(&analysis, &context)
}

fn assert_error(module: &str, needle: &str) {
    let errors = diagnostics(module);
    assert!(
        errors.iter().any(|d| d.message.contains(needle)),
        "{module}: {errors:?}"
    );
}

#[test]
fn never_is_uninhabited_but_can_parameterize_data_containers() {
    assert!(Type::Never.data() && Type::Never.runtime() && Type::Never.concrete());
    assert_eq!(Type::Never.level(), None);
    assert_eq!(Type::Never.to_string(), "Never");
    for value in [
        Value::Unit,
        Value::Bool(false),
        Value::Int(1),
        Value::None,
        Value::Array(vec![].into()),
    ] {
        assert!(!Type::Never.accepts(&value));
        assert_ne!(Type::of(&value), Type::Never);
    }
    assert!(Type::Option(Box::new(Type::Never)).accepts(&Value::None));
    assert!(Type::Array(Some(Box::new(Type::Never))).accepts(&Value::Array(vec![].into())));
    let result = Type::Result(Box::new(Type::Never), Box::new(Type::String));
    assert!(result.accepts(&Value::Err(Box::new(Value::Str("error".into())))));
    assert!(!result.accepts(&Value::Ok(Box::new(Value::Unit))));
}

#[test]
fn panic_is_checked_without_running_and_reports_its_original_call_once() {
    let module = r#"
fn abort(message: String) -> Never { panic(message + "!") }
fn forward(message: String) -> Int { abort(message) }
fn show(message: String) -> InlineContent { [#text(text: if forward(message) == 1 { "yes" } else { "no" })] }
"#;
    let output = evaluate(module, "#self.show(\"runtime\")\n\nindependent");
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    let error = &output.diagnostics[0];
    assert_eq!(error.message, "runtime!");
    assert_eq!(error.source.as_deref(), Some("abort/lib.notc"));
    assert_eq!(
        module[usize::from(error.span.start())..usize::from(error.span.end())].trim(),
        "panic(message + \"!\")"
    );
    assert!(error.recovered);
    assert!(text(&output.root).contains("independent"));
    assert!(!text(&output.root).contains("yes"));
    let failed = output
        .root
        .find(|item| matches!(item.evaluation, Some(notist::EvaluationState::Error(_))))
        .unwrap();
    assert!(
        failed
            .origin
            .as_ref()
            .unwrap()
            .expansion
            .iter()
            .any(|location| location.source == "doc.not")
    );
    assert!(
        output
            .root
            .descendants()
            .all(|item| item.function_id() != Some(notist::FunctionId::new("notist", "panic")))
    );
}

#[test]
fn result_and_option_matches_join_never_with_normal_return_types() {
    let module = r#"
fn expect<T: Type>(value: Result<T, String>) -> T {
    match value { ok(v) => v, err(e) => panic(e) }
}
fn reversed<T: Type>(value: Result<T, String>) -> T {
    match value { err(e) => panic(e), ok(v) => v }
}
fn optional(value?: String) -> String {
    match value { none => panic("missing"), some(v) => v }
}
fn show(flag: Bool) -> InlineContent {
    let result = if flag { ok<String, String>("good") } else { err("bad") };
    [#text(text: expect(result) + reversed(result) + optional("provided"))]
}
"#;
    let good = evaluate(module, "#self.show(true)");
    assert!(good.diagnostics.is_empty(), "{:?}", good.diagnostics);
    assert_eq!(text(&good.root), "goodgoodprovided");
    let bad = evaluate(module, "#self.show(false)");
    assert_eq!(bad.diagnostics.len(), 1, "{:?}", bad.diagnostics);
    assert_eq!(bad.diagnostics[0].message, "bad");
}

#[test]
fn never_branches_can_join_both_content_categories_without_a_fake_value() {
    for (returns, content) in [("InlineContent", "[kept]"), ("BlockContent", "[ kept ]")] {
        for reverse in [false, true] {
            let yes = if reverse {
                "panic(\"branch\")"
            } else {
                content
            };
            let no = if reverse {
                content
            } else {
                "panic(\"branch\")"
            };
            let module = format!(
                "fn show(flag: Bool) -> {returns} {{ if flag {{ {yes} }} else {{ {no} }} }}"
            );
            let good = evaluate(
                &module,
                if reverse {
                    "#self.show(false)"
                } else {
                    "#self.show(true)"
                },
            );
            assert!(good.diagnostics.is_empty(), "{:?}", good.diagnostics);
            assert!(text(&good.root).contains("kept"));
            let bad = evaluate(
                &module,
                if reverse {
                    "#self.show(true)"
                } else {
                    "#self.show(false)"
                },
            );
            assert_eq!(bad.diagnostics.len(), 1, "{:?}", bad.diagnostics);
            assert_eq!(bad.diagnostics[0].message, "branch");
        }
    }
    assert_error(
        "fn bad(flag: Bool) -> InlineContent { if flag { [inline] } else { [ block ] } }",
        "branches return different types",
    );
}

#[test]
fn strict_expressions_propagate_divergence_and_stop_before_their_continuation() {
    for expression in [
        "{ let ignored = panic(\"stop\"); 1 }",
        "1 + panic(\"stop\")",
        "panic(\"stop\") + 1",
        "!panic(\"stop\")",
        "(panic(\"stop\"), 1)",
        "(first: panic(\"stop\"))",
        "some(panic(\"stop\"))",
        "ok<Int, String>(panic(\"stop\"))",
        "err<Int, String>(panic(\"stop\"))",
        "normal(panic(\"stop\"))",
        "if panic(\"stop\") { 1 } else { 2 }",
        "if true { panic(\"stop\") } else { panic(\"other\") }",
    ] {
        let module = format!(
            r#"
fn normal(value: Int) -> Int {{ value }}
fn stop() -> Never {{ {expression} }}
fn show() -> InlineContent {{ let blocked = stop(); [unreached] }}
"#
        );
        let output = evaluate(&module, "#self.show()");
        assert_eq!(
            output.diagnostics.len(),
            1,
            "{expression}: {:?}",
            output.diagnostics
        );
        assert_eq!(output.diagnostics[0].message, "stop");
        assert!(!text(&output.root).contains("unreached"));
    }
}

#[test]
fn logical_short_circuiting_does_not_run_an_unselected_panic() {
    let module = r#"
fn show(flag: Bool) -> InlineContent {
    let a = false && panic("and");
    let b = true || panic("or");
    if a || !b || (flag && panic("selected")) { [yes] } else { [no] }
}
"#;
    let good = evaluate(module, "#self.show(false)");
    assert!(good.diagnostics.is_empty(), "{:?}", good.diagnostics);
    assert_eq!(text(&good.root), "no");
    let bad = evaluate(module, "#self.show(true)");
    assert_eq!(bad.diagnostics.len(), 1, "{:?}", bad.diagnostics);
    assert_eq!(bad.diagnostics[0].message, "selected");
}

#[test]
fn never_has_empty_elimination_and_no_inhabited_return_or_container_conversion() {
    assert!(diagnostics("fn absurd(value: Never) -> String { match value {} }").is_empty());
    for module in [
        "fn bad() -> Never { 1 }",
        "fn bad() -> Never { () }",
        "fn bad(value: Never = ()) -> String { \"x\" }",
        "fn impossible(value: Never) -> String { \"x\" } fn bad() -> String { impossible(1) }",
        "fn bad() -> Result<Int, String> { err<Never, String>(\"bad\") }",
    ] {
        assert!(!diagnostics(module).is_empty(), "{module}");
    }
    assert_error(
        "fn bad() -> Never { let value = panic(\"stop\"); missing }",
        "unbound reference",
    );
}

#[test]
fn never_type_arguments_and_defaults_preserve_normal_signature_rules() {
    let module = r#"
fn abort<T: Type = Never>(message: String) -> T { panic(message) }
fn empty() -> Array<Never> { (,) }
fn absent() -> Option<Never> { none }
fn failed() -> Result<Never, String> { err("only error") }
fn identity<T: Type>(value: T) -> T { value }
fn show() -> InlineContent { let value = identity<Int>(abort("generic")); [unreached] }
"#;
    let output = evaluate(module, "#self.show()");
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    assert_eq!(output.diagnostics[0].message, "generic");
    assert_error(
        "fn identity<T: Type>(value: T) -> T { value } fn bad() -> Int { identity(panic(\"bad\")) }",
        "cannot infer comptime",
    );
}

#[test]
fn comptime_panic_reports_the_inner_call_and_unselected_runtime_calls_are_not_errors() {
    let module = r#"
fn abort() -> Never { panic("constant failure") }
fn bad() -> Int { comptime abort() }
"#;
    let errors = diagnostics(module);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].source.as_deref(), Some("abort/lib.notc"));
    assert_eq!(errors[0].message, "comptime expression: constant failure");
    assert_eq!(
        module[usize::from(errors[0].span.start())..usize::from(errors[0].span.end())].trim(),
        "panic(\"constant failure\")"
    );
    assert_error(
        "fn parameter<n: Int>() -> Int { n } fn bad() -> Int { parameter<panic(\"argument failure\")>() }",
        "comptime argument: argument failure",
    );
    assert!(
        diagnostics("fn never_called() -> Never { panic(\"not executed during check\") }")
            .is_empty()
    );
    assert_error(
        "fn bad(flag: Bool) -> Int { if flag { 1 } else { compile_error(\"check time\") } }",
        "check time",
    );
}

#[test]
fn never_interpolation_is_a_computation_not_a_content_constructor() {
    let pipeline = Pipeline::default();
    let context = context("fn show() -> InlineContent { [#panic(\"interpolation\")] }");
    for path in ["doc.not", "doc.md"] {
        let analysis = pipeline
            .analyze(path, "#self.show()\n\nkept", &context)
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{path}: {:?}",
            analysis.diagnostics()
        );
        assert!(
            analysis
                .root()
                .descendants()
                .all(|item| item.function_id() != Some(notist::FunctionId::new("notist", "panic")))
        );
        let output = pipeline.evaluate(&analysis, &context);
        assert_eq!(
            output.diagnostics.len(),
            1,
            "{path}: {:?}",
            output.diagnostics
        );
        assert_eq!(output.diagnostics[0].message, "interpolation");
        assert_eq!(
            output.diagnostics[0].source.as_deref(),
            Some("abort/lib.notc")
        );
        assert!(text(&output.root).contains("kept"));
    }
    let module = "fn show() -> InlineContent { [#panic(\"inside content\")] }";
    assert_eq!(
        evaluate(module, "#self.show()").diagnostics[0].message,
        "inside content"
    );
    // Never confers no row/item/cell role and cannot hide structural mistakes.
    assert_error(
        "fn bad() -> BlockContent { [ #table[ #panic(\"not a row\") ] ] }",
        "only row",
    );
    let analysis = pipeline
        .analyze("doc.not", "#panic(\"standalone\")", &context)
        .unwrap();
    assert!(
        analysis
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("content result"))
    );
}

#[test]
fn unmounted_content_stays_lazy_even_with_divergent_arguments() {
    let module = r#"
fn show() -> InlineContent {
    let first = [#panic("unused literal")];
    let second = text(text: panic("unused argument"));
    [kept]
}
"#;
    let output = evaluate(module, "#self.show()");
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(text(&output.root), "kept");
}

#[test]
fn panic_uses_the_registered_implementation_not_a_name_based_special_form() {
    assert_error(
        "fn panic(message: String) -> Never; fn show() -> InlineContent { panic(\"no implementation\") }",
        "native value declaration has no implementation",
    );
    let module = r#"
fn panic(message: String) -> String { message }
fn show() -> InlineContent { [#text(text: panic("ordinary function"))] }
"#;
    let output = evaluate(module, "#self.show()");
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(text(&output.root), "ordinary function");
    for (call, needle) in [
        ("panic()", "requires `message`"),
        ("panic(1)", "must be string"),
        ("panic(\"x\", \"y\")", "too many positional"),
        ("panic<Int>(\"x\")", "unknown or excess comptime"),
        ("panic(\"x\")[body]", "takes no children"),
    ] {
        assert_error(&format!("fn bad() -> Never {{ {call} }}"), needle);
    }
}

#[test]
fn panic_never_reads_after_failure_and_pending_inputs_are_not_diagnostic_values() {
    use notist::{EvaluationInputs, EvaluationOutcome, InputRead, ResourcePath};
    use std::{cell::Cell, sync::Arc};

    struct Inputs {
        response: InputRead,
        reads: Cell<usize>,
    }
    impl EvaluationInputs for Inputs {
        fn file(&self, _: &ResourcePath) -> InputRead {
            self.reads.set(self.reads.get() + 1);
            self.response.clone()
        }
    }
    let module = r#"
fn read_message(source: Path) -> Never { panic(read_text(source)) }
fn before_read(source: Path) -> String { let stop = panic("before read"); read_text(source) }
fn show(source: Path, before: Bool) -> InlineContent {
    if before { let value = before_read(source); [unreached] } else { read_message(source) }
}
"#;
    let context = context(module).with_resource_root("/project");
    let pipeline = Pipeline::default();
    let inputs = Inputs {
        response: InputRead::Ready(Arc::from(b"from input".as_slice())),
        reads: Cell::new(0),
    };
    for before in [true, false] {
        let source = format!("#self.show(p\"message.txt\", {before})");
        let analysis = pipeline.analyze("doc.not", &source, &context).unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let EvaluationOutcome::Complete(output) =
            pipeline.evaluate_with_inputs(&analysis, &context, &inputs)
        else {
            panic!("unexpected missing input")
        };
        assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
        assert_eq!(
            output.diagnostics[0].message,
            if before { "before read" } else { "from input" }
        );
        assert_eq!(inputs.reads.get(), if before { 0 } else { 1 });
    }
    let inputs = Inputs {
        response: InputRead::Pending,
        reads: Cell::new(0),
    };
    let analysis = pipeline
        .analyze("doc.not", "#self.show(p\"message.txt\", false)", &context)
        .unwrap();
    assert!(matches!(
        pipeline.evaluate_with_inputs(&analysis, &context, &inputs),
        EvaluationOutcome::NeedResources(_)
    ));
    assert_eq!(inputs.reads.get(), 1);
    // A Never-returning Code wrapper is not a new standalone Markup read entry.
    let denied = Inputs {
        response: InputRead::Pending,
        reads: Cell::new(0),
    };
    let analysis = pipeline
        .analyze("doc.not", "#self.read_message(p\"message.txt\")", &context)
        .unwrap();
    assert!(
        analysis
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("content result"))
    );
    assert!(matches!(
        pipeline.evaluate_with_inputs(&analysis, &context, &denied),
        EvaluationOutcome::Complete(_)
    ));
    assert_eq!(denied.reads.get(), 0);
    assert_error(
        "fn bad() -> Never { comptime panic(read_text(p\"x.txt\")) }",
        "not permitted during comptime",
    );
}

#[cfg(feature = "serde")]
#[test]
fn never_and_native_panic_round_trip_without_changing_legacy_function_metadata() {
    use notist::{FunctionImpl, NativeFunction};
    let definition = notist::builtins::registry()
        .function("notist", "panic")
        .unwrap()
        .clone();
    let mut encoded = serde_json::to_value(&definition).unwrap();
    assert_eq!(encoded["returns"], "Never");
    assert_eq!(encoded["code"], false);
    assert_eq!(
        encoded["implementation"],
        serde_json::json!({"Native":"Panic"})
    );
    let restored: notist::FunctionDef = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(restored, definition);
    assert_eq!(
        restored.implementation,
        FunctionImpl::Native(NativeFunction::Panic)
    );
    encoded["code"] = true.into();
    assert!(serde_json::from_value::<notist::FunctionDef>(encoded).is_err());
    let mut module = notist::DefinitionModule::new("fake");
    let mut fake = definition;
    fake.id = notist::FunctionId::new("fake", "panic");
    module.functions.push(fake);
    assert!(
        notist_core::definitions::validate_module(&module)
            .iter()
            .any(|d| d.message.contains("reserved"))
    );
}
