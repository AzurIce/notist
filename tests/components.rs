use notist::{Dict, FunctionId, Value, project::Project};
use notist_html::{HtmlRegistry, Renderer};
fn definitions() -> notist::DefinitionModule {
    notist::analyze_module("widgets", "fn panel(title: String = \"\", open: Bool = false, count: Int = 9223372036854775807, ratio: Float = 1.5, optional?: String, data: Array = (false,))[children: Content] -> Content; fn badge(label: String) -> InlineContent;").unwrap()
}
#[test]
fn component_fields_escape_keep_scalar_semantics_and_record_nested_dependencies() {
    let mut registry = notist::builtins::registry().clone();
    let module = definitions();
    let mut html = HtmlRegistry::default();
    for definition in &module.functions {
        html.bind_component(
            definition,
            format!("https://example.test/{}.js", definition.id.name),
        )
        .unwrap();
    }
    registry.register(module).unwrap();
    let analysis = notist::Notist::default()
        .with_registry(registry)
        .analyze(
            "test.not",
            "#widgets::panel(title: \"<&\\\"\")[#widgets::badge(\"one\") #widgets::badge(\"two\")]",
        )
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let output = Renderer::new()
        .with_registry(html)
        .with_source_map()
        .render_with_diagnostics(analysis.root());
    assert!(output.diagnostics.is_empty());
    assert!(
        output.html.contains("notist-title=\"&lt;&amp;&quot;\""),
        "{}",
        output.html
    );
    assert!(output.html.contains("notist-open=\"false\""));
    assert!(output.html.contains("notist-count=\"9223372036854775807\""));
    assert!(output.html.contains("notist-ratio=\"1.5\""));
    assert!(!output.html.contains("notist-optional"));
    assert_eq!(output.used_components.len(), 2);
    assert_eq!(
        output.used_components[0].id,
        FunctionId::new("widgets", "panel")
    );
    assert_eq!(
        output.used_components[1].id,
        FunctionId::new("widgets", "badge")
    );
    assert!(
        output.html.find("notist-label=\"one\"").unwrap()
            < output.html.find("notist-label=\"two\"").unwrap()
    );
    assert!(
        output
            .source_map
            .iter()
            .any(|mapping| mapping.kind == notist_html::SourceMappingKind::Container)
    );
}
#[test]
fn complex_values_preserve_type_order_and_exact_numeric_bits() {
    let mut dict = Dict::default();
    dict.insert("second", Value::Int(i64::MAX));
    dict.insert("first", Value::Float(-0.0));
    dict.insert("unit", Value::Unit);
    let encoded = notist_html::components::encode_parameter(&Value::Array(vec![
        Value::Dict(dict),
        Value::Bool(false),
        Value::Str("".into()),
    ]));
    let json: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(json[0], "array");
    assert_eq!(json[1][0][0], "dict");
    assert_eq!(
        json[1][0][1][0],
        serde_json::json!(["second", ["int", "9223372036854775807"]])
    );
    assert_eq!(
        json[1][0][1][1],
        serde_json::json!(["first", ["float", "8000000000000000"]])
    );
    assert_eq!(json[1][1], serde_json::json!(["bool", false]));
    assert_eq!(json[1][2], serde_json::json!(["string", ""]));
}
#[test]
fn html_registry_rejects_tag_attribute_and_reserved_collisions() {
    let mut registry = HtmlRegistry::default();
    let first = notist::analyze_module("a-b", "fn c() -> Content;").unwrap();
    let second = notist::analyze_module("a", "fn b-c() -> Content;").unwrap();
    registry
        .bind_component(&first.functions[0], "first.js")
        .unwrap();
    assert!(
        registry
            .bind_component(&second.functions[0], "second.js")
            .unwrap_err()
            .contains("collision")
    );
    let uppercase =
        notist::analyze_module("case", "fn A() -> Content; fn u41x() -> Content;").unwrap();
    registry
        .bind_component(&uppercase.functions[0], "a.js")
        .unwrap();
    assert!(
        registry
            .bind_component(&uppercase.functions[1], "b.js")
            .is_err()
    );
    for source in [
        "fn f(protocol: String) -> Content;",
        "fn f(A: String, u41x: String) -> Content;",
    ] {
        let definition = notist::analyze_module("names", source).unwrap();
        assert!(
            registry
                .bind_component(&definition.functions[0], "f.js")
                .is_err()
        );
    }
    assert_eq!(
        notist_html::components::component_tag(&FunctionId::new("包", "图")),
        "u5305x-u56fex"
    );
}
#[test]
fn host_copies_used_directory_resources_and_checks_convention_conflicts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins");
    let project = Project::load(root.join("Notist.toml")).unwrap();
    let source = std::fs::read_to_string(root.join("document.not")).unwrap();
    let analysis = project.analyzer().analyze("document.not", &source).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let built = notist::html_host::build_page(analysis.root(), &project, temp.path()).unwrap();
    assert_eq!(built.used_components.len(), 3);
    assert!(
        temp.path()
            .join("packages/widgets/components/panel/style.js")
            .is_file()
    );
    assert!(
        temp.path()
            .join("packages/widgets/components/badge.js")
            .is_file()
    );
    let entry = std::fs::read_to_string(temp.path().join("components.js")).unwrap();
    assert_eq!(entry.matches("customElements.define").count(), 3);
    assert_eq!(entry.matches("widgets-panel").count(), 1);
    let missing = project
        .analyzer()
        .analyze("test.not", "#widgets::badge(\"test\")")
        .unwrap();
    let rendered = Renderer::new().render_with_diagnostics(missing.root());
    assert!(
        rendered
            .diagnostics
            .iter()
            .any(|error| error.message.contains("no HTML implementation"))
    );
    let fake = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(fake.path().join("components/f")).unwrap();
    std::fs::write(fake.path().join("lib.notc"), "fn f() -> Content;").unwrap();
    std::fs::write(
        fake.path().join("components/f.js"),
        "export default class {}",
    )
    .unwrap();
    std::fs::write(
        fake.path().join("components/f/index.js"),
        "export default class {}",
    )
    .unwrap();
    let project = Project::from_packages([notist::project::Package {
        name: "fake".into(),
        root: fake.path().into(),
        source: std::fs::read_to_string(fake.path().join("lib.notc")).unwrap(),
    }])
    .unwrap();
    assert!(
        notist::html_host::html_registry(&project).unwrap_err()[0]
            .diagnostic
            .message
            .contains("conflicting")
    );
}
#[test]
fn browser_host_uses_explicit_sources_and_urls_and_supports_module_inspection() {
    let config = "[dependencies]\nwidgets = {path = './widgets'}\n";
    let packages = serde_json::json!({"widgets":{"source":"fn panel()[children: Content] -> Content;", "components":{"panel":"https://example.test/panel.js"}}}).to_string();
    let preview: serde_json::Value = serde_json::from_str(&notist::preview::analyze_preview(
        "test.not",
        "#widgets::panel()[content]",
        config,
        &packages,
    ))
    .unwrap();
    assert!(preview["diagnostics"].as_array().unwrap().is_empty());
    assert!(preview["html"].as_str().unwrap().contains("<widgets-panel"));
    assert_eq!(
        preview["used_components"][0]["module"],
        "https://example.test/panel.js"
    );
    let module: serde_json::Value = serde_json::from_str(&notist::preview::analyze_preview(
        "lib.notc",
        "fn f(value: Int = false) -> Content; fn g() -> Content;",
        "",
        "{}",
    ))
    .unwrap();
    assert_eq!(module["tree"]["kind"], "Module");
    assert_eq!(module["ast"]["kind"], "Module");
    assert_eq!(module["ast"]["children"].as_array().unwrap().len(), 2);
    assert!(!module["diagnostics"].as_array().unwrap().is_empty());
    let manifest: serde_json::Value =
        serde_json::from_str(&notist::preview::configuration_json(config)).unwrap();
    assert_eq!(manifest["dependencies"][0]["name"], "widgets");
}

