use notist::{Context, Ctor, FunctionId, Pipeline, Value, builtins::Level};
fn context() -> Context {
    Context::default().with_module(Pipeline::default().compile_module("content.not", "widgets", "widgets.notc", "fn panel(title: String = \"Panel\")[children: BlockContent] -> BlockContent; fn badge(label: String)[children: InlineContent] -> InlineContent; fn leaf(value?: Int) -> BlockContent;", ).unwrap()).unwrap()
}
#[test]
fn extensions_preserve_fragment_boundaries_and_resolved_identity() {
    let project = context();
    let src = "before #self.badge(\"yes\")[inside]\n\n#self.panel()[\nbody #self.badge(\"nested\")\n]\n\nafter";
    let analysis = notist::Pipeline::default()
        .analyze("test.not", src, &project)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let panel = analysis
        .root()
        .find(|item| item.ctor.function_id() == Some(FunctionId::new("widgets", "panel")))
        .unwrap();
    assert_eq!(panel.level, Level::Block);
    assert_eq!(panel.fields.get("title"), Some(&Value::Str("Panel".into())));
    assert!(
        panel
            .children
            .iter()
            .any(|child| child.ctor == Ctor::Paragraph)
    );
    assert_eq!(
        notist::query::select(analysis.root(), "function:widgets.badge").len(),
        2
    );
    assert!(notist::query::select(analysis.root(), "level:block").contains(&panel));
    let json = notist::json::item(panel);
    assert_eq!(json["function"], "widgets.panel");
    assert_eq!(json["level"], "block");
    assert_eq!(json["fields"]["title"], "Panel");
    assert!(!json["children"].as_array().unwrap().is_empty());
    let (analysis, inspection) = Pipeline::default()
        .inspect("test.not", src, &project)
        .unwrap();
    let debug: serde_json::Value = serde_json::from_str(&notist::cst_json::inspection_json(
        &analysis,
        Some(&inspection),
    ))
    .unwrap();
    assert!(debug["diagnostics"].as_array().unwrap().is_empty());
    assert!(notist::dump::dump(panel).contains("[block]"));
}
#[test]
fn external_contract_errors_stay_distinct_from_unknown_calls() {
    let project = context();
    for (source, needle) in [
        ("#self.badge(1)", "string"),
        ("#self.badge()", "requires"),
        ("#self.leaf()[body]", "children"),
        ("#self.badge(\"x\")[\n#divider()\n]", "inline"),
        ("#self.missing()", "unknown"),
        ("#self.sub.f()", "unknown"),
    ] {
        let result = notist::Pipeline::default()
            .analyze("test.not", source, &project)
            .unwrap();
        assert!(
            result
                .diagnostics()
                .iter()
                .any(|d| d.message.to_lowercase().contains(needle)),
            "{source}: {:?}",
            result.diagnostics()
        );
    }
    let result = notist::Pipeline::default()
        .analyze("test.not", "#self.badge(1)", &project)
        .unwrap();
    assert!(
        result
            .root()
            .descendants()
            .any(|item| matches!(item.ctor, Ctor::Extension(_)))
    );
    assert!(
        !result
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("unknown"))
    );
    assert!(
        Pipeline::default()
            .analyze(
                "test.not",
                "#self.badge(\"x\")",
                &notist::Context::default()
            )
            .unwrap()
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("unknown"))
    );
}
#[cfg(feature = "serde")]
#[test]
fn serialized_extensions_need_no_module_to_restore_category_and_children() {
    let root = notist::Pipeline::default()
        .analyze("test.not", "#self.panel()[content]", &context())
        .unwrap()
        .into_parts()
        .0;
    let json = serde_json::to_value(&root).unwrap();
    let restored: notist::Item = serde_json::from_value(json).unwrap();
    let panel = restored
        .find(|node| matches!(node.ctor, Ctor::Extension(_)))
        .unwrap();
    assert_eq!(panel.level, Level::Block);
    assert_eq!(
        panel.ctor.function_id(),
        Some(FunctionId::new("widgets", "panel"))
    );
    assert_eq!(panel.fields.get("title"), Some(&Value::Str("Panel".into())));
    assert!(!panel.children.is_empty());
    let mut incomplete = serde_json::to_value(&root).unwrap();
    incomplete.as_object_mut().unwrap().remove("level");
    assert!(serde_json::from_value::<notist::Item>(incomplete).is_err());
}
#[cfg(feature = "serde")]
#[test]
fn float_serialization_and_ir_json_preserve_every_f64_bit_pattern() {
    for value in [
        0.0,
        -0.0,
        1.0,
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from_bits(0x7ff8_0000_0000_0042),
    ] {
        let original = Value::Float(value);
        let json = serde_json::to_string(&original).unwrap();
        let restored: Value = serde_json::from_str(&json).unwrap();
        let Value::Float(restored) = restored else {
            panic!("float type lost")
        };
        assert_eq!(restored.to_bits(), value.to_bits());
        let item = notist::Item::new(Ctor::Text, Default::default()).with_field("float", original);
        let json = notist::json::item(&item);
        assert_eq!(
            json["typed_fields"][1][0][1],
            serde_json::json!(["float", format!("{:016x}", value.to_bits())])
        );
    }
    // Finite numbers retain their scalar JSON representation.
    assert_eq!(
        serde_json::to_string(&Value::Float(1.5)).unwrap(),
        "{\"Float\":1.5}"
    );
}

#[cfg(feature = "serde")]
#[test]
fn serialized_items_retain_pending_calls_and_checked_expressions() {
    use notist::evaluation::{ExprKind, PendingEvaluation};
    let context = notist::Context::default()
        .with_module(
            notist_pipeline::Pipeline::default()
                .compile_module(
                    "content.not",
                    "demo",
                    "demo/lib.notc",
                    "fn greet(name: String) -> InlineContent { [#text(text: name)] }",
                )
                .unwrap(),
        )
        .unwrap();
    let analysis = notist::Pipeline::default()
        .analyze("doc.not", "#self.greet(\"world\")", &context)
        .unwrap();
    let restored: notist::Item =
        serde_json::from_value(serde_json::to_value(analysis.root()).unwrap()).unwrap();
    let original = analysis
        .root()
        .find(|i| i.function_id() == Some(FunctionId::new("demo", "greet")))
        .unwrap();
    let call = restored
        .find(|i| i.function_id() == original.function_id())
        .unwrap();
    assert_eq!(call.pending, original.pending);
    let Some(PendingEvaluation::Call(call)) = call.pending.as_deref() else {
        panic!("expected pending call")
    };
    assert!(matches!(&call.arguments[0].1.kind, ExprKind::Constant(_)));
    let json = notist::json::item(original);
    assert_eq!(json["pending"]["kind"], "call");
    assert_eq!(json["pending"]["arguments"][0]["name"], "name");
}
