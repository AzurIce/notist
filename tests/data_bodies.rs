use notist::{
    Context, Ctor, Evaluation, EvaluationInputs, EvaluationOutcome, InputRead, Item, Limits,
    Pipeline, ResourcePath, Value,
};
use std::{cell::Cell, sync::Arc};

fn context(dialect: &str, source: &str) -> Context {
    Context::default()
        .with_module(
            Pipeline::default()
                .compile_module(dialect, "demo", "views/lib.notc", source)
                .unwrap(),
        )
        .unwrap()
}
fn analyze(dialect: &str, module: &str, document: &str) -> (Pipeline, Context, notist::Analysis) {
    let pipeline = Pipeline::default();
    let context = context(dialect, module);
    let analysis = pipeline.analyze(dialect, document, &context).unwrap();
    (pipeline, context, analysis)
}
fn evaluate(dialect: &str, module: &str, document: &str) -> Evaluation {
    let (pipeline, context, analysis) = analyze(dialect, module, document);
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let before = analysis.clone();
    let result = pipeline.evaluate(&analysis, &context);
    assert_eq!(analysis, before);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert!(result.root.descendants().all(|n| n.as_expr().is_none()));
    result
}
fn html(root: &Item) -> String {
    let rendered = notist_html::Renderer::new().render_with_diagnostics(root);
    assert!(
        rendered.diagnostics.is_empty(),
        "{:?}",
        rendered.diagnostics
    );
    rendered.html
}

#[test]
fn scalar_display_is_explicit_lossless_and_escaped() {
    for dialect in ["doc.not", "doc.md"] {
        let module = r#"fn view() -> InlineContent {
            [#text(text: str(()))/#text(text: str(true))/#text(text: str(-9223372036854775807 - 1))/#text(text: str(9007199254740993))/#text(text: str(-0.0))/#text(text: str(1.0))/#text(text: str("雪<&"))]
        }"#;
        let result = evaluate(dialect, module, "#self.view()");
        assert_eq!(
            html(&result.root),
            "<p>()/true/-9223372036854775808/9007199254740993/-0/1/雪&lt;&amp;</p>"
        );
    }
}

#[test]
fn nonfinite_float_display_is_not_wire_encoding() {
    use notist_core::expr::Expr;
    for (number, expected) in [
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
    ] {
        let c = context(
            "doc.not",
            "fn fmt(x: Float) -> InlineContent { [#text(text: str(x))] }",
        );
        let (analysis, _) = Pipeline::default().analyze_lowered(
            std::path::Path::new("doc.not"),
            Arc::from(" "),
            notist_core::frontend::FrontendOutput {
                forest: vec![
                    Expr::call("self.fmt", Default::default())
                        .with_literal_arg("x", Value::Float(number)),
                ],
                ..Default::default()
            },
            &c,
            false,
        );
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let result = Pipeline::default().evaluate(&analysis, &c);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(html(&result.root), expected);
    }
}

#[test]
fn str_rejects_resources_views_containers_types_and_content() {
    for expression in [
        "p\"./secret\"",
        "(1,2)",
        "(key: 1)",
        "some(1)",
        "ok<Int, String>(1)",
        "[secret]",
        "Int",
        "data_expect<Data>(parse_json(\"{}\"))",
    ] {
        let module = format!("fn view() -> InlineContent {{ [#text(text: str({expression}))] }}");
        let (_, _, analysis) = analyze("doc.not", &module, "#self.view()");
        assert!(!analysis.diagnostics().is_empty(), "accepted {expression}");
    }
}

