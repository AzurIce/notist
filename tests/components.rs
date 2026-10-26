use notist::{Dict, Environment, FunctionId, Value};
use notist_html::{HtmlRegistry, Renderer};
fn definitions() -> notist::DefinitionModule {
    notist_pipeline::Pipeline::default().compile_module("content.not", "widgets", "widgets/lib.notc", "fn panel(title: String = \"\", open: Bool = false, count: Int = 9223372036854775807, ratio: Float = 1.5, optional?: String, data: Array = (false,))[children: BlockContent] -> BlockContent; fn badge(label: String) -> InlineContent;").unwrap().definitions
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
            "#self.panel(title: \"<&\\\"\")[ #self.badge(\"one\") #self.badge(\"two\") ]",
            &notist::Context::new(registry.clone()).with_root("widgets"),
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
fn html_registry_distinguishes_modules_and_rejects_attribute_and_reserved_collisions() {
    let mut registry = HtmlRegistry::default();
    let first = notist_pipeline::Pipeline::default()
        .compile_module(
            "content.not",
            "a-b",
            "a-b/lib.notc",
            "fn c() -> BlockContent;",
        )
        .unwrap();
    let second = notist_pipeline::Pipeline::default()
        .compile_module(
            "content.not",
            "a",
            "a/lib.notc",
            "fn b-c() -> BlockContent;",
        )
        .unwrap();
    registry
        .bind_component(&first.definitions.functions[0], "first.js")
        .unwrap();
    registry
        .bind_component(&second.definitions.functions[0], "second.js")
        .unwrap();
    assert_ne!(
        notist_html::components::component_tag(&first.definitions.functions[0].id),
        notist_html::components::component_tag(&second.definitions.functions[0].id)
    );
    let uppercase = notist_pipeline::Pipeline::default()
        .compile_module(
            "content.not",
            "case",
            "case/lib.notc",
            "fn A() -> BlockContent; fn u41x() -> BlockContent;",
        )
        .unwrap();
    registry
        .bind_component(&uppercase.definitions.functions[0], "a.js")
        .unwrap();
    assert!(
        registry
            .bind_component(&uppercase.definitions.functions[1], "b.js")
            .is_err()
    );
    for source in [
        "fn f(protocol: String) -> BlockContent;",
        "fn f(A: String, u41x: String) -> BlockContent;",
    ] {
        let definition = notist_pipeline::Pipeline::default()
            .compile_module("content.not", "names", "names/lib.notc", source)
            .unwrap();
        assert!(
            registry
                .bind_component(&definition.definitions.functions[0], "f.js")
                .is_err()
        );
    }
    assert_eq!(
        notist_html::components::component_tag(&FunctionId::new("包", "图")),
        format!(
            "notist-{}-u56fex",
            notist_html::components::module_key("包")
        )
    );
}
#[test]
fn missing_html_targets_and_filesystem_entry_conflicts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let project =
        Environment::load_from(&notist::FsResources::new(&root), "components/lib.notc").unwrap();
    let missing = notist::Pipeline::default()
        .analyze("test.not", "#self.badge(\"test\")", project.context())
        .unwrap();
    let rendered = Renderer::new().render_with_diagnostics(missing.root());
    assert!(
        rendered
            .diagnostics
            .iter()
            .any(|error| error.message.contains("no HTML implementation"))
    );
    let fake = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(fake.path().join("f")).unwrap();
    std::fs::write(fake.path().join("lib.notc"), "fn f() -> BlockContent;").unwrap();
    std::fs::write(fake.path().join("f.js"), "export default class {}").unwrap();
    std::fs::write(fake.path().join("f/index.js"), "export default class {}").unwrap();
    let mut vault = notist::Vault::open(fake.path());
    let source = "#import \"./lib.notc\" as ui;\n#ui.f()";
    let analysis = vault.analyze("test.not", source).unwrap();
    let evaluated = vault.evaluate("test.not", &analysis).unwrap();
    let notist::VaultError::Environment(errors) = vault
        .html_registry("test.not", source, &evaluated.root)
        .unwrap_err()
    else {
        panic!("expected component binding diagnostics");
    };
    assert!(errors[0].diagnostic.message.contains("conflicting"));
}

