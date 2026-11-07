use notist::{Analysis, Context, Ctor, Evaluation, Item, Limits, Pipeline, Value};
use notist_core::extensions::{DocumentContext, DocumentExtension, ExtensionResult};
use notist_html::Renderer;

fn context(dialect: &str, source: &str) -> Context {
    Context::default()
        .with_module(
            Pipeline::default()
                .compile_module(dialect, "demo", "widgets/lib.notc", source)
                .unwrap(),
        )
        .unwrap()
}

fn evaluate(dialect: &str, source: &str) -> (Analysis, Evaluation) {
    let pipeline = Pipeline::default();
    let context = context(dialect, source);
    let analysis = pipeline.analyze(dialect, "#self.view()", &context).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let before = analysis.clone();
    let evaluation = pipeline.evaluate(&analysis, &context);
    assert!(
        evaluation.diagnostics.is_empty(),
        "{:?}",
        evaluation.diagnostics
    );
    assert_eq!(analysis, before);
    assert!(
        evaluation
            .root
            .descendants()
            .all(|node| node.as_expr().is_none())
    );
    (analysis, evaluation)
}

fn groups(root: &Item) -> Vec<&Item> {
    root.descendants().filter(|node| node.is_group()).collect()
}

fn with_id<'a>(root: &'a Item, id: &str) -> &'a Item {
    root.find(|node| node.attrs.get("id") == Some(&Value::Str(id.into())))
        .unwrap()
}

