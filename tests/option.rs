use notist::{Context, FunctionId, Pipeline, Type, Value};

const MODULE: &str = r#"
fn graph(source: String, rule?: String, theme: String = "light") -> BlockContent;

fn diagram(source: String, rule?: String) -> BlockContent {
    [
        #notist.raw<BlockContent>(source)

        #graph(source, rule: rule)
    ]
}

fn label(rule?: String) -> String {
    match rule {
        some(r) => r,
        none => "all",
    }
}

fn show(rule?: String) -> InlineContent {
    [#text(text: label(rule))]
}

fn equal(a?: Int, b?: Int) -> String {
    if a == b { "eq" } else { "ne" }
}

fn show_equal(a?: Int, b?: Int) -> InlineContent {
    [#text(text: equal(a, b))]
}

fn via_none() -> String {
    label(rule: none)
}

fn show_via_none() -> InlineContent {
    [#text(text: via_none())]
}

fn sibling(flag: Bool) -> String {
    let value = if flag { some("x") } else { none };
    match value {
        some(v) => v,
        none => "empty",
    }
}

fn sibling_swapped(flag: Bool) -> String {
    let value = if flag { none } else { some("x") };
    match value {
        some(v) => v,
        none => "empty",
    }
}

fn show_sibling(flag: Bool) -> InlineContent {
    [#text(text: sibling(flag))]
}

fn show_sibling_swapped(flag: Bool) -> InlineContent {
    [#text(text: sibling_swapped(flag))]
}

fn tail() -> Option<String> { none }

fn use_tail() -> String {
    match tail() {
        some(v) => v,
        none => "was none",
    }
}

fn show_tail() -> InlineContent {
    [#text(text: use_tail())]
}

fn explicit(value: Option<String>) -> String {
    match value {
        some(v) => v,
        none => "explicit none",
    }
}

fn call_explicit() -> String {
    explicit(some("direct"))
}

fn show_explicit() -> InlineContent {
    [#text(text: call_explicit())]
}

fn folded() -> String {
    match some(1) {
        some(v) => "hit some",
        none => "hit none",
    }
}

fn show_folded() -> InlineContent {
    [#text(text: folded())]
}

fn bag(items: Array<Option<String>>) -> BlockContent;

fn pack(rule?: String) -> BlockContent {
    [ #bag((rule, rule)) ]
}
"#;

fn context() -> Context {
    Context::default()
        .with_module(
            Pipeline::default()
                .compile_module("content.not", "opt", "opt/lib.notc", MODULE)
                .unwrap(),
        )
        .unwrap()
}

fn evaluate_text(source: &str) -> String {
    let context = context();
    let pipeline = Pipeline::default();
    let analysis = pipeline.analyze("test.not", source, &context).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{source}: {:?}",
        analysis.diagnostics()
    );
    let evaluation = pipeline.evaluate(&analysis, &context);
    assert!(
        evaluation.diagnostics.is_empty(),
        "{source}: {:?}",
        evaluation.diagnostics
    );
    evaluation
        .root
        .descendants()
        .find_map(|item| match item.fields.get("text") {
            Some(Value::Str(text)) => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{source}: no text output"))
}

fn module_diagnostics(module: &str) -> Vec<String> {
    let context = Context::default()
        .with_module(
            Pipeline::default()
                .compile_module("content.not", "opt", "opt/lib.notc", module)
                .unwrap(),
        )
        .unwrap();
    Pipeline::default()
        .check_module("opt", &context)
        .iter()
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn optional_parameter_is_an_option_value_inside_code_bodies() {
    assert_eq!(evaluate_text("#self.show(\"x\")"), "x");
    assert_eq!(evaluate_text("#self.show()"), "all");
}

#[test]
fn option_equality_compares_variant_and_payload() {
    assert_eq!(evaluate_text("#self.show_equal()"), "eq");
    assert_eq!(evaluate_text("#self.show_equal(1)"), "ne");
    assert_eq!(evaluate_text("#self.show_equal(1, 1)"), "eq");
    assert_eq!(evaluate_text("#self.show_equal(1, 2)"), "ne");
    let diagnostics =
        module_diagnostics("fn bad(a?: Int) -> String { if a == 1 { \"x\" } else { \"y\" } }");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains("requires equal operand types")),
        "{diagnostics:?}"
    );
}

#[test]
fn none_is_typed_at_optional_boundaries_typed_siblings_and_option_tails() {
    assert_eq!(evaluate_text("#self.show_via_none()"), "all");
    assert_eq!(evaluate_text("#self.show_sibling(true)"), "x");
    assert_eq!(evaluate_text("#self.show_sibling(false)"), "empty");
    assert_eq!(evaluate_text("#self.show_sibling_swapped(true)"), "empty");
    assert_eq!(evaluate_text("#self.show_sibling_swapped(false)"), "x");
    assert_eq!(evaluate_text("#self.show_tail()"), "was none");
}

#[test]
fn bare_none_outside_typed_positions_is_rejected() {
    for (module, needle) in [
        (
            "fn bad() -> String { none }",
            "cannot determine the type of `none`",
        ),
        (
            "fn req(x: String) -> String { x }\nfn bad() -> String { req(none) }",
            "cannot determine the type of `none`",
        ),
        (
            "fn bad(x: Int) -> String { match x { 1 => none, _ => \"other\" } }",
            "cannot determine the type of `none`",
        ),
    ] {
        let diagnostics = module_diagnostics(module);
        assert!(
            diagnostics.iter().any(|d| d.contains(needle)),
            "{module}: {diagnostics:?}"
        );
    }
}

#[test]
fn option_subjects_match_constructor_patterns() {
    assert_eq!(evaluate_text("#self.show_folded()"), "hit some");
    assert_eq!(evaluate_text("#self.show_explicit()"), "direct");
    // A non-Option subject keeps plain == matching: some(1) is an ordinary
    // value expression, and the cross-type comparison is an ordinary error.
    let module = "fn plain() -> String { match 1 { some(1) => \"ctor\", _ => \"plain\" } }";
    let diagnostics = module_diagnostics(module);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains("requires equal operand types")),
        "{diagnostics:?}"
    );
    // Runtime Option matches require some+none arms or a final wildcard.
    let diagnostics =
        module_diagnostics("fn bad(rule?: String) -> String { match rule { some(r) => r, } }");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains("requires a final `_` arm")),
        "{diagnostics:?}"
    );
    assert!(
        module_diagnostics(
            "fn ok(rule?: String) -> String { match rule { some(r) => r, _ => \"other\", } }"
        )
        .is_empty()
    );
}

#[test]
fn forwarding_an_omitted_argument_omits_the_native_field() {
    let context = context();
    let pipeline = Pipeline::default();
    let graph_id = FunctionId::new("opt", "graph");
    for (source, expected) in [
        ("#self.diagram(\"src\")", None),
        ("#self.diagram(\"src\", rule: \"Number\")", Some("Number")),
    ] {
        let analysis = pipeline.analyze("test.not", source, &context).unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{source}: {:?}",
            analysis.diagnostics()
        );
        let evaluation = pipeline.evaluate(&analysis, &context);
        assert!(
            evaluation.diagnostics.is_empty(),
            "{source}: {:?}",
            evaluation.diagnostics
        );
        let graph = evaluation
            .root
            .find(|item| item.function_id() == Some(graph_id.clone()))
            .unwrap_or_else(|| panic!("{source}: graph boundary"));
        assert_eq!(
            graph.fields.get("source"),
            Some(&Value::Str("src".into())),
            "{source}"
        );
        assert_eq!(
            graph.fields.get("theme"),
            Some(&Value::Str("light".into())),
            "{source}"
        );
        assert_eq!(
            graph.fields.get("rule"),
            expected.map(|rule| Value::Str(rule.into())).as_ref(),
            "{source}"
        );
    }
}

#[test]
fn html_omits_none_attributes_and_encodes_nested_options() {
    let context = context();
    let mut html = notist_html::HtmlRegistry::default();
    for id in [
        FunctionId::new("opt", "graph"),
        FunctionId::new("opt", "bag"),
    ] {
        let definition = context.registry().get(&id).unwrap().clone();
        html.bind_component(&definition, format!("https://example.test/{}.js", id.name))
            .unwrap();
    }
    let pipeline = Pipeline::default();
    let render = |source: &str| {
        let analysis = pipeline.analyze("test.not", source, &context).unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{source}: {:?}",
            analysis.diagnostics()
        );
        let evaluation = pipeline.evaluate(&analysis, &context);
        assert!(
            evaluation.diagnostics.is_empty(),
            "{source}: {:?}",
            evaluation.diagnostics
        );
        let output = notist_html::Renderer::new()
            .with_registry(html.clone())
            .render_with_diagnostics(&evaluation.root);
        assert!(
            output.diagnostics.is_empty(),
            "{source}: {:?}",
            output.diagnostics
        );
        output.html
    };
    let omitted = render("#self.diagram(\"src\")");
    assert!(!omitted.contains("notist-rule"), "{omitted}");
    assert!(omitted.contains("notist-source=\"src\""), "{omitted}");
    let provided = render("#self.diagram(\"src\", rule: \"Number\")");
    assert!(provided.contains("notist-rule=\"Number\""), "{provided}");
    let nested = render("#self.pack(\"v\")");
    assert!(
        nested.contains("[&quot;some&quot;,[&quot;string&quot;,&quot;v&quot;]]"),
        "{nested}"
    );
    let nested_none = render("#self.pack()");
    assert!(nested_none.contains("[&quot;none&quot;]"), "{nested_none}");
    assert!(!nested_none.contains("&quot;some&quot;"), "{nested_none}");
}

#[test]
fn option_types_are_nominal_and_display_with_their_payload() {
    let optional = Type::Option(Box::new(Type::String));
    assert_eq!(optional.to_string(), "option of string");
    assert!(optional.accepts(&Value::Some(Box::new(Value::Str("x".into())))));
    assert!(optional.accepts(&Value::None));
    assert!(!optional.accepts(&Value::Str("x".into())));
    assert!(!Type::String.accepts(&Value::None));
    assert_ne!(optional, Type::String);
    assert_eq!(
        Type::Option(Box::new(Type::String)),
        Type::Option(Box::new(Type::String))
    );
    assert_ne!(
        Type::Option(Box::new(Type::String)),
        Type::Option(Box::new(Type::Int))
    );
    assert_eq!(
        Type::of(&Value::Some(Box::new(Value::Int(1)))),
        Type::Option(Box::new(Type::Int))
    );
}

#[cfg(feature = "serde")]
#[test]
fn option_values_round_trip_and_key_bit_exact() {
    use notist::ComptimeValue;
    for value in [
        Value::None,
        Value::Some(Box::new(Value::Int(1))),
        Value::Some(Box::new(Value::None)),
        Value::Some(Box::new(Value::Float(f64::from_bits(
            0x7ff8_0000_0000_0042,
        )))),
    ] {
        let json = serde_json::to_string(&value).unwrap();
        let restored: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, value);
    }
    let none = ComptimeValue::Data(Value::None).key();
    let some_none = ComptimeValue::Data(Value::Some(Box::new(Value::None))).key();
    assert_ne!(none, some_none);
    assert_eq!(none, ComptimeValue::Data(Value::None).key());
}