#[test]
fn array_methods_support_expression_receivers_and_do_not_become_free_functions() {
    for dialect in ["doc.not", "doc.md"] {
        let module = r#"
            fn make() -> Array<Int> { (4,5,6) }
            fn erased(x: Array) -> Int { x.len() }
            fn len(x: Int) -> Int { 99 }
            fn view() -> InlineContent {
                let xs = (1,2);
                [#text(text: str(xs.len()))/#text(text: str(make().len()))/#text(text: str((,).len()))/#text(text: str(erased(xs)))]
            }
        "#;
        assert_eq!(
            html(&evaluate(dialect, module, "#self.view()").root),
            "<p>2/3/0/2</p>"
        );
    }
    for expression in [
        "3.len()",
        "(1,).missing()",
        "(1,).len(2)",
        "(1,).len<Int>()",
        "arr_len((1,))",
        "array_len((1,))",
        "len((1,))",
        "notist.array.len((1,))",
    ] {
        let module = format!("fn view() -> InlineContent {{ [#text(text: str({expression}))] }}");
        assert!(
            !analyze("doc.not", &module, "#self.view()")
                .2
                .diagnostics()
                .is_empty(),
            "accepted {expression}"
        );
    }
}

#[test]
fn loops_join_content_in_order_in_both_frontends_and_body_forms() {
    for dialect in ["doc.not", "doc.md"] {
        for document in [
            "#for x in (1,2,3) [#text(text: str(x))]",
            "#for x in (1,2,3) { [#text(text: str(x))] }",
        ] {
            assert_eq!(html(&evaluate(dialect, "", document).root), "<p>123</p>");
        }
        let module = "fn view() -> InlineContent { for x in (1,2,3) [#text(text: str(x))] }";
        assert_eq!(
            html(&evaluate(dialect, module, "#self.view()").root),
            "<p>123</p>"
        );
    }
}

#[test]
fn capture_shadowing_repeated_mounts_and_nested_loops_are_lexical() {
    let module = r#"fn view() -> InlineContent {
        let x = 9;
        let value = for x in (1,2) { for y in (3,4) [#text(text: str(x))#text(text: str(y))] };
        [@(id: "first")#value#text(text: str(x))@(id: "second")#value]
    }"#;
    for dialect in ["doc.not", "doc.md"] {
        let result = evaluate(dialect, module, "#self.view()");
        assert_eq!(
            html(&result.root),
            "<p><span id=\"first\">13142324</span>9<span id=\"second\">13142324</span></p>"
        );
        assert_eq!(
            result
                .root
                .descendants()
                .filter(|n| n.ctor() == Ctor::Group)
                .count(),
            2
        );
    }
}

#[test]
fn structural_evidence_flows_through_lets_blocks_and_runtime_branches() {
    let module = r#"fn view(xs: Array<Int>, show: Bool) -> BlockContent {
        let rows = for x in xs {
            if show { row()[#for cell in (x, x + 1) { notist.cell()[#text(text: str(cell))] }] }
            else { [ ] }
        };
        table()[#rows]
    }"#;
    for dialect in ["doc.not", "doc.md"] {
        let result = evaluate(dialect, module, "#self.view((1,2), true)");
        assert_eq!(
            html(&result.root),
            "<table><tbody><tr><td>1</td><td>2</td></tr><tr><td>2</td><td>3</td></tr></tbody></table>"
        );
        assert_eq!(
            html(&evaluate(dialect, module, "#self.view((,), false)").root),
            "<table></table>"
        );
        let list =
            "fn view() -> BlockContent { list()[#for x in (1,2) { item()[#text(text: str(x))] }] }";
        assert_eq!(
            html(&evaluate(dialect, list, "#self.view()").root),
            "<ul><li>1</li><li>2</li></ul>"
        );
    }
}

#[test]
fn invalid_roles_are_rejected_even_for_empty_input_or_unselected_runtime_branch() {
    for body in [
        "paragraph()[wrong]",
        "if show { row()[#cell[ok]] } else { paragraph()[wrong] }",
        "helper()",
    ] {
        let module = format!(
            "fn helper() -> BlockContent {{ row()[#cell[ok]] }} fn view(xs: Array<Int>, show: Bool) -> BlockContent {{ let rows = for x in xs {{ {body} }}; table()[#rows] }}"
        );
        let (_, _, analysis) = analyze("doc.not", &module, "#self.view((,), true)");
        assert!(
            analysis
                .diagnostics()
                .iter()
                .any(|d| d.message.contains("takes only row children")),
            "{:?}",
            analysis.diagnostics()
        );
    }
}

#[test]
fn iteration_requires_a_static_element_type_and_a_content_result() {
    for module in [
        "fn view() -> InlineContent { for x in (,) [ok] }",
        "fn view(xs: Array) -> InlineContent { for x in xs [ok] }",
        "fn view() -> InlineContent { for x in (1,2) { x } }",
        "fn view() -> InlineContent { for x in data_expect<Data>(parse_json(\"[]\")) [ok] }",
        "fn view() -> InlineContent { for (i, x) in (1,2) [ok] }",
    ] {
        // Parse errors are reported during module compilation; checked errors during analysis.
        let p = Pipeline::default();
        if let Ok(code) = p.compile_module("doc.not", "demo", "lib.notc", module) {
            let c = Context::default().with_module(code).unwrap();
            let analysis = p.analyze("doc.not", "#self.view()", &c).unwrap();
            assert!(!analysis.diagnostics().is_empty(), "accepted {module}");
        }
    }
}

struct Inputs {
    reads: Cell<usize>,
    bytes: Option<Arc<[u8]>>,
}
impl EvaluationInputs for Inputs {
    fn file(&self, _: &ResourcePath) -> InputRead {
        self.reads.set(self.reads.get() + 1);
        self.bytes
            .clone()
            .map(InputRead::Ready)
            .unwrap_or(InputRead::Pending)
    }
}
fn run_inputs(module: &str, inputs: &Inputs, limits: Limits) -> EvaluationOutcome {
    let (p, c, a) = analyze("doc.not", module, "#self.view()");
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    p.evaluate_with_inputs_and_limits(&a, &c, inputs, limits)
}
const SUBJECT: &str = "data_expect<Array<Int>>(data_decode<Array<Int>>(data_expect<Data>(read_json(p\"./values.json\"))))";

#[test]
fn unused_traversals_do_not_read_and_each_mounted_subject_is_evaluated_once() {
    let inputs = Inputs {
        reads: Cell::new(0),
        bytes: Some(Arc::from(b"[1,2,3]".as_slice())),
    };
    let module = format!(
        "fn view() -> InlineContent {{ let hidden = for x in {SUBJECT} {{ panic(\"unused\") }}; [ok] }}"
    );
    // Never-only bodies need an explicit content result in their checking context.
    let module = module.replace(
        "panic(\"unused\")",
        "if x == 0 { panic(\"unused\") } else { [] }",
    );
    assert!(matches!(
        run_inputs(&module, &inputs, Limits::default()),
        EvaluationOutcome::Complete(_)
    ));
    assert_eq!(inputs.reads.get(), 0);
    let module = format!(
        "fn view() -> InlineContent {{ let loop = for x in {SUBJECT} [#text(text: str(x))]; [#loop#loop] }}"
    );
    let EvaluationOutcome::Complete(result) = run_inputs(&module, &inputs, Limits::default())
    else {
        panic!()
    };
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(html(&result.root), "<p>123123</p>");
    assert_eq!(inputs.reads.get(), 2);
    let pending = Inputs {
        reads: Cell::new(0),
        bytes: None,
    };
    assert!(matches!(
        run_inputs(&module, &pending, Limits::default()),
        EvaluationOutcome::NeedResources(_)
    ));
}

#[test]
fn method_receiver_is_evaluated_once_and_outer_reads_remain_strict() {
    let inputs = Inputs {
        reads: Cell::new(0),
        bytes: Some(Arc::from(b"[1,2,3]".as_slice())),
    };
    let module = format!("fn view() -> InlineContent {{ [#text(text: str({SUBJECT}.len()))] }}");
    let EvaluationOutcome::Complete(result) = run_inputs(&module, &inputs, Limits::default())
    else {
        panic!()
    };
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(html(&result.root), "<p>3</p>");
    assert_eq!(inputs.reads.get(), 1);
    let strict = format!(
        "fn view() -> InlineContent {{ let xs = {SUBJECT}; let hidden = for x in xs []; [ok] }}"
    );
    run_inputs(&strict, &inputs, Limits::default());
    assert_eq!(inputs.reads.get(), 2);
}

#[test]
fn empty_outputs_nested_calls_and_repeated_mounts_share_cumulative_budgets() {
    for module in [
        "fn part() -> InlineContent { for x in (1,2,3) [] } fn view() -> InlineContent { [#part()#part()] }",
        "fn view() -> InlineContent { let x = for v in (1,2,3) []; [#x#x] }",
        "fn view() -> InlineContent { for x in (1,2) { for y in (1,2,3) [] } }",
    ] {
        let (p, c, a) = analyze("doc.not", module, "#self.view()");
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        let result = p.evaluate_with_limits(
            &a,
            &c,
            Limits {
                iterations: 5,
                ..Limits::default()
            },
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.message.contains("iteration limit")),
            "{:?}",
            result.diagnostics
        );
    }
    let (p, c, a) = analyze(
        "doc.not",
        "fn view() -> InlineContent { for x in (1,2,3) [#text(text: str(x))] }",
        "#self.view()",
    );
    for limits in [
        Limits {
            loop_work: 2,
            ..Limits::default()
        },
        Limits {
            content_nodes: 6,
            ..Limits::default()
        },
    ] {
        let result = p.evaluate_with_limits(&a, &c, limits);
        assert!(!result.diagnostics.is_empty());
        assert!(result.root.descendants().all(|n| n.as_expr().is_none()));
    }
    let (p, c, a) = analyze(
        "doc.not",
        "fn recur() -> InlineContent { for x in (1,) [#recur()] }",
        "#self.recur()",
    );
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let result = p.evaluate(&a, &c);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("depth limit") || d.message.contains("nesting limit"))
    );
}

#[test]
fn failing_iteration_does_not_publish_a_partial_prefix() {
    let module = "fn view() -> InlineContent { for x in (1,2,3) { if x == 2 { panic(\"bad record\") } else { [#text(text: str(x))] } } }";
    let (p, c, a) = analyze("doc.not", module, "#self.view()");
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let result = p.evaluate(&a, &c);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("for iteration 1: bad record")),
        "{:?}",
        result.diagnostics
    );
    assert!(result.root.descendants().all(|n| n.ctor() != Ctor::Text));
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.source.as_deref() == Some("views/lib.notc"))
    );
}