#[test]
fn html_binding_ignores_unused_and_hidden_native_calls() {
    let mut inputs = notist::MemoryResources::new("/notes");
    inputs.insert(
        "ui.notc",
        br#"
fn unused() -> InlineContent;
fn used() -> InlineContent;
fn discard[children: InlineContent] -> InlineContent { [kept] }
fn fail[children: InlineContent] -> InlineContent { compile_error("broken") }
"#
        .to_vec(),
    );
    inputs.insert("unused.js", b"export default class {}".to_vec());
    inputs.insert("unused/index.js", b"export default class {}".to_vec());
    inputs.insert("used.js", b"export default class {}".to_vec());
    let mut vault = notist::Vault::new(inputs.clone());
    for content in [
        "#ui.discard[#ui.unused()] #ui.used()",
        "#ui.fail[#ui.unused()] #ui.used()",
    ] {
        let source = format!("#import \"ui.notc\" as ui;\n{content}");
        let output = vault
            .render_html("doc.not", &source, notist::RenderOptions::default())
            .unwrap();
        assert!(
            output.rendered.diagnostics.is_empty(),
            "{:?}",
            output.rendered.diagnostics
        );
        assert_eq!(output.rendered.used_components.len(), 1);
        assert_eq!(output.rendered.used_components[0].id.name, "used");
        let mut prepared = notist::PreparedInputs {
            root: "/notes".into(),
            config: None,
            files: inputs.files().clone(),
            module_urls: Default::default(),
        };
        prepared
            .files
            .insert("/notes/doc.not".into(), source.as_bytes().to_vec());
        let description: serde_json::Value =
            serde_json::from_str(&notist::preview::describe_prepared(
                "/notes/doc.not",
                &serde_json::to_string(&prepared).unwrap(),
            ))
            .unwrap();
        assert_eq!(description["functions"].as_array().unwrap().len(), 1);
        assert_eq!(description["functions"][0]["name"], "used");
        let code: serde_json::Value = serde_json::from_str(&notist::preview::describe_prepared(
            "/notes/ui.notc",
            &serde_json::to_string(&prepared).unwrap(),
        ))
        .unwrap();
        assert!(code["functions"].as_array().unwrap().is_empty());
    }
}
#[test]
fn browser_host_uses_explicit_sources_and_urls_and_supports_module_inspection() {
    let config = "";
    let inputs = notist::PreparedInputs {
        root: "/preview".into(),
        config: Some("Notist.toml".into()),
        files: std::collections::BTreeMap::from([
            (
                "lib.notc".into(),
                b"import \"widgets/lib.notc\" as widgets;".to_vec(),
            ),
            ("Notist.toml".into(), config.as_bytes().to_vec()),
            ("widgets/Notist.toml".into(), b"".to_vec()),
            (
                "widgets/lib.notc".into(),
                b"fn panel<C: Type>()[children: C] -> BlockContent;".to_vec(),
            ),
            ("widgets/panel.js".into(), Vec::new()),
        ]),
        module_urls: std::collections::BTreeMap::from([(
            "widgets/panel.js".into(),
            "https://example.test/panel.js".into(),
        )]),
    };
    let message = serde_json::to_string(&inputs).unwrap();
    let preview: serde_json::Value = serde_json::from_str(&notist::preview::analyze_prepared(
        "test.not",
        "#import \"widgets/lib.notc\" as widgets;\n#widgets.panel()[content]",
        &message,
    ))
    .unwrap();
    assert!(preview["diagnostics"].as_array().unwrap().is_empty());
    assert!(preview["html"].as_str().unwrap().contains(&format!(
        "<{} ",
        preview["used_components"][0]["tag"].as_str().unwrap()
    )));
    assert_eq!(
        preview["used_components"][0]["module"],
        "https://example.test/panel.js"
    );
    let module: serde_json::Value = serde_json::from_str(&notist::preview::analyze_prepared(
        "lib.notc",
        "fn f(value: Int = false) -> BlockContent; fn g() -> BlockContent;",
        &message,
    ))
    .unwrap();
    assert_eq!(module["tree"]["kind"], "Module");
    assert_eq!(module["ast"]["kind"], "Module");
    assert_eq!(module["ast"]["children"].as_array().unwrap().len(), 2);
    assert!(!module["diagnostics"].as_array().unwrap().is_empty());
    assert!(
        notist::environment::parse_config(config)
            .unwrap()
            .extensions
            .is_empty()
    );
}

#[test]
fn native_nonfinite_defaults_use_the_same_component_binding_path() {
    let mut module = notist::DefinitionModule::new("native");
    let mut definition = notist::FunctionDef::new(
        notist::FunctionId::new("native", "meter"),
        notist::builtins::Accepts::Nothing,
        notist::Type::InlineContent,
    );
    definition.parameters.push(notist::ParameterDef::new(
        "reading",
        notist::Type::Float,
        notist::ParameterMode::Default(Value::Float(f64::NAN)),
    ));
    module.functions.push(definition.clone());
    let mut semantics = notist::builtins::registry().clone();
    semantics.register(module).unwrap();
    let mut html = HtmlRegistry::default();
    html.bind_component(&definition, "meter.js").unwrap();
    let analysis = notist::Pipeline::default()
        .analyze(
            "test.not",
            "#self.meter()",
            &notist::Context::new(semantics.clone()).with_root("native"),
        )
        .unwrap();
    let result = Renderer::new()
        .with_registry(html)
        .render_with_diagnostics(analysis.root());
    assert!(result.diagnostics.is_empty());
    assert!(result.html.contains("notist-reading=\"NaN\""));
}

