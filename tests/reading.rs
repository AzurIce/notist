use notist::{
    Context, Evaluation, EvaluationInputs, EvaluationOutcome, InputError, InputRead, Item,
    Pipeline, ResourcePath, Value,
};
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

const MODULE: &str = r#"
fn contents(source: Path) -> String { read_text(source) }
fn from_file(source: Path = p"./default.txt") -> BlockContent {
    let body = contents(source);
    [ #text(text: body) ]
}
fn forward(source: Path) -> BlockContent { from_file(source) }
fn own() -> BlockContent { from_file(p"./own.txt") }
fn branch(source: Path, enabled: Bool) -> BlockContent {
    if enabled { from_file(source) } else { [ skipped ] }
}
fn unused(source: Path) -> BlockContent { let hidden = from_file(source); [ unused ] }
fn generic<message: String>() -> BlockContent { [#text(text: message)] }
fn comptime_read(source: Path) -> BlockContent { generic<contents(p"./own.txt")>() }
"#;

fn context() -> Context {
    Context::default()
        .with_resource_root("/project")
        .with_module(
            Pipeline::default()
                .compile_module_with_root(
                    "content.not",
                    "view",
                    "/project/views/lib.notc",
                    MODULE,
                    "/project",
                )
                .unwrap(),
        )
        .unwrap()
}

#[derive(Default)]
struct Inputs {
    files: BTreeMap<String, InputRead>,
    requested: RefCell<Vec<ResourcePath>>,
}
impl EvaluationInputs for Inputs {
    fn file(&self, path: &ResourcePath) -> InputRead {
        self.requested.borrow_mut().push(path.clone());
        self.files
            .get(&path.logical_path())
            .cloned()
            .unwrap_or(InputRead::Failed(InputError::NotFound))
    }
}
fn text(root: &Item) -> String {
    root.descendants()
        .filter_map(|node| match node.fields.get("text") {
            Some(Value::Str(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}
fn evaluate(source: &str, inputs: &dyn EvaluationInputs) -> Evaluation {
    let context = context();
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("/project/pages/page.not", source, &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let EvaluationOutcome::Complete(evaluation) =
        pipeline.evaluate_with_inputs(&analysis, &context, inputs)
    else {
        panic!("unexpected pending input")
    };
    *evaluation
}

#[test]
fn read_text_uses_bound_paths_and_the_shared_code_call_model() {
    let inputs = Inputs {
        files: [
            ("pages/caller.txt", "caller"),
            ("views/default.txt", "default"),
            ("views/own.txt", "own"),
        ]
        .into_iter()
        .map(|(path, text)| {
            (
                format!("/project/{path}"),
                InputRead::Ready(Arc::from(text.as_bytes())),
            )
        })
        .collect(),
        ..Default::default()
    };
    for (source, expected) in [
        (r#"#self.from_file(p"./caller.txt")"#, "caller"),
        (r#"#self.forward(p"./caller.txt")"#, "caller"),
        ("#self.from_file()", "default"),
        ("#self.own()", "own"),
    ] {
        let output = evaluate(source, &inputs);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        assert!(text(&output.root).contains(expected));
        assert_eq!(output.dependencies.len(), 1);
    }
}

#[test]
fn checking_and_unused_content_do_not_read_inputs() {
    let inputs = Inputs::default();
    for source in [
        r#"#self.branch(p"./missing.txt", false)"#,
        r#"#self.unused(p"./missing.txt")"#,
    ] {
        let output = evaluate(source, &inputs);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        assert!(output.dependencies.is_empty());
    }
    assert!(inputs.requested.borrow().is_empty());
}

#[test]
fn missing_capability_and_resource_errors_use_the_read_call_source() {
    let context = context();
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("/project/pages/page.not", "#self.own()", &context)
        .unwrap();
    let pure = pipeline.evaluate(&analysis, &context);
    assert!(
        pure.diagnostics
            .iter()
            .any(|d| d.message.contains("capability is unavailable")),
        "{:?}",
        pure.diagnostics
    );
    let inputs = Inputs::default();
    let output = evaluate("#self.own()\n\nkept", &inputs);
    let error = output
        .diagnostics
        .iter()
        .find(|d| d.message.contains("resource not found"))
        .unwrap();
    assert_eq!(error.source.as_deref(), Some("/project/views/lib.notc"));
    assert_eq!(
        MODULE[usize::from(error.span.start())..usize::from(error.span.end())].trim(),
        "read_text(source)"
    );
    assert!(text(&output.root).contains("kept"));
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
            .any(|e| e.source == "/project/pages/page.not")
    );
    assert_eq!(output.dependencies.len(), 1);
}

#[test]
fn markup_and_indirect_comptime_cannot_read() {
    let pipeline = Pipeline::default();
    let context = context();
    for source in [
        r#"#read_text(p"./x")"#,
        r#"#text(text: read_text(p"./x"))"#,
        r#"#self.generic<self.contents(p"./x")>()"#,
        "#self.comptime_read(p\"./x\")",
    ] {
        let analysis = pipeline
            .analyze("/project/pages/page.not", source, &context)
            .unwrap();
        let output = pipeline.evaluate(&analysis, &context);
        let diagnostics = analysis
            .diagnostics()
            .iter()
            .chain(&output.diagnostics)
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            diagnostics.contains("read_text can only")
                || diagnostics.contains("literal arguments")
                || diagnostics.contains("not permitted during comptime"),
            "{source}: {diagnostics}"
        );
    }
    let analysis = pipeline
        .analyze(
            "/project/pages/page.not",
            "#self.comptime_read(p\"./x\")",
            &context,
        )
        .unwrap();
    assert!(
        analysis
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("not permitted during comptime")),
        "{:?}",
        analysis.diagnostics()
    );
}

#[test]
fn read_text_has_no_implicit_string_to_path_conversion() {
    let pipeline = Pipeline::default();
    let context = Context::default()
        .with_resource_root("/project")
        .with_module(
            pipeline
                .compile_module_with_root(
                    "content.not",
                    "view",
                    "/project/lib.notc",
                    "fn show() -> BlockContent { [ #text(text: read_text(\"data.txt\")) ] }",
                    "/project",
                )
                .unwrap(),
        )
        .unwrap();
    let analysis = pipeline
        .analyze("/project/page.not", "#self.show()", &context)
        .unwrap();
    assert!(
        analysis
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("must be Path")),
        "{:?}",
        analysis.diagnostics()
    );
    let inputs = Inputs::default();
    let _ = pipeline.evaluate_with_inputs(&analysis, &context, &inputs);
    assert!(inputs.requested.borrow().is_empty());
}

#[test]
fn pending_never_returns_a_partial_evaluation() {
    let inputs = Inputs {
        files: BTreeMap::from([("/project/views/own.txt".into(), InputRead::Pending)]),
        ..Default::default()
    };
    let pipeline = Pipeline::default();
    let context = context();
    let analysis = pipeline
        .analyze(
            "/project/pages/page.not",
            "before\n\n#self.own()\n\nafter",
            &context,
        )
        .unwrap();
    let EvaluationOutcome::NeedResources(paths) =
        pipeline.evaluate_with_inputs(&analysis, &context, &inputs)
    else {
        panic!("must prepare")
    };
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].logical_path(), "/project/views/own.txt");
}

#[test]
fn pending_does_not_run_output_extensions_but_complete_failures_do() {
    use notist::extensions::{DocumentContext, DocumentExtension, ExtensionResult};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Debug)]
    struct Output(Arc<AtomicUsize>);
    impl DocumentExtension for Output {
        fn name(&self) -> &str {
            "count-output"
        }
        fn after_evaluate(&self, _: &DocumentContext<'_>, _: &mut Item) -> ExtensionResult {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let context = context().with_extension(Output(calls.clone())).unwrap();
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("/project/pages/page.not", "#self.own()", &context)
        .unwrap();
    let inputs = Inputs {
        files: BTreeMap::from([("/project/views/own.txt".into(), InputRead::Pending)]),
        ..Default::default()
    };
    assert!(matches!(
        pipeline.evaluate_with_inputs(&analysis, &context, &inputs),
        EvaluationOutcome::NeedResources(_)
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(matches!(
        pipeline.evaluate_with_inputs(&analysis, &context, &Inputs::default()),
        EvaluationOutcome::Complete(_)
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn same_named_value_declarations_do_not_acquire_native_reading_implementations() {
    let pipeline = Pipeline::default();
    let context = Context::default().with_resource_root("/project").with_module(pipeline.compile_module_with_root(
        "content.not", "view", "/project/lib.notc", "fn read_text(source: Path) -> String; fn show() -> BlockContent { [ #text(text: read_text(p\"data\")) ] }", "/project",
    ).unwrap()).unwrap();
    let analysis = pipeline
        .analyze("/project/page.not", "#self.show()", &context)
        .unwrap();
    let inputs = Inputs::default();
    let output = pipeline.evaluate_with_inputs(&analysis, &context, &inputs);
    assert!(matches!(output, EvaluationOutcome::Complete(_)));
    assert!(inputs.requested.borrow().is_empty());
    assert!(
        analysis
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("no implementation")),
        "{:?}",
        analysis.diagnostics()
    );
}

#[cfg(feature = "serde")]
#[test]
fn function_implementation_wire_preserves_legacy_code_flags_and_rejects_conflicts() {
    use notist::{FunctionDef, FunctionImpl, NativeFunction};
    for definition in notist::builtins::definitions().functions.into_iter().chain(
        context()
            .registry()
            .functions()
            .filter(|d| d.id.module == "view")
            .cloned(),
    ) {
        let wire = serde_json::to_value(&definition).unwrap();
        assert_eq!(wire["code"], definition.implementation.is_code());
        assert_eq!(
            serde_json::from_value::<FunctionDef>(wire.clone()).unwrap(),
            definition
        );
        if matches!(
            definition.implementation,
            FunctionImpl::Native(
                NativeFunction::ReadText | NativeFunction::Panic | NativeFunction::Data(_)
            )
        ) {
            assert!(wire.get("implementation").is_some());
        } else {
            assert!(wire.get("implementation").is_none());
        }
        let mut conflict = wire;
        conflict["implementation"] = if definition.implementation.is_code() {
            serde_json::json!({"Native": "Construct"})
        } else {
            serde_json::json!("Code")
        };
        assert!(serde_json::from_value::<FunctionDef>(conflict).is_err());
    }
}

#[test]
fn invalid_utf8_and_request_limits_are_recoverable() {
    let inputs = Inputs {
        files: BTreeMap::from([(
            "/project/views/own.txt".into(),
            InputRead::Ready(Arc::from([0xffu8])),
        )]),
        ..Default::default()
    };
    let output = evaluate("#self.own()\n\nkept", &inputs);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not valid UTF-8"))
    );
    let pipeline = Pipeline::default();
    let context = context();
    let analysis = pipeline
        .analyze("/project/pages/page.not", "#self.own()", &context)
        .unwrap();
    let EvaluationOutcome::Complete(output) = pipeline.evaluate_with_inputs_and_limits(
        &analysis,
        &context,
        &inputs,
        notist::Limits {
            read_requests: 0,
            ..Default::default()
        },
    ) else {
        panic!()
    };
    assert!(
        output
            .diagnostics
            .iter()
            .any(|d| d.message.contains("request limit"))
    );
}