#[test]
fn native_nonfinite_defaults_use_the_same_component_binding_path() {
    let mut module = notist::DefinitionModule::new("native");
    let mut definition = notist::FunctionDef::new(
        notist::FunctionId::new("native", "meter"),
        notist::builtins::Accepts::Nothing,
        notist::ReturnRule::Fixed(notist::builtins::Level::Inline),
    );
    definition.parameters.push(notist::ParameterDef::new(
        "reading",
        notist::ValueType::Float,
        notist::ParameterMode::Default(Value::Float(f64::NAN)),
    ));
    module.functions.push(definition.clone());
    let mut semantics = notist::builtins::registry().clone();
    semantics.register(module).unwrap();
    let mut html = HtmlRegistry::default();
    html.bind_component(&definition, "meter.js").unwrap();
    let analysis = notist::Notist::default()
        .with_registry(semantics)
        .analyze("test.not", "#native::meter()")
        .unwrap();
    let result = Renderer::new()
        .with_registry(html)
        .render_with_diagnostics(analysis.root());
    assert!(result.diagnostics.is_empty());
    assert!(result.html.contains("notist-reading=\"NaN\""));
}

#[test]
fn inline_component_accepting_block_children_keeps_a_parseable_html_tree() {
    let module =
        notist::analyze_module("widgets", "fn popup()[children: Content] -> InlineContent;")
            .unwrap();
    let mut semantics = notist::builtins::registry().clone();
    let mut html = HtmlRegistry::default();
    html.bind_component(&module.functions[0], "popup.js")
        .unwrap();
    semantics.register(module).unwrap();
    let result = notist::Notist::default()
        .with_registry(semantics)
        .analyze("test.not", "before #widgets::popup()[inside] after")
        .unwrap();
    assert!(result.diagnostics().is_empty());
    let rendered = Renderer::new()
        .with_registry(html)
        .render_with_diagnostics(result.root());
    assert!(rendered.diagnostics.is_empty());
    assert_eq!(
        rendered.html,
        "<div class=\"notist-paragraph\">before <widgets-popup notist-protocol=\"1\"><p>inside</p></widgets-popup> after</div>"
    );
}
