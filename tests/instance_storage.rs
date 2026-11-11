use notist::{Context, DefId, FunctionId, Node, Pipeline, Value};
use notist_core::{definitions::ResolvedInstance, shared::Shared};

fn context(pipeline: &Pipeline, source: &str) -> Context {
    Context::default()
        .with_module(
            pipeline
                .compile_module("doc.not", "instance-storage", "instances.notc", source)
                .unwrap(),
        )
        .unwrap()
}

fn instances(root: &Node, function: DefId) -> Vec<&Shared<ResolvedInstance>> {
    root.descendants()
        .filter_map(|node| node.as_item()?.instance.as_ref())
        .filter(|instance| instance.function == function)
        .collect()
}

#[test]
fn repeated_constructor_targets_share_only_within_one_evaluation() {
    let pipeline = Pipeline::default();
    let context = context(
        &pipeline,
        "fn leaf(value: Int) -> InlineContent; fn typed<T: Type>(value: T) -> InlineContent; fn scalar<n: Int>() -> InlineContent;",
    );
    let leaf = FunctionId::new("instance-storage", "leaf");
    let typed = FunctionId::new("instance-storage", "typed");
    let scalar = FunctionId::new("instance-storage", "scalar");
    for dialect in ["doc.not", "doc.md"] {
        let analysis = pipeline
            .analyze(
                dialect,
                "#self.leaf(1)#self.leaf(2)#self.typed<Int>(3)#self.typed<Int>(4)#self.typed<Bool>(true)#self.scalar<1>()#self.scalar<1>()#self.scalar<2>()",
                &context,
            )
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let before = analysis.clone();
        let first = pipeline.evaluate(&analysis, &context);
        let second = pipeline.evaluate(&analysis, &context);
        assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
        assert_eq!(first, second);
        assert_eq!(analysis, before);
        let leaves = instances(&first.root, leaf.def_id());
        assert_eq!(leaves.len(), 2);
        assert!(leaves[0].shares_storage(leaves[1]));
        assert!(!leaves[0].shares_storage(instances(&second.root, leaf.def_id())[0]));
        let values: Vec<_> = first
            .root
            .descendants()
            .filter(|node| node.function_id() == Some(leaf.def_id()))
            .map(|node| node.field("value").unwrap().clone())
            .collect();
        assert_eq!(values, [Value::Int(1), Value::Int(2)]);
        let types = instances(&first.root, typed.def_id());
        assert_eq!(types.len(), 3);
        assert!(types[0].shares_storage(types[1]));
        assert!(!types[0].shares_storage(types[2]));
        assert!(!types[0].shares_storage(leaves[0]));
        let scalars = instances(&first.root, scalar.def_id());
        assert_eq!(scalars.len(), 3);
        assert!(scalars[0].shares_storage(scalars[1]));
        assert!(!scalars[0].shares_storage(scalars[2]));
    }
}

#[test]
fn mapped_targets_own_the_right_descriptor_and_survive_the_context() {
    use notist::extensions::{Replace, Replacement};
    let pipeline = Pipeline::default();
    let original = FunctionId::new("instance-storage", "original");
    let selected = FunctionId::new("instance-storage", "selected");
    let other = FunctionId::new("instance-storage", "other");
    let context = context(
        &pipeline,
        "fn original<T: Type>(value: T) -> InlineContent; fn selected<T: Type>(value: T) -> InlineContent; fn other<T: Type>(value: T) -> InlineContent;",
    ).with_extension(Replace::new([Replacement {
        from: original.clone(), to: selected.clone(), span: Default::default(),
    }])).unwrap();
    let analysis = pipeline.analyze("doc.not", "#self.original<Int>(1)#self.selected<Int>(2)#self.selected<Bool>(true)#self.other<Int>(3)", &context).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let mut result = pipeline.evaluate(&analysis, &context);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let targets = instances(&result.root, selected.def_id());
    assert_eq!(targets.len(), 3);
    assert!(targets[0].shares_storage(targets[1]));
    assert!(!targets[0].shares_storage(targets[2]));
    assert!(!targets[0].shares_storage(instances(&result.root, other.def_id())[0]));
    assert!(instances(&result.root, original.def_id()).is_empty());
    let original_root = result.root.clone();
    // A consumer editing one public descriptor still gets ordinary COW.
    fn edit_first(node: &mut Node, target: DefId) -> bool {
        if node.function_id() == Some(target) {
            node.as_item_mut()
                .unwrap()
                .instance
                .as_mut()
                .unwrap()
                .arguments
                .clear();
            return true;
        }
        node.children_mut()
            .into_iter()
            .flatten()
            .any(|child| edit_first(child, target))
    }
    assert!(edit_first(&mut result.root, selected.def_id()));
    let edited = instances(&result.root, selected.def_id());
    assert!(edited[0].arguments.is_empty());
    assert!(!edited[1].arguments.is_empty());
    assert!(
        !instances(&original_root, selected.def_id())[0]
            .arguments
            .is_empty()
    );
    drop(analysis);
    drop(context);
    result.sources.validate_node(&original_root).unwrap();
    assert_eq!(
        result
            .sources
            .definitions()
            .function(selected.def_id())
            .unwrap(),
        &selected
    );
    assert_eq!(
        result
            .sources
            .definitions()
            .function(other.def_id())
            .unwrap(),
        &other
    );
}

