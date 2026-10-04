use notist::{
    Ctor, FunctionId, Notist, Value,
    builtins::Level,
    project::{Package, Project},
};
fn environment() -> Project {
    Project::from_packages([Package { name: "widgets".into(), root: "virtual/widgets".into(), source: "fn panel(title: String = \"Panel\")[children: Content] -> Content; fn badge(label: String)[children: InlineContent] -> InlineContent; fn leaf(value?: Int) -> Content;".into() }]).unwrap()
}
#[test]
fn extensions_reflow_traverse_query_and_debug_with_resolved_identity() {
    let project = environment();
    let src = "before #widgets::badge(\"yes\")[inside] #widgets::panel()[\nbody #widgets::badge(\"nested\")\n] after";
    let analysis = project.analyzer().analyze("test.not", src).unwrap();
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
        notist::query::select(analysis.root(), "function:widgets::badge").len(),
        2
    );
    assert!(notist::query::select(analysis.root(), "level:block").contains(&panel));
    let json: serde_json::Value =
        serde_json::from_str(&notist::query::render_json(src, &[panel])).unwrap();
    assert_eq!(json[0]["function"], "widgets::panel");
    assert_eq!(json[0]["level"], "block");
    assert_eq!(json[0]["fields"]["title"], "Panel");
    assert!(!json[0]["children"].as_array().unwrap().is_empty());
    let debug: serde_json::Value = serde_json::from_str(
        &notist::cst_json::analyze_json_with_registry(src, project.registry()),
    )
    .unwrap();
    assert!(debug["diagnostics"].as_array().unwrap().is_empty());
    assert!(notist::dump::dump(panel).contains("[block]"));
}
#[test]
fn external_contract_errors_stay_distinct_from_unknown_calls() {
    let project = environment();
    for (source, needle) in [
        ("#widgets::badge(1)", "string"),
        ("#widgets::badge()", "requires"),
        ("#widgets::leaf()[body]", "children"),
        ("#widgets::badge(\"x\")[\n#divider()\n]", "inline"),
        ("#widgets::missing()", "unknown"),
        ("#widgets::sub::f()", "not supported"),
    ] {
        let result = project.analyzer().analyze("test.not", source).unwrap();
        assert!(
            result
                .diagnostics()
                .iter()
                .any(|d| d.message.to_lowercase().contains(needle)),
            "{source}: {:?}",
            result.diagnostics()
        );
    }
    let result = project
        .analyzer()
        .analyze("test.not", "#widgets::badge(1)")
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
        Notist::default()
            .analyze("test.not", "#widgets::badge(\"x\")")
            .unwrap()
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("unknown"))
    );
}
#[cfg(feature = "serde")]
#[test]
fn serialized_extensions_need_no_package_to_restore_category_and_children() {
    let root = environment()
        .analyzer()
        .analyze("test.not", "#widgets::panel()[content]")
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
    let legacy = serde_json::json!({"ctor":"Paragraph","fields":[],"children":[],"attrs":[]});
    assert_eq!(
        serde_json::from_value::<notist::Item>(legacy)
            .unwrap()
            .level,
        Level::Block
    );
}
#[test]
fn loader_uses_dependency_keys_nearest_config_and_editor_sources_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("packages/actual/components")).unwrap();
    std::fs::create_dir_all(root.join("nested/deep")).unwrap();
    let config = root.join("Notist.toml");
    std::fs::write(
        &config,
        "[dependencies]\nalias = {path = 'packages/actual'}\n",
    )
    .unwrap();
    let module = root.join("packages/actual/lib.notc");
    std::fs::write(&module, "fn badge() -> InlineContent;").unwrap();
    let project = Project::for_document(root.join("nested/deep/test.not"), None).unwrap();
    assert_eq!(project.config_path(), Some(config.as_path()));
    assert!(project.registry().resolve("alias::badge").is_ok());
    assert!(project.registry().resolve("actual::badge").is_err());
    let mut overlays = std::collections::BTreeMap::new();
    overlays.insert(
        module.clone(),
        "fn badge(count: Unknown) -> Content;".into(),
    );
    let errors = Project::load_with_sources(&config, &overlays).unwrap_err();
    assert_eq!(errors[0].path, module);
    assert!(errors[0].diagnostic.message.contains("unknown"));
    std::fs::write(root.join("nested/Notist.toml"), "").unwrap();
    let inner = Project::for_document(root.join("nested/deep/test.not"), None).unwrap();
    assert!(inner.packages().is_empty());
    assert!(
        Project::for_document(root.join("nested/deep/test.not"), Some(&config))
            .unwrap()
            .registry()
            .resolve("alias::badge")
            .is_ok()
    );
    std::fs::write(
        &config,
        "[dependencies]\nalias = {path = 'packages/actual'}\nmissing = {path = 'missing'}\n",
    )
    .unwrap();
    let errors = Project::load(&config).unwrap_err();
    assert_eq!(errors[0].path, config);
    assert!(!errors[0].diagnostic.span.is_empty());
    assert!(
        Project::from_packages([Package {
            name: "notist".into(),
            root: root.into(),
            source: "fn x() -> Content;".into()
        }])
        .is_err()
    );
}

#[test]
fn namespaced_identity_is_case_sensitive_and_same_local_names_stay_separate() {
    let project = Project::from_packages([
        Package {
            name: "one".into(),
            root: "one".into(),
            source: "fn A() -> InlineContent; fn a() -> InlineContent;".into(),
        },
        Package {
            name: "two".into(),
            root: "two".into(),
            source: "fn a() -> InlineContent;".into(),
        },
    ])
    .unwrap();
    let source = "#one::A() #one::a() #two::a()";
    let result = project.analyzer().analyze("test.not", source).unwrap();
    assert!(result.diagnostics().is_empty());
    for identity in ["one::A", "one::a", "two::a"] {
        assert_eq!(
            notist::query::select(result.root(), &format!("function:{identity}")).len(),
            1
        );
        assert_eq!(
            notist::query::select(result.root(), &format!("ctor:{identity}")).len(),
            1
        );
    }
    let item = notist::query::select(result.root(), "function:one::A")[0];
    assert!(notist::dump::dump(item).contains("one::A"));
    let json: serde_json::Value =
        serde_json::from_str(&notist::query::render_json(source, &[item])).unwrap();
    assert_eq!(json[0]["ctor"], "one::A");
}

#[cfg(feature = "serde")]
#[test]
fn float_serialization_and_query_wire_preserve_every_f64_bit_pattern() {
    for value in [
        0.0,
        -0.0,
        1.0,
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
        let json: serde_json::Value =
            serde_json::from_str(&notist::query::render_json("", &[&item])).unwrap();
        assert_eq!(
            json[0]["typed_fields"][1][0][1],
            serde_json::json!(["float", format!("{:016x}", value.to_bits())])
        );
    }
    // The old finite representation remains readable and unchanged.
    assert_eq!(
        serde_json::to_string(&Value::Float(1.5)).unwrap(),
        "{\"Float\":1.5}"
    );
}
