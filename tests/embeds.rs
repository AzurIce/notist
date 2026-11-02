use notist::{
    Context, Ctor, DiagnosticCode, Item, MemoryResources, Pipeline, ResourcePath, Type, Value,
    Vault,
};
use notist_html::Renderer;

const MODULE: &str = r#"
fn identity(target: Path) -> Path { target }
fn forward(target: Path) -> InlineContent { [#embed(identity(target))] }
fn defaulted(target: Path = p"./default.svg") -> InlineContent { [#self.forward(target)] }
fn own() -> InlineContent { [#embed(p"./own.svg")] }
fn legacy(target: String, query?: String, fragment?: String) -> InlineContent {
    [#embed(target, query: query, fragment: fragment)]
}
fn suffix(target: Path, fragment?: String) -> InlineContent { [#embed(target, fragment: fragment)] }
fn resource(target: ResourceTarget) -> InlineContent { [#embed(target)] }
fn hidden(target: ResourceTarget, description?: String, title?: String, query?: String, fragment?: String) -> InlineContent { [hidden] }
fn native(target: ResourceTarget, description?: String, title?: String, query?: String, fragment?: String) -> InlineContent;
"#;

fn context() -> Context {
    let mut module = Pipeline::default()
        .compile_module_with_root(
            "content.not",
            "view",
            "/project/views/lib.notc",
            MODULE,
            "/project",
        )
        .unwrap();
    // Native metadata can express named-only parameters; source declarations
    // currently make every parameter positional. Match the full embed contract.
    for definition in &mut module.definitions.functions {
        if matches!(definition.id.name.as_str(), "hidden" | "native") {
            for parameter in &mut definition.parameters[1..] {
                parameter.positional = false;
            }
        }
    }
    Context::default()
        .with_resource_root("/project")
        .with_module(module)
        .unwrap()
}

fn expected(path: &str) -> Value {
    Value::Path(ResourcePath::resolve("/project", "doc.not", &format!("/{path}")).unwrap())
}

fn evaluate(path: &str, source: &str, context: &Context) -> Item {
    let pipeline = Pipeline::default();
    let analysis = pipeline.analyze(path, source, context).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{source}: {:?}",
        analysis.diagnostics()
    );
    let evaluation = pipeline.evaluate(&analysis, context);
    assert!(
        evaluation.diagnostics.is_empty(),
        "{source}: {:?}",
        evaluation.diagnostics
    );
    evaluation.root
}

fn embed(root: &Item) -> &Item {
    root.find(notist::embed::is_embed).expect("native embed")
}

#[test]
fn paths_forward_defaults_and_function_literals_keep_their_binding_sites() {
    for extension in ["not", "md"] {
        let path = format!("/project/pages/virtual.{extension}");
        for (source, target) in [
            (r#"#embed(p"./direct.svg")"#, "pages/direct.svg"),
            (r#"#self.forward(p"./caller.svg")"#, "pages/caller.svg"),
            (r#"#self.defaulted()"#, "views/default.svg"),
            (
                r#"#self.defaulted(p"../assets/override.svg")"#,
                "assets/override.svg",
            ),
            (r#"#self.own()"#, "views/own.svg"),
            (r#"#self.resource(p"./caller.svg")"#, "pages/caller.svg"),
        ] {
            let root = evaluate(&path, source, &context());
            assert_eq!(
                embed(&root).fields.get("target"),
                Some(&expected(target)),
                "{source}"
            );
        }
    }
}

#[test]
fn deferred_strings_bind_at_the_embed_operation_and_keep_the_captured_root() {
    let pipeline = Pipeline::default();
    let context = context();
    for source in [
        r#"#self.legacy("./image%20one.svg?q=1#part%2520")"#,
        r#"#self.legacy("/views/image%20one.svg", query: "q=1", fragment: "part%20")"#,
    ] {
        let analysis = pipeline
            .analyze("/project/pages/page.not", source, &context)
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let evaluation =
            pipeline.evaluate(&analysis, &context.clone().with_resource_root("/different"));
        assert!(
            evaluation.diagnostics.is_empty(),
            "{:?}",
            evaluation.diagnostics
        );
        let item = embed(&evaluation.root);
        assert_eq!(
            item.fields.get("target"),
            Some(&expected("views/image one.svg"))
        );
        assert_eq!(item.fields.get("query"), Some(&Value::Str("q=1".into())));
        assert_eq!(
            item.fields.get("fragment"),
            Some(&Value::Str("part%20".into()))
        );
    }
}

#[test]
fn legacy_sugar_decodes_url_paths_once_and_separates_query_and_svg_fragment() {
    for extension in ["not", "md"] {
        let source = "![Image](../assets/a%2520b%3F%23.svg?x=1&amp;y=2#part%2520)";
        let root = evaluate(
            &format!("/project/pages/page.{extension}"),
            source,
            &context(),
        );
        let item = embed(&root);
        assert_eq!(
            item.fields.get("target"),
            Some(&expected("assets/a%20b?#.svg"))
        );
        assert_eq!(
            item.fields.get("fragment"),
            Some(&Value::Str("part%20".into()))
        );
        // Only Markdown decodes character references in its surface syntax.
        let query = if extension == "md" {
            "x=1&y=2"
        } else {
            "x=1&amp;y=2"
        };
        assert_eq!(item.fields.get("query"), Some(&Value::Str(query.into())));
        let output = Renderer::new()
            .with_resource_resolver(|_, path| {
                assert_eq!(Value::Path(path.clone()), expected("assets/a%20b?#.svg"));
                Some("../published/a%2520b%3F%23.svg".into())
            })
            .render_with_diagnostics(&root);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        assert!(output.html.contains("a%2520b%3F%23.svg?x=1"));
        assert!(output.html.contains("#part%2520"));
    }
}

#[test]
fn explicit_path_filenames_are_not_split_or_url_decoded() {
    let root = evaluate(
        "/project/pages/virtual.not",
        r#"#embed(p"./a?b#c%20.svg", query: "raw=1&v=2", fragment: "图/part%20")"#,
        &context(),
    );
    assert_eq!(
        embed(&root).fields.get("target"),
        Some(&expected("pages/a?b#c%20.svg"))
    );
    let before = root.clone();
    let output = Renderer::new()
        .with_resource_resolver(|_, path| {
            assert_eq!(path.path(), "pages/a?b#c%20.svg");
            Some("media/a%3Fb%23c%2520.svg".into())
        })
        .render_with_diagnostics(&root);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(
        output
            .html
            .contains("href=\"media/a%3Fb%23c%2520.svg?raw=1&amp;v=2#%E5%9B%BE%2Fpart%2520\""),
        "{}",
        output.html
    );
    assert_eq!(root, before);
}

#[test]
fn optional_suffixes_forward_without_rebinding_paths_or_inventing_empty_fields() {
    for (source, fragment) in [
        (r#"#self.suffix(p"./image.svg")"#, None),
        (
            r#"#self.suffix(p"./image.svg", fragment: "part")"#,
            Some("part"),
        ),
        (r#"#self.suffix(p"./image.svg", fragment: "")"#, Some("")),
    ] {
        let root = evaluate("/project/pages/page.not", source, &context());
        let item = embed(&root);
        assert_eq!(
            item.fields.get("target"),
            Some(&expected("pages/image.svg"))
        );
        assert_eq!(
            item.fields.get("fragment"),
            fragment.map(|s| Value::Str(s.into())).as_ref()
        );
    }
}

#[test]
fn remote_targets_remain_urls_and_do_not_request_local_resource_bindings() {
    for target in [
        "https://example.test/a.svg?q=1#part%20one",
        "//cdn.test/a.svg#part",
        "data:image/svg+xml,bad",
    ] {
        let root = evaluate(
            "/project/pages/page.not",
            &format!("#embed(\"{target}\")"),
            &context(),
        );
        assert_eq!(
            embed(&root).fields.get("target"),
            Some(&Value::Str(target.into()))
        );
        assert!(embed(&root).fields.get("fragment").is_none());
        let output = Renderer::new()
            .with_resource_resolver(|_, _| panic!("remote embed requested a local resource"))
            .render_with_diagnostics(&root);
        if target.starts_with("data:") {
            assert_eq!(output.diagnostics.len(), 1);
            assert!(!output.html.contains("href="));
        } else {
            assert!(output.diagnostics.is_empty());
            assert!(output.html.contains(&format!("href=\"{target}\"")));
        }
    }
    let root = evaluate(
        "/project/pages/page.not",
        r#"#embed("https://example.test/a.svg#existing", query: "v=2")"#,
        &context(),
    );
    assert!(Renderer::new().render(&root).contains("a.svg?v=2#existing"));
}

#[test]
fn conflicting_suffixes_and_invalid_queries_fail_at_known_and_deferred_boundaries() {
    let context = context();
    for (target, suffix, message) in [
        ("a.svg?x=1", "query: \"y=2\"", "query supplied both"),
        (
            "a.svg#part",
            "fragment: \"other\"",
            "fragment supplied both",
        ),
        (
            "https://example.test/a?x=1",
            "query: \"y=2\"",
            "already contains a query",
        ),
        (
            "https://example.test/a#part",
            "fragment: \"other\"",
            "already contains a fragment",
        ),
        ("a.svg", "query: \"x=1#bad\"", "query must be"),
        ("a.svg", "query: \"x=1\\nbad\"", "query must be"),
    ] {
        let pipeline = Pipeline::default();
        let source = format!("#embed(\"{target}\", {suffix})");
        let analysis = pipeline
            .analyze("/project/page.not", &source, &context)
            .unwrap();
        assert!(
            analysis
                .diagnostics()
                .iter()
                .any(|d| d.message.contains(message)),
            "{source}: {:?}",
            analysis.diagnostics()
        );
        let source = format!("#self.legacy(\"{target}\", {suffix})");
        let analysis = pipeline
            .analyze("/project/page.not", &source, &context)
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let evaluation = pipeline.evaluate(&analysis, &context);
        assert!(
            evaluation
                .diagnostics
                .iter()
                .any(|d| d.message.contains(message)),
            "{source}: {:?}",
            evaluation.diagnostics
        );
    }
}

#[test]
fn local_paths_require_safe_host_urls_and_never_fall_back_to_url_strings() {
    let source = r#"#embed(p"./private/image.svg", fragment: "part")"#;
    let root = evaluate("/project/pages/page.not", source, &context());
    let output = Renderer::new()
        .with_url_resolver(|_, _| panic!("Path passed to URL resolver"))
        .render_with_diagnostics(&root);
    assert_eq!(output.diagnostics.len(), 1);
    assert_eq!(
        output.diagnostics[0].code,
        Some(DiagnosticCode::UnpublishedResource)
    );
    assert_eq!(output.diagnostics[0].span, embed(&root).span);
    assert!(!output.html.contains("href="));
    assert!(!output.html.contains("/project"));
    assert!(!output.html.contains("private/"));
    for url in ["javascript:bad", "java\nscript:bad", "data:text/html,bad"] {
        let output = Renderer::new()
            .with_resource_resolver(|_, _| Some(url.into()))
            .render_with_diagnostics(&root);
        assert_eq!(output.diagnostics.len(), 1);
        assert!(!output.html.contains("href="));
    }
    let output = Renderer::new()
        .with_resource_resolver(|_, _| Some("a.svg#other".into()))
        .render_with_diagnostics(&root);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|d| d.message.contains("already contains a fragment"))
    );
}

#[test]
fn links_keep_the_string_contract_and_do_not_use_the_path_resolver() {
    let root = evaluate(
        "/project/pages/page.not",
        "[Guide](guide.not?q=1#part)",
        &context(),
    );
    let link = root.find(|item| item.ctor == Ctor::Link).unwrap();
    assert_eq!(
        link.fields.get("target"),
        Some(&Value::Str("guide.not?q=1#part".into()))
    );
    let analysis = Pipeline::default()
        .analyze(
            "/project/pages/page.not",
            r#"#link(p"./guide.not")[Guide]"#,
            &context(),
        )
        .unwrap();
    assert!(
        analysis
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("must be string"))
    );
    let mut malformed = link.clone();
    malformed
        .fields
        .insert("target", expected("pages/guide.not"));
    let output = Renderer::new()
        .with_resource_resolver(|_, _| panic!("link migration is deferred"))
        .render_with_diagnostics(&malformed);
    assert_eq!(output.diagnostics.len(), 1);
    assert!(!output.html.contains("href="));
    assert!(!Type::String.accepts(&expected("pages/image.svg")));
    assert!(Type::ResourceTarget.accepts(&expected("pages/image.svg")));
    assert!(Type::ResourceTarget.accepts(&Value::Str("https://example.test".into())));
}

#[test]
fn execution_replacements_publish_only_effective_native_embeds() {
    use notist::extensions::{Replace, Replacement};
    let context = context()
        .with_extension(Replace::new([Replacement {
            from: notist::FunctionId::new("notist", "embed"),
            to: notist::FunctionId::new("view", "hidden"),
            span: Default::default(),
        }]))
        .unwrap();
    let root = evaluate(
        "/project/pages/page.not",
        r#"#embed(p"./missing.svg")"#,
        &context,
    );
    assert!(!root.descendants().any(notist::embed::is_embed));
    let output = Renderer::new()
        .with_resource_resolver(|_, _| panic!("expanded input must not publish"))
        .render_with_diagnostics(&root);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(output.html.contains("hidden"));
    let context = crate::context()
        .with_extension(Replace::new([Replacement {
            from: notist::FunctionId::new("view", "native"),
            to: notist::FunctionId::new("notist", "embed"),
            span: Default::default(),
        }]))
        .unwrap();
    let root = evaluate(
        "/project/pages/page.not",
        r#"#self.native("./image.svg#part")"#,
        &context,
    );
    assert_eq!(
        embed(&root).fields.get("target"),
        Some(&expected("pages/image.svg"))
    );
    assert_eq!(
        embed(&root).fields.get("fragment"),
        Some(&Value::Str("part".into()))
    );
}

#[test]
fn virtual_vault_preview_maps_resources_without_requiring_them_to_exist() {
    let mut vault = Vault::new(MemoryResources::new("/project"));
    let output = vault
        .render_html(
            "pages/not-on-disk.md",
            r#"#embed(p"../assets/a?b#c%.svg", fragment: "part")"#,
            Default::default(),
        )
        .unwrap();
    assert!(output.analysis.diagnostics().is_empty());
    assert!(output.evaluation.diagnostics.is_empty());
    assert!(output.rendered.diagnostics.is_empty());
    assert!(
        output
            .rendered
            .html
            .contains("href=\"../assets/a%3Fb%23c%25.svg#part\"")
    );
    assert_eq!(
        embed(&output.evaluation.root).fields.get("target"),
        Some(&expected("assets/a?b#c%.svg"))
    );
}

#[test]
fn deferred_outside_vault_embeds_report_one_host_boundary_diagnostic() {
    let mut resources = MemoryResources::new("/project");
    resources.insert("views/lib.notc", MODULE);
    let mut vault = Vault::new(resources);
    let output = vault
        .render_html(
            "pages/virtual.not",
            "#import \"../views/lib.notc\" as view;\n#view.legacy(\"../../private/a.svg\")",
            Default::default(),
        )
        .unwrap();
    assert!(output.analysis.diagnostics().is_empty());
    assert_eq!(
        output.evaluation.diagnostics.len(),
        1,
        "{:?}",
        output.evaluation.diagnostics
    );
    assert_eq!(
        output.evaluation.diagnostics[0].code,
        Some(DiagnosticCode::OutsideVault)
    );
    assert_eq!(
        output.evaluation.diagnostics[0].source.as_deref(),
        Some("/project/views/lib.notc")
    );
    assert!(
        output.rendered.diagnostics.is_empty(),
        "{:?}",
        output.rendered.diagnostics
    );
    assert!(!output.rendered.html.contains("href="));
}

#[test]
fn error_boundaries_hide_embeds_while_expanded_boundaries_expose_results() {
    use notist::EvaluationState;
    let image =
        Item::new(Ctor::Embed, Default::default()).with_field("target", expected("missing.svg"));
    let mut wrapper = Item::new(Ctor::Node, Default::default()).with_children(vec![image]);
    wrapper.evaluation = Some(EvaluationState::Error("hidden".into()));
    assert_eq!(notist::embed::targets(&wrapper).count(), 0);
    wrapper.evaluation = Some(EvaluationState::Expanded);
    assert_eq!(notist::embed::targets(&wrapper).count(), 1);
}

#[test]
fn an_explicit_native_embed_handler_uses_the_same_host_resource_contract() {
    let context = context();
    let root = evaluate(
        "/project/pages/page.not",
        r#"#self.native(p"./image.svg", fragment: "part")"#,
        &context,
    );
    let mut registry = notist_html::HtmlRegistry::default();
    registry
        .register(
            context
                .registry()
                .get(&notist::FunctionId::new("view", "native"))
                .unwrap(),
            notist_html::Target::Native(Ctor::Embed),
        )
        .unwrap();
    let output = Renderer::from_registry(registry)
        .with_resource_resolver(|_, path| {
            assert_eq!(Value::Path(path.clone()), expected("pages/image.svg"));
            Some("media/image.svg".into())
        })
        .render_with_diagnostics(&root);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(
        output.html.contains("href=\"media/image.svg#part\""),
        "{}",
        output.html
    );
}