#[test]
fn specialization_limit_falls_back_without_changing_output_values() {
    let pipeline = Pipeline::default();
    let context = context(&pipeline, "fn marker<n: Int>() -> InlineContent;");
    let marker = FunctionId::new("instance-storage", "marker");
    let mut source = String::new();
    for n in 0..20 {
        source.push_str(&format!("#self.marker<{n}>()#self.marker<{n}>()"));
    }
    source.push_str("#self.marker<0>()");
    let analysis = pipeline.analyze("doc.not", &source, &context).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let result = pipeline.evaluate(&analysis, &context);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let descriptors = instances(&result.root, marker.def_id());
    assert_eq!(descriptors.len(), 41);
    for n in 0..20 {
        let first = descriptors[n * 2];
        let second = descriptors[n * 2 + 1];
        assert_eq!(
            first.arguments,
            vec![notist::ComptimeValue::Data(Value::Int(n as i64))]
        );
        assert_eq!(first, second);
        assert_eq!(first.shares_storage(second), n < 16);
    }
    assert!(descriptors[0].shares_storage(descriptors[40]));
}

#[test]
fn float_descriptors_preserve_signed_zero_and_nan_value_semantics() {
    let pipeline = Pipeline::default();
    let context = context(&pipeline, "fn marker<n: Float>() -> InlineContent;");
    let marker = FunctionId::new("instance-storage", "marker");
    let analysis = pipeline.analyze("doc.not", "#self.marker<0.0>()#self.marker<-0.0>()#self.marker<0.0>()#self.marker<-0.0>()#self.marker<(0.0 / 0.0)>()#self.marker<(0.0 / 0.0)>()", &context).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let result = pipeline.evaluate(&analysis, &context);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let descriptors = instances(&result.root, marker.def_id());
    assert_eq!(descriptors.len(), 6);
    assert!(descriptors[0].shares_storage(descriptors[2]));
    assert!(descriptors[1].shares_storage(descriptors[3]));
    assert!(!descriptors[0].shares_storage(descriptors[1]));
    assert_eq!(descriptors[0], descriptors[1]);
    assert!(descriptors[4].shares_storage(descriptors[5]));
    assert_ne!(descriptors[4], descriptors[5]);
    for (descriptor, expected) in descriptors.iter().take(4).zip([0.0_f64, -0.0, 0.0, -0.0]) {
        let notist::ComptimeValue::Data(Value::Float(actual)) = descriptor.arguments[0] else {
            panic!("float descriptor")
        };
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
    #[cfg(any(feature = "serde", feature = "command-extensions"))]
    {
        let wire = serde_json::to_value(result.sources.wire(&result.root)).unwrap();
        let restored: Node = result.sources.decode(wire.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(result.sources.wire(&restored)).unwrap(),
            wire
        );
        for (before, after) in descriptors
            .iter()
            .zip(instances(&restored, marker.def_id()))
        {
            assert_eq!(before.arguments[0].key(), after.arguments[0].key());
        }
    }
}

#[test]
fn sharing_descriptors_does_not_memoize_results_or_suppress_diagnostics() {
    let pipeline = Pipeline::default();
    let context = context(
        &pipeline,
        r#"
        fn show(value: Int) -> InlineContent {
            [#text(text: if value == 2 { panic("per-call failure") } else { str(value) })]
        }
    "#,
    );
    let analysis = pipeline
        .analyze(
            "doc.not",
            "#self.show(1)#self.show(2)#self.show(3)#self.show(2)",
            &context,
        )
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let result = pipeline.evaluate(&analysis, &context);
    assert_eq!(result.diagnostics.len(), 2);
    assert!(
        result
            .diagnostics
            .iter()
            .all(|d| d.recovered && d.message.contains("per-call failure"))
    );
    // Both failures point into the same function body; identical diagnostics
    // must still be emitted once per failed invocation, not deduplicated.
    assert_eq!(result.diagnostics[0], result.diagnostics[1]);
    let texts: Vec<_> = result
        .root
        .descendants()
        .filter_map(|node| node.field("text"))
        .cloned()
        .collect();
    assert_eq!(texts, [Value::Str("1".into()), Value::Str("3".into())]);
    let text = FunctionId::new("notist", "text");
    let descriptors = instances(&result.root, text.def_id());
    assert_eq!(descriptors.len(), 2);
    assert!(descriptors[0].shares_storage(descriptors[1]));
    assert_eq!(result, pipeline.evaluate(&analysis, &context));
}

#[cfg(any(feature = "serde", feature = "command-extensions"))]
#[test]
fn shared_and_detached_descriptors_have_identical_wire_data() {
    fn detach(node: &mut Node) {
        if let Some(item) = node.as_item_mut()
            && let Some(instance) = &item.instance
        {
            item.instance = Some(Shared::new((**instance).clone()));
        }
        for child in node.children_mut().into_iter().flatten() {
            detach(child);
        }
    }
    let pipeline = Pipeline::default();
    let context = context(&pipeline, "fn marker<n: Int>(value: Int) -> InlineContent;");
    let analysis = pipeline
        .analyze(
            "doc.not",
            "#self.marker<1>(1)#self.marker<1>(2)#self.marker<2>(3)",
            &context,
        )
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let result = pipeline.evaluate(&analysis, &context);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let mut detached = result.root.clone();
    detach(&mut detached);
    assert_eq!(detached, result.root);
    let marker = FunctionId::new("instance-storage", "marker");
    let detached_instances = instances(&detached, marker.def_id());
    assert!(!detached_instances[0].shares_storage(detached_instances[1]));
    let encoded = serde_json::to_value(result.sources.wire(&result.root)).unwrap();
    assert_eq!(
        encoded,
        serde_json::to_value(result.sources.wire(&detached)).unwrap()
    );
    let decoded: Node = result.sources.decode(encoded.clone()).unwrap();
    assert_eq!(decoded, result.root);
    assert_eq!(
        serde_json::to_value(result.sources.wire(&decoded)).unwrap(),
        encoded
    );
}