#[test]
fn inline_component_accepting_block_children_keeps_a_parseable_html_tree() {
    let module = notist_pipeline::Pipeline::default()
        .compile_module(
            "content.not",
            "widgets",
            "widgets/lib.notc",
            "fn popup()[children: BlockContent] -> InlineContent;",
        )
        .unwrap();
    let mut semantics = notist::builtins::registry().clone();
    let mut html = HtmlRegistry::default();
    html.bind_component(&module.definitions.functions[0], "popup.js")
        .unwrap();
    semantics.register(module.definitions).unwrap();
    let result = notist::Pipeline::default()
        .analyze(
            "test.not",
            "before #self.popup()[ inside ] after",
            &notist::Context::new(semantics).with_root("widgets"),
        )
        .unwrap();
    assert!(result.diagnostics().is_empty());
    let rendered = Renderer::new()
        .with_registry(html)
        .render_with_diagnostics(result.root());
    assert!(rendered.diagnostics.is_empty());
    assert_eq!(
        rendered.html,
        format!(
            "<div class=\"notist-paragraph\">before <{tag} notist-protocol=\"2\" notist-function=\"widgets.popup\" notist-content=\"inline\"><p>inside</p></{tag}> after</div>",
            tag = notist_html::components::component_tag(&FunctionId::new("widgets", "popup"))
        )
    );
}

#[test]
fn docs_status_annotations_keep_native_nodes_and_nested_component_bindings() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let mut vault = notist::Vault::open(&root);
    for status in ["draft", "staged", "outdate"] {
        let source = format!(
            "@(status: \"{status}\")\n= Status\n\nFirst paragraph.\n\n#import \"components/lib.notc\" as components;\n#components.badge(\"Nested\")"
        );
        let output = vault
            .render_html("status.not", &source, notist::RenderOptions::default())
            .unwrap();
        assert!(output.analysis.diagnostics().is_empty());
        assert!(output.rendered.diagnostics.is_empty());
        let section = &output.analysis.root().children[0];
        assert_eq!(section.ctor, notist::Ctor::Section);
        assert_eq!(
            section.attrs.get("status"),
            Some(&Value::Str(status.into()))
        );
        assert_eq!(section.children.len(), 3);
        assert_eq!(section.children[0].ctor, notist::Ctor::Heading);
        assert!(
            output
                .rendered
                .html
                .contains(&format!("data-status=\"{status}\""))
        );
        assert!(output.rendered.html.contains("First paragraph."));
        assert!(
            output
                .rendered
                .html
                .contains(&format!("<{} ", output.rendered.used_components[0].tag))
        );
        assert_eq!(
            output
                .rendered
                .used_components
                .iter()
                .map(|component| component.id.clone())
                .collect::<Vec<_>>(),
            vec![FunctionId::new(
                root.join("components/lib.notc").to_str().unwrap(),
                "badge"
            )]
        );
    }
}

#[test]
fn generic_component_fields_use_the_execution_targets_concrete_types() {
    use notist::{Context, FunctionId, Pipeline};
    use notist_html::{HtmlRegistry, Renderer};
    let module = notist_pipeline::Pipeline::default().compile_module("content.not", "generic", "generic/lib.notc", "fn source<T: Type>(value: Array<T>) -> InlineContent; fn target<U: Type>(value: Array<U>) -> InlineContent;").unwrap();
    let context = Context::default()
        .with_module(module)
        .unwrap()
        .with_extension(notist::extensions::Replace::new([
            notist::extensions::Replacement {
                from: FunctionId::new("generic", "source"),
                to: FunctionId::new("generic", "target"),
                span: Default::default(),
            },
        ]))
        .unwrap();
    let p = Pipeline::default();
    let a = p
        .analyze("doc.not", "#self.source((1, 2))", &context)
        .unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let evaluated = p.evaluate(&a, &context);
    assert!(
        evaluated.diagnostics.is_empty(),
        "{:?}",
        evaluated.diagnostics
    );
    let mut registry = HtmlRegistry::default();
    registry
        .bind_component(
            context
                .registry()
                .get(&FunctionId::new("generic", "target"))
                .unwrap(),
            "/target.js",
        )
        .unwrap();
    let rendered = Renderer::new()
        .with_registry(registry)
        .render_with_diagnostics(&evaluated.root);
    assert!(
        rendered.diagnostics.is_empty(),
        "{:?}",
        rendered.diagnostics
    );
    assert!(
        rendered
            .html
            .contains(&format!("<{} ", rendered.used_components[0].tag))
    );
    assert!(rendered.html.contains("notist-value="), "{}", rendered.html);
}
