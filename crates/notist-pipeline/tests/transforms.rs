use notist_core::{
    builtins,
    builtins::{Accepts, Level},
    definitions::{
        DefinitionModule, FunctionDef, FunctionId, ParameterDef, ParameterMode, ReturnRule,
        ValueType,
    },
    item::{Ctor, Item, Value},
    registry::Registry,
};
use notist_pipeline::{
    Pipeline, analyze_module,
    transforms::{Replace, TransformPlan},
};
use rowan::TextRange;

fn span() -> TextRange {
    TextRange::new(2.into(), 8.into())
}
fn rule(from: &str, to: &str) -> Replace {
    let id = |name: &str| {
        let (package, function) = name.split_once("::").unwrap();
        FunctionId::new(package, function)
    };
    Replace {
        from: id(from),
        to: id(to),
        span: span(),
    }
}
fn registry() -> Registry {
    let mut registry = builtins::registry().clone();
    registry.register(analyze_module("maths", "fn first(text: String) -> InlineContent; fn second(text: String) -> InlineContent; fn inline()[children: InlineContent] -> InlineContent; fn block(text: String) -> Content; fn wrong(source: String) -> InlineContent; fn optional(text?: String) -> InlineContent;").unwrap()).unwrap();
    registry
}

#[test]
fn both_frontends_keep_analysis_identity_and_transform_nested_math() {
    let registry = registry();
    let plan = TransformPlan::compile(&[rule("notist::math", "maths::first")], &registry).unwrap();
    for (path, source) in [
        ("demo.not", "#callout[$x$ #math(\"y\")]"),
        ("demo.md", "- $x$\n\n| Heading |\n| --- |\n| $y$ |"),
    ] {
        let analysis = Pipeline::default()
            .analyze(path, source, &registry)
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let before: Vec<_> = analysis
            .root()
            .descendants()
            .filter(|node| node.ctor == Ctor::Math)
            .cloned()
            .collect();
        assert_eq!(before.len(), 2);
        let output = plan.apply(analysis.root());
        assert!(output.diagnostics.is_empty());
        let after: Vec<_> = output
            .root
            .descendants()
            .filter(|node| node.function_id() == Some(FunctionId::new("maths", "first")))
            .collect();
        assert_eq!(after.len(), before.len());
        for (before, after) in before.iter().zip(after) {
            assert_eq!(before.fields, after.fields);
            assert_eq!(before.span, after.span);
            assert_eq!(before.level, after.level);
            assert_eq!(before.attrs, after.attrs);
            assert_eq!(before.children, after.children);
        }
        assert_eq!(
            analysis
                .root()
                .descendants()
                .filter(|node| node.ctor == Ctor::Math)
                .count(),
            2
        );
    }
}

#[test]
fn ordered_rules_run_once_each_and_can_target_builtins() {
    let registry = registry();
    let analysis = Pipeline::default()
        .analyze("demo.not", "$x$", &registry)
        .unwrap();
    let rules = [
        rule("notist::math", "maths::first"),
        rule("maths::first", "maths::second"),
    ];
    let output = TransformPlan::compile(&rules, &registry)
        .unwrap()
        .apply(analysis.root());
    assert!(
        output
            .root
            .find(|node| node.function_id() == Some(FunctionId::new("maths", "second")))
            .is_some()
    );
    let reversed = TransformPlan::compile(&[rules[1].clone(), rules[0].clone()], &registry)
        .unwrap()
        .apply(analysis.root());
    assert!(
        reversed
            .root
            .find(|node| node.function_id() == Some(FunctionId::new("maths", "first")))
            .is_some()
    );
    let roundtrip = TransformPlan::compile(
        &[rules[0].clone(), rule("maths::first", "notist::math")],
        &registry,
    )
    .unwrap()
    .apply(analysis.root());
    assert_eq!(roundtrip.root, *analysis.root());
    let same = TransformPlan::compile(&[rule("notist::math", "notist::math")], &registry)
        .unwrap()
        .apply(analysis.root());
    assert_eq!(same.root, *analysis.root());
}

#[test]
fn rejects_unknown_functions_and_incompatible_or_structural_contracts_atomically() {
    let registry = registry();
    for (from, to, reason) in [
        ("notist::math", "maths::missing", "unknown"),
        ("maths::missing", "notist::math", "unknown"),
        ("notist::math", "maths::block", "levels"),
        ("notist::math", "maths::wrong", "parameter"),
        ("notist::math", "maths::optional", "parameter"),
        ("notist::strong", "notist::math", "children"),
        ("notist::heading", "notist::heading", "structural"),
        ("notist::item", "notist::callout", "structural"),
        ("notist::group", "maths::inline", "fixed"),
    ] {
        let errors = TransformPlan::compile(
            &[rule("notist::math", "maths::first"), rule(from, to)],
            &registry,
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(reason)),
            "{errors:?}"
        );
        assert!(errors.iter().all(|error| error.span == span()));
    }
}