#[test]
fn inline_annotations_belong_to_each_mount_not_its_first_child_or_shared_value() {
    let source = r#"
fn view() -> InlineContent {
    let value = [#strong[A]#emph[B]];
    [@(id: "left")#value/@(id: "right")#value]
}
"#;
    for dialect in ["doc.not", "doc.md"] {
        let (analysis, evaluation) = evaluate(dialect, source);
        assert!(groups(analysis.root()).is_empty());
        let left = with_id(&evaluation.root, "left");
        let right = with_id(&evaluation.root, "right");
        for group in [left, right] {
            assert!(group.is_group());
            assert!(group.function_id().is_none());
            assert_eq!(group.children().len(), 2);
            assert_eq!(group.children()[0].ctor(), Ctor::Strong);
            assert_eq!(group.children()[1].ctor(), Ctor::Emph);
            assert!(group.children().iter().all(|child| child.attrs.is_empty()));
            let origin = group.origin.as_ref().unwrap();
            assert_eq!(origin.source, "widgets/lib.notc");
            assert_eq!(origin.expansion[0].source, dialect);
        }
        assert_ne!(left.span, right.span);
        assert_eq!(left.children(), right.children());
        let before = evaluation.root.clone();
        let rendered = Renderer::new().render_with_diagnostics(&evaluation.root);
        assert!(
            rendered.diagnostics.is_empty(),
            "{:?}",
            rendered.diagnostics
        );
        assert_eq!(
            rendered.html,
            "<p><span id=\"left\"><strong>A</strong><em>B</em></span>/<span id=\"right\"><strong>A</strong><em>B</em></span></p>"
        );
        assert_eq!(evaluation.root, before);
        let mapped = Renderer::new()
            .with_source_map()
            .render_with_diagnostics(&evaluation.root);
        assert!(mapped.diagnostics.is_empty());
        for group in [left, right] {
            let range = usize::from(group.span.start())..usize::from(group.span.end());
            let mapping = mapped.source_map.iter().find(|m| m.range == range).unwrap();
            assert_eq!(mapping.origin, group.origin);
            assert_eq!(&source[range], "#value");
        }
    }
}

#[test]
fn block_annotations_cover_all_roots_without_leaking_into_the_next_fragment() {
    let source = r#"
fn view() -> BlockContent {
    let value = [ first

second ];
    [
@(id: "whole")
#value

after
    ]
}
"#;
    for dialect in ["doc.not", "doc.md"] {
        let (_, evaluation) = evaluate(dialect, source);
        let group = with_id(&evaluation.root, "whole");
        assert!(group.is_group());
        assert_eq!(group.level, notist::builtins::Level::Block);
        assert_eq!(group.children().len(), 2);
        assert!(
            group
                .children()
                .iter()
                .all(|child| { child.ctor() == Ctor::Paragraph && child.attrs.is_empty() })
        );
        let rendered = Renderer::new().render_with_diagnostics(&evaluation.root);
        assert!(
            rendered.diagnostics.is_empty(),
            "{:?}",
            rendered.diagnostics
        );
        assert_eq!(
            rendered.html,
            "<div id=\"whole\"><p>first</p><p>second</p></div><p>after</p>"
        );
    }
}

#[test]
fn annotated_empty_mounts_remain_but_attribute_free_mounts_dissolve() {
    let source = r#"
fn spin() -> InlineContent { [#spin()] }
fn view() -> InlineContent {
    let unused = [#spin()];
    let empty = [];
    [@(id: "empty")#empty#empty]
}
"#;
    for dialect in ["doc.not", "doc.md"] {
        let (_, evaluation) = evaluate(dialect, source);
        let mounted = groups(&evaluation.root);
        assert_eq!(mounted.len(), 1);
        assert!(mounted.iter().all(|group| group.children().is_empty()));
        assert_eq!(
            mounted[0].attrs.get("id"),
            Some(&Value::Str("empty".into()))
        );
        let rendered = Renderer::new().render_with_diagnostics(&evaluation.root);
        assert!(
            rendered.diagnostics.is_empty(),
            "{:?}",
            rendered.diagnostics
        );
        assert_eq!(rendered.html, "<p><span id=\"empty\"></span></p>");
        assert_eq!(groups(&evaluation.root).len(), 1);
    }
}

#[test]
fn unannotated_nested_value_mounts_dissolve_all_group_levels() {
    let source = r#"
fn view() -> InlineContent {
    let a = [A];
    let b = [#a];
    let c = [#b];
    [#c]
}
"#;
    for dialect in ["doc.not", "doc.md"] {
        let (_, evaluation) = evaluate(dialect, source);
        let mounted = groups(&evaluation.root);
        assert!(mounted.is_empty());
        assert_eq!(
            evaluation.root.children()[0].children()[0].ctor(),
            Ctor::Text
        );
        assert_eq!(Renderer::new().render(&evaluation.root), "<p>A</p>");
        assert!(groups(&evaluation.root).is_empty());
    }
}

#[test]
fn empty_block_mount_has_a_block_attribute_target_without_borrowing_the_next_block() {
    let source = r#"
fn view() -> BlockContent {
    let empty = [ ];
    [
@(id: "empty-block")
#empty

after
    ]
}
"#;
    for dialect in ["doc.not", "doc.md"] {
        let (_, evaluation) = evaluate(dialect, source);
        let group = with_id(&evaluation.root, "empty-block");
        assert!(group.is_group());
        assert!(group.children().is_empty());
        let rendered = Renderer::new().render_with_diagnostics(&evaluation.root);
        assert!(
            rendered.diagnostics.is_empty(),
            "{:?}",
            rendered.diagnostics
        );
        assert_eq!(rendered.html, "<div id=\"empty-block\"></div><p>after</p>");
    }
}

#[test]
fn an_empty_annotation_has_the_same_dissolution_semantics_as_no_annotation() {
    for dialect in ["doc.not", "doc.md"] {
        let (_, evaluation) = evaluate(
            dialect,
            "fn view() -> InlineContent { let empty = []; [@(:)#empty] }",
        );
        let mounted = groups(&evaluation.root);
        assert!(mounted.is_empty());
    }
}

#[test]
fn mount_attributes_survive_runtime_branch_selection() {
    let source = r#"
fn select(enabled: Bool) -> InlineContent {
    let value = if enabled { [A] } else { [B] };
    [@(id: "selected")#value]
}
fn view() -> InlineContent { select(false) }
"#;
    for dialect in ["doc.not", "doc.md"] {
        let (_, evaluation) = evaluate(dialect, source);
        let selected = with_id(&evaluation.root, "selected");
        assert!(selected.is_group());
        assert_eq!(
            selected.children()[0].field("text"),
            Some(&Value::Str("B".into()))
        );
        assert_eq!(
            Renderer::new().render(&evaluation.root),
            "<p><span id=\"selected\">B</span></p>"
        );
    }
}

#[test]
fn group_rendering_does_not_export_path_attributes() {
    let source = r#"
fn view() -> InlineContent {
    let value = [A];
    [@(secret: p"./private.txt")#value]
}
"#;
    let (_, evaluation) = evaluate("doc.not", source);
    let group = groups(&evaluation.root)[0];
    assert!(matches!(group.attrs.get("secret"), Some(Value::Path(_))));
    let output = Renderer::new().render_with_diagnostics(&evaluation.root);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|d| d.message.contains("Path"))
    );
    assert!(!output.html.contains("private"));
    assert!(!output.html.contains("secret"));
}

#[test]
fn internal_annotations_remain_internal_and_group_annotations_do_not_replace_them() {
    let source = r#"
fn view() -> InlineContent {
    let value = [@(id: "inner")#strong[A]#emph[B]];
    [@(id: "outer")#value]
}
"#;
    for dialect in ["doc.not", "doc.md"] {
        let (_, evaluation) = evaluate(dialect, source);
        assert!(with_id(&evaluation.root, "outer").is_group());
        assert_eq!(with_id(&evaluation.root, "inner").ctor(), Ctor::Strong);
        assert_eq!(
            Renderer::new().render(&evaluation.root),
            "<p><span id=\"outer\"><strong id=\"inner\">A</strong><em>B</em></span></p>"
        );
    }
}

#[test]
fn retained_empty_groups_consume_the_content_node_budget() {
    let source = format!(
        "fn view() -> InlineContent {{ let empty = []; [{}] }}",
        "@(status: \"kept\")#empty".repeat(20)
    );
    let pipeline = Pipeline::default();
    let context = context("doc.not", &source);
    let analysis = pipeline
        .analyze("doc.not", "#self.view()", &context)
        .unwrap();
    assert!(analysis.diagnostics().is_empty());
    let evaluation = pipeline.evaluate_with_limits(
        &analysis,
        &context,
        Limits {
            content_nodes: 6,
            ..Limits::default()
        },
    );
    assert!(
        evaluation
            .diagnostics
            .iter()
            .any(|d| d.message.contains("content size limit"))
    );
    assert!(
        evaluation
            .root
            .descendants()
            .all(|node| node.as_expr().is_none())
    );
    assert!(
        evaluation
            .root
            .descendants()
            .filter(|node| !node.is_error())
            .count()
            <= 6
    );
}

#[derive(Debug)]
struct TouchOutput;
impl DocumentExtension for TouchOutput {
    fn name(&self) -> &str {
        "touch-output"
    }

    fn after_evaluate(&self, _: &DocumentContext<'_>, root: &mut Item) -> ExtensionResult {
        root.attrs.insert("checked-output", Value::Bool(true));
        Ok(())
    }
}

#[test]
fn completed_groups_survive_output_extension_validation() {
    let pipeline = Pipeline::default();
    let context = context(
        "doc.not",
        "fn view() -> InlineContent { let value = [A]; [@(id: \"value\")#value] }",
    )
    .with_extension(TouchOutput)
    .unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.view()", &context)
        .unwrap();
    let evaluation = pipeline.evaluate(&analysis, &context);
    assert!(
        evaluation.diagnostics.is_empty(),
        "{:?}",
        evaluation.diagnostics
    );
    assert_eq!(
        evaluation.root.attrs.get("checked-output"),
        Some(&Value::Bool(true))
    );
    assert_eq!(groups(&evaluation.root).len(), 1);
}

struct PendingInputs;
impl notist::EvaluationInputs for PendingInputs {
    fn file(&self, _: &notist::ResourcePath) -> notist::InputRead {
        notist::InputRead::Pending
    }
}

#[test]
fn pending_reads_in_retained_groups_do_not_expose_partial_results_or_run_output_hooks() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[derive(Debug)]
    struct CountOutput(Arc<AtomicUsize>);
    impl DocumentExtension for CountOutput {
        fn name(&self) -> &str {
            "count-output"
        }
        fn after_evaluate(&self, _: &DocumentContext<'_>, _: &mut Item) -> ExtensionResult {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    let source = r#"
fn view() -> InlineContent {
    let value = [#text(text: read_text(p"./data.txt"))];
    [@(id: "loaded")#value]
}
"#;
    let pipeline = Pipeline::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let context = context("doc.not", source)
        .with_extension(CountOutput(calls.clone()))
        .unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.view()", &context)
        .unwrap();
    assert!(analysis.diagnostics().is_empty());
    let outcome = pipeline.evaluate_with_inputs(&analysis, &context, &PendingInputs);
    let notist::EvaluationOutcome::NeedResources(paths) = outcome else {
        panic!("partial evaluation must not become a completed document");
    };
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].path(), "widgets/data.txt");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn groups_are_explicit_content_not_a_combination_of_execution_flags() {
    let group = Item::new(Ctor::Interpolation, Default::default());
    assert!(!group.is_group());
    let group = Item::new(Ctor::Group, Default::default());
    assert!(group.is_group());
    assert!(group.as_expr().is_none());
    assert!(group.instance().is_none());
}

#[test]
fn rendering_structural_groups_checks_their_children_without_mutating_the_tree() {
    let table = Pipeline::default()
        .analyze("doc.not", "#table[#row[#cell[A]]]", &Context::default())
        .unwrap()
        .root()
        .children()[0]
        .clone();
    let mut table = table;
    let group = Item::new(Ctor::Group, table.span)
        .with_children(std::mem::take(table.children_mut().unwrap()));
    *table.children_mut().unwrap() = vec![group];
    let before = table.clone();
    let rendered = Renderer::new().render_with_diagnostics(&table);
    assert!(
        rendered.diagnostics.is_empty(),
        "{:?}",
        rendered.diagnostics
    );
    assert_eq!(
        rendered.html,
        "<table><tbody><tr><td>A</td></tr></tbody></table>"
    );
    assert_eq!(table, before);
    table.children_mut().unwrap()[0]
        .attrs
        .insert("id", Value::Str("rows".into()));
    let rendered = Renderer::new().render_with_diagnostics(&table);
    assert_eq!(rendered.diagnostics.len(), 1);
    assert!(
        rendered.diagnostics[0]
            .message
            .contains("content group attributes omitted")
    );
    assert!(!rendered.html.contains("id=\"rows\""));
}

#[test]
fn a_group_does_not_grant_structural_roles_to_an_opaque_block_value() {
    for dialect in ["doc.not", "doc.md"] {
        let source = "fn view() -> BlockContent { let value = [ wrong ]; table()[#value] }";
        let context = context(dialect, source);
        let analysis = Pipeline::default()
            .analyze(dialect, "#self.view()", &context)
            .unwrap();
        assert!(
            analysis
                .diagnostics()
                .iter()
                .any(|d| d.message.contains("takes only row children"))
        );
    }
}

#[cfg(any(feature = "serde", feature = "command-extensions"))]
#[test]
fn retained_group_state_attributes_and_empty_children_round_trip() {
    let (_, evaluation) = evaluate(
        "doc.not",
        "fn view() -> InlineContent { let value = []; [@(id: \"empty\")#value] }",
    );
    let encoded = serde_json::to_string(&evaluation.root).unwrap();
    let decoded: Item = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, evaluation.root);
    assert!(with_id(&decoded, "empty").is_group());
    assert!(with_id(&decoded, "empty").children().is_empty());
}