#[test]
fn loop_iterations_preserve_local_section_shape_and_existing_call_boundaries() {
    let module = r#"fn view() -> BlockContent {
        for x in (1,2) {
            [
#heading<1>[#text(text: str(x))]

body
            ]
        }
    }"#;
    for dialect in ["doc.not", "doc.md"] {
        let result = evaluate(dialect, module, "#self.view()");
        assert_eq!(
            result
                .root
                .descendants()
                .filter(|n| n.ctor() == Ctor::Section)
                .count(),
            2
        );
        assert_eq!(
            result
                .root
                .descendants()
                .filter(|n| n.function_id()
                    == Some(notist_core::definitions::FunctionId::new("demo", "view")))
                .count(),
            0
        );
        assert_eq!(html(&result.root).matches("<section>").count(), 2);
    }
}

#[test]
fn pending_loop_subject_never_runs_output_hooks_or_returns_partial_tree() {
    use notist_core::extensions::{DocumentContext, DocumentExtension, ExtensionResult};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Debug)]
    struct Count(Arc<AtomicUsize>);
    impl DocumentExtension for Count {
        fn name(&self) -> &str {
            "loop-count-output"
        }
        fn after_evaluate(&self, _: &DocumentContext<'_>, _: &mut Item) -> ExtensionResult {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    let module =
        format!("fn view() -> InlineContent {{ for x in {SUBJECT} [#text(text: str(x))] }}");
    let (p, c, a) = analyze("doc.not", &module, "#self.view()");
    assert!(a.diagnostics().is_empty());
    let count = Arc::new(AtomicUsize::new(0));
    let c = c.with_extension(Count(count.clone())).unwrap();
    let a = p.analyze("doc.not", "#self.view()", &c).unwrap();
    let inputs = Inputs {
        reads: Cell::new(0),
        bytes: None,
    };
    assert!(matches!(
        p.evaluate_with_inputs(&a, &c, &inputs),
        EvaluationOutcome::NeedResources(_)
    ));
    assert_eq!(count.load(Ordering::Relaxed), 0);
}

#[test]
fn nominal_option_and_result_elements_keep_their_declared_payload_types() {
    let module = r#"
fn optional(xs: Array<Option<String>>) -> InlineContent {
    for x in xs { match x { some(s) => [#text(text: s)], none => [] } }
}
fn results(xs: Array<Result<String, Int>>) -> InlineContent {
    for x in xs { match x { ok(s) => [#text(text: s)], err(n) => [#text(text: str(n))] } }
}
fn view() -> InlineContent {
    [#optional((none, some("A")))#results((ok("B"), err(2)))]
}
"#;
    assert_eq!(
        html(&evaluate("doc.not", module, "#self.view()").root),
        "<p>AB2</p>"
    );
}

#[test]
fn annotations_source_ranges_and_markdown_container_offsets_cover_the_whole_loop() {
    for (dialect, document) in [
        (
            "doc.not",
            "雪 @(id: \"loop\")#for x in (1,2) [#text(text: str(x))] 尾",
        ),
        (
            "doc.md",
            "> 雪 @(id: \"loop\")#for x in (1,2) [#text(text: str(x))] 尾",
        ),
    ] {
        let result = evaluate(dialect, "", document);
        let group = result
            .root
            .find(|n| n.attrs.get("id") == Some(&Value::Str("loop".into())))
            .unwrap();
        assert!(group.is_group());
        let range = usize::from(group.span.start())..usize::from(group.span.end());
        assert_eq!(&document[range], "#for x in (1,2) [#text(text: str(x))]");
        let rendered = html(&result.root);
        assert!(
            rendered.contains("<span id=\"loop\">12</span>"),
            "{rendered}"
        );
        assert!(rendered.contains("尾"));
    }
}

#[test]
fn loop_prefix_boundaries_keep_code_strings_comments_and_adjacent_text_opaque() {
    for dialect in ["doc.not", "doc.md"] {
        for module in [
            r##"fn view() -> InlineContent { [#for x in ("]",) { [#text(text: x)] }尾] }"##,
            r##"fn view() -> InlineContent { [#for x in ("]",) /* ] { #for */ [#text(text: x)]尾] }"##,
        ] {
            let result = evaluate(dialect, module, "#self.view()");
            assert_eq!(html(&result.root), "<p>]尾</p>");
        }
    }
}

#[test]
fn loop_spelling_stays_opaque_in_native_raw_code_and_math() {
    for document in [
        "`#for` 入口",
        "`#for x in (1,) [danger]`",
        "```not\n#for x in (1,) [danger]\n```",
        "$#for x in (1,) [danger]$",
    ] {
        let result = evaluate("doc.not", "", document);
        assert!(html(&result.root).contains("#for"), "{document}");
        assert!(!result.root.descendants().any(Item::is_group));
    }
}

#[test]
fn nested_markup_loops_are_bounded_without_reparsing_exponential_content_trees() {
    for dialect in ["doc.not", "doc.md"] {
        let document = format!(
            "{}#text(text: str(x)){}",
            "#for x in (1,) [".repeat(10),
            "]".repeat(10)
        );
        assert_eq!(html(&evaluate(dialect, "", &document).root), "<p>1</p>");
        let document = format!("{}end{}", "#for x in (1,) [".repeat(40), "]".repeat(40));
        let (_, _, analysis) = analyze(dialect, "", &document);
        assert!(!analysis.diagnostics().is_empty());
    }
}

#[test]
fn for_remains_noncomptime_and_does_not_implicitly_convert_empty_inline_to_rows() {
    for module in [
        "fn view() -> InlineContent { comptime for x in (1,2) [ok] }",
        "fn view(xs: Array<Int>) -> BlockContent { table()[#for x in xs []] }",
        "fn view() -> InlineContent { for x in (1,2) [ wrong\n\nblock ] }",
    ] {
        let (p, c, a) = analyze("doc.not", module, "#self.view((,))");
        assert!(!a.diagnostics().is_empty() || !p.evaluate(&a, &c).diagnostics.is_empty());
    }
}

#[test]
fn prepared_loop_describe_analyze_and_render_agree_on_pending_and_real_data_errors() {
    use serde_json::{Value as Json, json};
    let source = "#import \"view.notc\" as view;\n#view.show()";
    let module = r#"fn show() -> BlockContent {
        let rows = for record in data_expect<Array<Data>>(data_decode<Array<Data>>(data_expect<Data>(read_json(p"./records.json")))) {
            row()[#cell[#text(text: str(data_expect<Int>(data_require<Int>(record, "id"))))]]
        };
        table()[#rows]
    }"#;
    let mut inputs = notist::PreparedInputs {
        root: "/project".into(),
        incomplete: true,
        files: [
            ("/project/page.not", source),
            ("/project/view.notc", module),
        ]
        .into_iter()
        .map(|(p, s)| (p.into(), s.as_bytes().to_vec()))
        .collect(),
        ..Default::default()
    };
    let call = |inputs: &notist::PreparedInputs, mode| -> Json {
        let inputs = serde_json::to_string(inputs).unwrap();
        serde_json::from_str(&match mode {
            0 => notist::preview::describe_prepared("/project/page.not", &inputs),
            1 => notist::preview::analyze_prepared("/project/page.not", source, &inputs),
            _ => notist::preview::render_prepared("/project/page.not", source, &inputs),
        })
        .unwrap()
    };
    for mode in 0..3 {
        assert_eq!(
            call(&inputs, mode),
            json!({"need_resources": [{"root":"/project", "path":"records.json"}]})
        );
    }
    inputs.files.insert(
        "/project/records.json".into(),
        br#"[{"id":1},{"id":2}]"#.to_vec(),
    );
    for mode in [1, 2] {
        let output = call(&inputs, mode);
        assert_eq!(output["diagnostics"], json!([]), "{output}");
        assert!(output["html"].as_str().unwrap().contains("<tr"));
        assert!(output["html"].as_str().unwrap().contains("<td"));
    }
    inputs.files.insert(
        "/project/records.json".into(),
        br#"[{"id":1},{"id":"broken"}]"#.to_vec(),
    );
    for mode in [1, 2] {
        let output = call(&inputs, mode);
        let diagnostics = output["diagnostics"].as_array().unwrap();
        assert!(
            diagnostics
                .iter()
                .any(|d| d["path"] == "/project/records.json"
                    && d["message"].as_str().unwrap().contains("/1/id")),
            "{output}"
        );
        assert!(
            !output["html"].as_str().unwrap().contains("<td>1</td>"),
            "partial row leaked"
        );
    }
    inputs
        .files
        .remove(&std::path::PathBuf::from("/project/records.json"));
    inputs.failures.insert(
        "/project/records.json".into(),
        notist::PreparedFailure::AccessDenied,
    );
    for mode in [1, 2] {
        let output = call(&inputs, mode);
        assert!(output.get("need_resources").is_none());
        assert!(
            output["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["message"].as_str().unwrap().contains("access denied"))
        );
    }
}

#[test]
fn cheap_scalar_projections_do_not_grow_a_batch_cache_but_arrays_still_share() {
    use notist_core::data::ParsedDataCache;
    let cache = notist::DataCache::new(16 * 1024 * 1024, 4);
    let limits = notist::DataLimits::default();
    let bytes: Arc<[u8]> = format!(
        "[{}]",
        (0..10_000)
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
    .into_bytes()
    .into();
    let root = cache
        .parse(notist::DataFormat::Json, bytes, limits)
        .unwrap();
    let before = cache.stats();
    for index in 0..10_000 {
        let data = root.require_index(index).unwrap();
        assert_eq!(
            cache.decode(&data, &notist::Type::Int, limits).unwrap(),
            Value::Int(index)
        );
        assert!(matches!(
            cache
                .decode(
                    &data,
                    &notist::Type::Option(Box::new(notist::Type::Int)),
                    limits
                )
                .unwrap(),
            Value::Some(_)
        ));
    }
    assert_eq!(cache.stats(), before);
    let array = notist::Type::Array(Some(Box::new(notist::Type::Int)));
    cache.decode(&root, &array, limits).unwrap();
    cache.decode(&root, &array, limits).unwrap();
    assert_eq!(cache.stats().projections, 1);
    assert_eq!(cache.stats().entries, 2);
}

#[cfg(any(feature = "serde", feature = "command-extensions"))]
#[test]
fn new_native_functions_and_deferred_loop_ir_round_trip_without_public_payloads() {
    use notist_core::{
        definitions::{FunctionImpl, NativeFunction},
        evaluation::{ExprKind, TypedExpr},
    };
    for (module, name, implementation) in [
        ("notist", "str", NativeFunction::Str),
        ("notist.array", "len", NativeFunction::ArrayLen),
    ] {
        let original = notist_core::builtins::registry()
            .function(module, name)
            .unwrap();
        let encoded = serde_json::to_string(original).unwrap();
        let decoded: notist_core::definitions::FunctionDef =
            serde_json::from_str(&encoded).unwrap();
        assert_eq!(&decoded, original);
        assert_eq!(decoded.implementation, FunctionImpl::Native(implementation));
    }
    let (p, c, a) = analyze(
        "doc.not",
        "fn view() -> InlineContent { for x in (1,2) [#text(text: str(x))] }",
        "#self.view()",
    );
    let result = p.evaluate(&a, &c);
    let encoded = serde_json::to_string(&result.root).unwrap();
    assert_eq!(serde_json::from_str::<Item>(&encoded).unwrap(), result.root);
    let subject = TypedExpr {
        ty: Some(notist::Type::Array(Some(Box::new(notist::Type::Int)))),
        known: None,
        kind: ExprKind::Array(Vec::new()),
        span: Default::default(),
        source: "test.notc".into(),
        shape: Default::default(),
    };
    let expr = TypedExpr {
        ty: Some(notist::Type::InlineContent),
        known: None,
        kind: ExprKind::For {
            binding: "x".into(),
            subject: subject.clone().into(),
            body: subject.into(),
        },
        span: Default::default(),
        source: "test.notc".into(),
        shape: Default::default(),
    };
    let encoded = serde_json::to_string(&Item::expression(expr.clone())).unwrap();
    assert_eq!(
        serde_json::from_str::<Item>(&encoded).unwrap(),
        Item::expression(expr)
    );
}
