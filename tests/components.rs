use notist::{Dict, Environment, FunctionId, Value};
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
    let analysis = notist::Pipeline::default()
        .analyze(
            "test.not",
            "#widgets::panel(title: \"<&\\\"\")[#widgets::badge(\"one\") #widgets::badge(\"two\")]",
            &registry,
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
fn missing_html_targets_and_filesystem_entry_conflicts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let project = Environment::load_from(&notist::FsResources::new(&root), "Notist.toml").unwrap();
    let missing = notist::Pipeline::default()
        .analyze(
            "test.not",
            "#notist-doc::badge(\"test\")",
            project.registry(),
        )
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
    let project = Environment::from_packages([notist::Package {
        name: "fake".into(),
        root: fake.path().into(),
        source: std::fs::read_to_string(fake.path().join("lib.notc")).unwrap(),
    }])
    .unwrap();
    let mut vault = notist::Vault::open(fake.path()).with_environment(project);
    let notist::VaultError::Environment(errors) = vault.html_registry("test.not").unwrap_err()
    else {
        panic!("expected component binding diagnostics");
    };
    assert!(errors[0].diagnostic.message.contains("conflicting"));
}
#[test]
fn browser_host_uses_explicit_sources_and_urls_and_supports_module_inspection() {
    let config = "[dependencies]\nwidgets = {path = './widgets'}\n";
    let inputs = notist::PreparedInputs {
        root: "/preview".into(),
        config: Some("Notist.toml".into()),
        files: std::collections::BTreeMap::from([
            ("Notist.toml".into(), config.as_bytes().to_vec()),
            (
                "widgets/Notist.toml".into(),
                b"[package]\nname = 'widgets'".to_vec(),
            ),
            (
                "widgets/lib.notc".into(),
                b"fn panel()[children: Content] -> Content;".to_vec(),
            ),
            ("widgets/components/panel.js".into(), Vec::new()),
        ]),
        module_urls: std::collections::BTreeMap::from([(
            "widgets/components/panel.js".into(),
            "https://example.test/panel.js".into(),
        )]),
    };
    let message = serde_json::to_string(&inputs).unwrap();
    let preview: serde_json::Value = serde_json::from_str(&notist::preview::analyze_prepared(
        "test.not",
        "#widgets::panel()[content]",
        &message,
    ))
    .unwrap();
    assert!(preview["diagnostics"].as_array().unwrap().is_empty());
    assert!(preview["html"].as_str().unwrap().contains("<widgets-panel"));
    assert_eq!(
        preview["used_components"][0]["module"],
        "https://example.test/panel.js"
    );
    let module: serde_json::Value = serde_json::from_str(&notist::preview::analyze_prepared(
        "lib.notc",
        "fn f(value: Int = false) -> Content; fn g() -> Content;",
        &message,
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
    let analysis = notist::Pipeline::default()
        .analyze("test.not", "#native::meter()", &semantics)
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
    let result = notist::Pipeline::default()
        .analyze(
            "test.not",
            "before #widgets::popup()[inside] after",
            &semantics,
        )
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
