use notist::{Context, Pipeline, Value};

fn evaluate(module: &str, source: &str) -> notist::Evaluation {
    let pipeline = Pipeline::default();
    let context = Context::default()
        .with_module(
            pipeline
                .compile_module("test.not", "execution", "execution/lib.notc", module)
                .unwrap(),
        )
        .unwrap();
    let analysis = pipeline.analyze("test.not", source, &context).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    pipeline.evaluate(&analysis, &context)
}

#[test]
fn checked_argument_order_is_not_reordered_to_source_or_parameter_positions() {
    let result = evaluate(
        r#"
        fn select(z: String, a: String) -> String { z }
        fn run(flag: Bool) -> InlineContent {
            [#text(text: select(z: if flag { panic("z-last") } else { "z" },
                               a: if flag { panic("a-first") } else { "a" }))]
        }
    "#,
        "#self.run(true)",
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("a-first"))
    );
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("z-last"))
    );
}

#[test]
fn captured_optional_content_replays_without_sharing_mutable_execution_state() {
    let result = evaluate(
        r#"
        fn label(value?: String) -> InlineContent {
            match value { some(v) => [#text(text: v)], none => [] }
        }
        fn run(value: String) -> InlineContent {
            let saved = label(value: some(value));
            [#saved#saved#label(value: none)]
        }
    "#,
        "#self.run(\"captured\")",
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let texts: Vec<_> = result
        .root
        .descendants()
        .filter_map(|node| match node.field("text") {
            Some(Value::Str(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["captured", "captured"]);
    assert!(
        result
            .root
            .descendants()
            .all(|node| node.as_expr().is_none())
    );
}
