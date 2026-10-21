use notist::{Environment, MemoryResources, Vault};

fn resources() -> MemoryResources {
    let mut r = MemoryResources::new("/project");
    for (path, text) in [
        ("Notist.toml", ""),
        (
            "lib.notc",
            "fn stale() -> InlineContent { compile_error(\"stale\") }",
        ),
        ("dep/Notist.toml", ""),
        ("dep/lib.notc", "fn value() -> String { \"dependency\" }"),
    ] {
        r.insert(path, text.as_bytes().to_vec());
    }
    r
}

#[test]
fn code_tooling_uses_project_identity_dependencies_and_the_supplied_source() {
    let vault = Vault::new(resources());
    let source = "import \"dep/lib.notc\" as dep; fn helper() -> String { dep.value() } fn view() -> InlineContent { [#text(text: self.helper())] }";
    let analysis = vault.analyze_code("lib.notc", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(
        analysis.definitions.as_ref().unwrap().module,
        "/project/lib.notc"
    );
    assert_eq!(analysis.sources["/project/lib.notc"].as_ref(), source);
    assert!(analysis.sources.contains_key("/project/dep/lib.notc"));
    let json: serde_json::Value =
        serde_json::from_str(&notist::cst_json::module_json(source, &analysis)).unwrap();
    assert_eq!(json["functions"][1]["function"], "/project/lib.notc.view");
    assert!(json["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn module_tooling_reports_body_errors_without_a_markup_caller() {
    let vault = Vault::new(resources());
    let source = "fn bad() -> InlineContent { compile_error(\"broken\") } fn generic<C: Type>() -> C { 1 + }";
    let analysis = vault.analyze_code("lib.notc", source).unwrap();
    assert!(analysis.definitions.is_some());
    assert!(analysis.diagnostics.iter().any(|d| d.message == "broken"));
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|d| d.phase == notist::Phase::Syntax)
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|d| d.source.as_deref() == Some("/project/lib.notc"))
    );
}

#[test]
fn invalid_signatures_keep_the_recovered_module_tree_and_definition_source() {
    let source = "fn invalid(value: Unknown) -> InlineContent;";
    let analysis = Vault::new(resources())
        .analyze_code("lib.notc", source)
        .unwrap();
    assert!(analysis.definitions.is_none());
    assert!(!analysis.diagnostics.is_empty());
    assert_eq!(analysis.sources["/project/lib.notc"].as_ref(), source);
    let json: serde_json::Value =
        serde_json::from_str(&notist::cst_json::module_json(source, &analysis)).unwrap();
    assert_eq!(json["tree"]["kind"], "Module");
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["source"] == "/project/lib.notc")
    );
}

#[test]
fn explicitly_installed_code_preserves_its_path_and_can_be_checked_with_edits() {
    let old = "fn old() -> InlineContent { [old] }";
    let path = std::path::Path::new("/project/custom.notc");
    let mut inputs = MemoryResources::new("/project");
    inputs.insert(path, old.as_bytes().to_vec());
    let environment = Environment::load_from(&inputs, path).unwrap();
    let vault = Vault::new(inputs);
    let source = "fn new() -> InlineContent { [new] } fn view() -> InlineContent { self.new() }";
    let analysis = vault.analyze_code(path, source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.definitions.unwrap().module, "/project/custom.notc");
    assert_eq!(
        environment.context().program().sources()["/project/custom.notc"].as_ref(),
        old
    );
}

#[test]
fn defaulted_comptime_instances_validate_dependent_defaults_at_the_definition() {
    let source = "fn invalid<T: Type = String, value: T = 1>() -> InlineContent { [unused] }";
    let analysis = Vault::new(resources())
        .analyze_code("lib.notc", source)
        .unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|d| d.message.contains("comptime `value` requires string")),
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|d| d.source.as_deref() == Some("/project/lib.notc"))
    );
}