#[test]
fn recovery_nodes_are_preserved_while_valid_descendants_still_transform() {
    let registry = registry();
    let plan = TransformPlan::compile(&[rule("notist::math", "maths::first")], &registry).unwrap();
    let good = Item::new(Ctor::Math, span()).with_field("text", Value::Str("x".into()));
    let mut bad = Item::new(Ctor::Math, TextRange::new(0.into(), 10.into()))
        .with_field("text", Value::Bool(false))
        .with_children(vec![good]);
    bad.attrs.insert("id", Value::Str("original".into()));
    let original = bad.clone();
    let output = plan.apply(&bad);
    assert_eq!(bad, original);
    assert_eq!(output.root.ctor, Ctor::Math);
    assert_eq!(output.root.fields, bad.fields);
    assert_eq!(output.root.attrs, bad.attrs);
    assert_eq!(output.diagnostics.len(), 1);
    assert_eq!(output.diagnostics[0].span, bad.span);
    assert_eq!(
        output.root.children[0].function_id(),
        Some(FunctionId::new("maths", "first"))
    );
    for bad in [
        Item::new(Ctor::Math, span()),
        Item::new(Ctor::Math, span())
            .with_field("text", Value::Str("x".into()))
            .with_field("extra", Value::Int(3)),
        Item::new(Ctor::Math, span())
            .with_field("text", Value::Str("x".into()))
            .with_children(vec![Item::text("child".into(), span())]),
    ] {
        let output = plan.apply(&bad);
        assert_eq!(output.root, bad);
        assert_eq!(output.diagnostics.len(), 1);
    }
}

#[test]
fn metadata_children_and_exact_values_survive_extension_replacement() {
    let mut registry = registry();
    registry.register(analyze_module("values", "fn first(count: Int, ratio: Float, data: Dict)[children: Content] -> Content; fn second(data: Dict, ratio: Float, count: Int)[children: Content] -> Content;").unwrap()).unwrap();
    let source =
        "#values::first(9223372036854775807, -0.0, (\"last\": (false, 0.1), \"first\": 7))[child]";
    let analysis = Pipeline::default()
        .analyze("values.not", source, &registry)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let mut root = analysis.root().clone();
    root.children[0]
        .attrs
        .insert("id", Value::Str("example".into()));
    let output = TransformPlan::compile(&[rule("values::first", "values::second")], &registry)
        .unwrap()
        .apply(&root);
    assert!(output.diagnostics.is_empty());
    let before = &root.children[0];
    let after = &output.root.children[0];
    assert_eq!(before.fields, after.fields);
    assert_eq!(before.children, after.children);
    assert_eq!(before.attrs, after.attrs);
    assert_eq!(before.span, after.span);
    assert!(
        matches!(after.fields.get("ratio"), Some(Value::Float(value)) if value.to_bits() == (-0.0_f64).to_bits())
    );
}

#[test]
fn native_default_contracts_compare_float_bits_and_reject_dynamic_levels() {
    let mut registry = registry();
    let mut module = DefinitionModule::new("native");
    for (name, value) in [
        ("a", f64::from_bits(0x7ff8000000000001)),
        ("b", f64::from_bits(0x7ff8000000000001)),
        ("c", f64::from_bits(0x7ff8000000000002)),
    ] {
        let mut definition = FunctionDef::new(
            FunctionId::new("native", name),
            Accepts::Nothing,
            ReturnRule::Fixed(Level::Inline),
        );
        definition.parameters.push(ParameterDef::new(
            "value",
            ValueType::Float,
            ParameterMode::Default(Value::Float(value)),
        ));
        module.functions.push(definition);
    }
    registry.register(module).unwrap();
    let plan = TransformPlan::compile(&[rule("native::a", "native::b")], &registry).unwrap();
    let ctor = Ctor::Extension(notist_core::item::ExtensionCtor {
        id: FunctionId::new("native", "a"),
        accepts: Accepts::Nothing,
        level: Level::Inline,
    });
    let unnormalized = Item::new(ctor.clone(), span());
    assert_eq!(plan.apply(&unnormalized).diagnostics.len(), 1);
    let input = unnormalized.with_field("value", Value::Float(f64::from_bits(0x7ff8000000000001)));
    let output = plan.apply(&input);
    assert!(output.diagnostics.is_empty());
    assert!(
        matches!(output.root.fields.get("value"), Some(Value::Float(v)) if v.to_bits() == 0x7ff8000000000001)
    );
    assert!(TransformPlan::compile(&[rule("native::a", "native::c")], &registry).is_err());
    assert!(
        TransformPlan::compile(&[rule("notist::raw", "notist::raw")], &registry).unwrap_err()[0]
            .message
            .contains("fixed")
    );
}
