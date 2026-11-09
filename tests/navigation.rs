use notist::{Environment, MemoryResources, Pipeline};

#[test]
fn prepared_navigation_preserves_lexical_shadowing_and_reserved_namespaces() {
    let pipeline = Pipeline::default();
    for (body, target) in [
        ("fn test(value: Int) -> Int { value() }", None),
        ("fn test<value: Type>() -> Int { value() }", None),
        (
            "fn test()[children: InlineContent] -> Int { children() }",
            None,
        ),
        ("fn test() -> Int { let value = 1; value() }", None),
        (
            "fn test() -> Int { let value = value(); value }",
            Some("value"),
        ),
        ("fn test(value: Int) -> Int { self.value() }", Some("value")),
        (
            "fn test(value: Int) -> InlineContent { notist.text(\"text\") }",
            Some("text"),
        ),
    ] {
        let source = format!("import \"dep.notc\": value, children;\n{body}");
        let mut resources = MemoryResources::new("/project");
        resources.insert("main.notc", source.as_bytes().to_vec());
        resources.insert(
            "dep.notc",
            b"fn value() -> Int { 1 } fn children() -> Int { 2 }".to_vec(),
        );
        let environment = Environment::load_from(&resources, "main.notc").unwrap();
        let navigation = pipeline
            .source_navigation("/project/main.notc", &source, environment.context())
            .unwrap();
        let offset = if let Some(offset) = source.find("notist.text") {
            offset
        } else if let Some(offset) = source.find("self.value") {
            offset
        } else {
            source.rfind("()").unwrap()
                - if body.contains("children()") {
                    "children".len()
                } else {
                    "value".len()
                }
        };
        for _ in 0..3 {
            assert_eq!(
                navigation
                    .definition_at(environment.context(), offset)
                    .map(|definition| definition.id.name.as_str()),
                target,
                "{source}"
            );
        }
    }
}

#[test]
fn prepared_navigation_resolves_import_aliases_and_nested_calls_in_all_frontends() {
    let pipeline = Pipeline::default();
    let mut resources = MemoryResources::new("/project");
    resources.insert("dep.notc", b"fn value() -> String { \"text\" }".to_vec());
    resources.insert(
        "root.notc",
        b"import \"dep.notc\": value as renamed;".to_vec(),
    );
    let environment = Environment::load_from(&resources, "root.notc").unwrap();
    for path in ["doc.not", "doc.md", "root.notc"] {
        let prefix = if path.ends_with("notc") { "" } else { "#" };
        let body = if prefix.is_empty() {
            "fn test() -> String { renamed() }"
        } else {
            "#text(renamed())"
        };
        let source = format!("{prefix}import \"dep.notc\": value as renamed;\n{body}");
        let navigation = pipeline
            .source_navigation(path, &source, environment.context())
            .unwrap();
        for offset in [
            source.find("value as").unwrap(),
            source.rfind("renamed()").unwrap(),
        ] {
            let definition = navigation
                .definition_at(environment.context(), offset)
                .unwrap();
            assert_eq!(definition.id.name, "value");
            assert_eq!(definition.id.module, "/project/dep.notc");
        }
    }
}
