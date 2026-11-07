use notist::{
    Context, Ctor, Frontend, FrontendOptions, FunctionId, Item, MemoryResources, Pipeline,
    ResourcePath, Type, Value, Vault,
};

const MODULE: &str = r#"
fn capture(source: Path, extra?: Path) -> BlockContent;
fn bag(paths: Array<Path>, data: Dict) -> BlockContent;
fn identity(source: Path) -> Path { source }
fn forward(source: Path) -> BlockContent { [ #capture(identity(source)) ] }
fn defaulted(source: Path = p"./default.json") -> BlockContent { [ #capture(source) ] }
fn own() -> BlockContent { [ #capture(p"./own.json") ] }
fn nested() -> BlockContent {
    [
@(source: p"./annotation.json")
#divider()
    ]
}
fn optional(source?: Path) -> BlockContent { [ #capture(p"./own.json", extra: source) ] }
fn equal(source: Path) -> String { if source == p"./own.json" { "equal" } else { "different" } }
fn show_equal(source: Path) -> InlineContent { [#text(text: equal(source))] }
fn generic<T: Type>(source: T) -> BlockContent { [ #capture(source) ] }
fn fixed<source: Path = p"./fixed.json">() -> BlockContent { [ #capture(source) ] }
fn defaults(paths: Array<Path> = (p"./a.json", p"./b.json"), data: Dict = (file: p"./dict.json")) -> BlockContent {
    [ #bag(paths, data) ]
}
"#;

fn context() -> Context {
    Context::default()
        .with_resource_root("/project")
        .with_module(
            Pipeline::default()
                .compile_module_with_root(
                    "content.not",
                    "view",
                    "/project/views/lib.notc",
                    MODULE,
                    "/project",
                )
                .unwrap(),
        )
        .unwrap()
}

fn expected(path: &str) -> Value {
    Value::Path(ResourcePath::resolve("/project", "/project/doc.not", &format!("/{path}")).unwrap())
}

fn evaluated(path: &str, source: &str) -> Item {
    let context = context();
    let pipeline = Pipeline::default();
    let analysis = pipeline.analyze(path, source, &context).unwrap();
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
    evaluation.root
}

fn captured(root: &Item) -> &Item {
    root.find(|node| {
        node.ctor()
            .function_id()
            .is_some_and(|id| id.name == "capture")
    })
    .expect("capture output")
}

#[test]
fn text_frontends_preserve_path_literals_until_resolve() {
    let source = r#"#self.capture(source: p"./data/(甲)[乙]|\".json")"#;
    for frontend in [
        &notist::NotistFrontend as &dyn Frontend,
        &notist::MarkdownFrontend,
    ] {
        let output = frontend.compile(source, FrontendOptions::default());
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        let mut paths = Vec::new();
        fn visit(expr: &notist::expr::Expr, paths: &mut Vec<(String, notist::TextRange)>) {
            use notist::expr::Expr;
            match expr {
                Expr::PathLiteral { text, span } => paths.push((text.clone(), *span)),
                Expr::Fragment { children, .. } => {
                    for child in children {
                        visit(child, paths);
                    }
                }
                Expr::Call { args, .. } => {
                    for arg in args {
                        visit(arg.value(), paths);
                    }
                }
                _ => {}
            }
        }
        for expr in &output.forest {
            visit(expr, &mut paths);
        }
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].0, "./data/(甲)[乙]|\".json");
        assert_eq!(
            &source[usize::from(paths[0].1.start())..usize::from(paths[0].1.end())],
            r#"p"./data/(甲)[乙]|\".json""#
        );
    }
}

#[test]
fn caller_paths_defaults_module_literals_and_forwarding_keep_their_origins() {
    for extension in ["not", "md"] {
        let path = format!("/project/pages/page.{extension}");
        for (source, target) in [
            (r#"#self.forward(p"./data.json")"#, "pages/data.json"),
            (r#"#self.defaulted()"#, "views/default.json"),
            (
                r#"#self.defaulted(p"../data/custom.json")"#,
                "data/custom.json",
            ),
            (r#"#self.own()"#, "views/own.json"),
            (r#"#self.fixed()"#, "views/fixed.json"),
            (r#"#self.capture(p"/data/root.json")"#, "data/root.json"),
            (
                r#"#self.generic<Path>(p"./typed.json")"#,
                "pages/typed.json",
            ),
        ] {
            let root = evaluated(&path, source);
            assert_eq!(
                captured(&root).field("source"),
                Some(&expected(target)),
                "{source}"
            );
        }
        let root = evaluated(&path, r#"#self.optional(p"./extra.json")"#);
        assert_eq!(
            captured(&root).field("extra"),
            Some(&expected("pages/extra.json"))
        );
        let root = evaluated(&path, "#self.optional()");
        assert_eq!(captured(&root).field("extra"), None);
    }
}

#[test]
fn nested_defaults_collections_and_annotations_resolve_in_their_own_files() {
    for extension in ["not", "md"] {
        let path = format!("/project/pages/page.{extension}");
        let source = r#"@!(input: (file: p"./root.json"), sequence: (p"./a", p"./b"))
@(input: p"./first", input: p"./last")
#divider()

#self.nested()

#self.defaults()
"#;
        let root = evaluated(&path, source);
        let Some(Value::Dict(input)) = root.attrs.get("input") else {
            panic!("missing root dict")
        };
        assert_eq!(input.get("file"), Some(&expected("pages/root.json")));
        assert_eq!(
            root.attrs.get("sequence"),
            Some(&Value::Array(
                vec![expected("pages/a"), expected("pages/b")].into()
            ))
        );
        let dividers: Vec<_> = root
            .descendants()
            .filter(|node| node.ctor() == Ctor::Divider)
            .collect();
        assert_eq!(
            dividers[0].attrs.get("input"),
            Some(&expected("pages/last"))
        );
        assert_eq!(
            dividers[1].attrs.get("source"),
            Some(&expected("views/annotation.json"))
        );
        let bag = root
            .find(|node| node.ctor().function_id() == Some(FunctionId::new("view", "bag")))
            .unwrap();
        assert_eq!(
            bag.field("paths"),
            Some(&Value::Array(
                vec![expected("views/a.json"), expected("views/b.json")].into()
            ))
        );
        let Some(Value::Dict(data)) = bag.field("data") else {
            panic!("missing default dict")
        };
        assert_eq!(data.get("file"), Some(&expected("views/dict.json")));
    }
}

#[test]
fn path_identity_is_normalized_and_equality_is_not_string_conversion() {
    assert_eq!(expected("views/a/../own.json"), expected("views/own.json"));
    assert_ne!(expected("pages/own.json"), expected("views/own.json"));
    for (literal, result) in [
        (r#"p"../views/./own.json""#, "equal"),
        (r#"p"./own.json""#, "different"),
    ] {
        let root = evaluated(
            "/project/pages/page.not",
            &format!("#self.show_equal({literal})"),
        );
        assert!(
            root.descendants()
                .any(|node| node.field("text") == Some(&Value::Str(result.into())))
        );
    }
    let context = context();
    for source in [
        r#"#self.capture("./data.json")"#,
        r#"#text(text: p"./data.json")"#,
    ] {
        let analysis = Pipeline::default()
            .analyze("/project/pages/page.not", source, &context)
            .unwrap();
        assert!(
            analysis
                .diagnostics()
                .iter()
                .any(|d| d.message.contains("must be")),
            "{:?}",
            analysis.diagnostics()
        );
    }
    assert_eq!(Type::of(&expected("a")), Type::Path);
    assert!(!Type::String.accepts(&expected("a")));
    assert!(
        Type::Array(Some(Box::new(Type::Path))).accepts(&Value::Array(vec![expected("a")].into()))
    );
}

#[test]
fn invalid_paths_report_literal_ranges_in_both_frontends_and_module_defaults() {
    for extension in ["not", "md"] {
        for literal in [
            r#"p"../../outside""#,
            r#"p"""#,
            r#"p"bad\\name""#,
            r#"p"bad\nname""#,
            r#"p"https://example.com""#,
        ] {
            let source = format!("前文 @(input: (file: {literal}))#strong[正文]");
            let analysis = Pipeline::default()
                .analyze(
                    format!("/project/pages/page.{extension}"),
                    &source,
                    &context(),
                )
                .unwrap();
            let diagnostic = analysis
                .diagnostics()
                .iter()
                .find(|d| d.message.contains("path") || d.message.contains("paths"))
                .expect("invalid path diagnostic");
            assert_eq!(
                &source[usize::from(diagnostic.span.start())..usize::from(diagnostic.span.end())],
                literal,
                "{diagnostic:?}"
            );
        }
    }
    let source = r#"fn bad(source: Path = p"../outside") -> BlockContent;"#;
    let errors = Pipeline::default()
        .compile_module_with_root(
            "content.not",
            "bad",
            "/project/lib.notc",
            source,
            "/project",
        )
        .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.source.as_deref() == Some("/project/lib.notc")
                && &source[usize::from(error.span.start())..usize::from(error.span.end())]
                    == r#"p"../outside""#)
    );
}

#[test]
fn p_is_not_reserved_and_plain_text_and_raw_blocks_do_not_construct_paths() {
    let pipeline = Pipeline::default();
    let module = pipeline
        .compile_module("content.not", "p", "p.notc", "fn p(p: Path) -> Path { p }")
        .unwrap();
    assert_eq!(module.definitions.functions[0].parameters[0].ty, Type::Path);
    for extension in ["not", "md"] {
        let source = "p\"ordinary text\"\n\n```notc\np\"../../not-a-resource\"\n```";
        let analysis = pipeline
            .analyze(format!("doc.{extension}"), source, &Context::default())
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
    }
}

#[test]
fn real_imports_use_defining_paths_but_never_read_path_literal_targets() {
    use notist::resources::{ResourceError, ResourceKind, Resources};
    use std::{
        cell::RefCell,
        path::{Path, PathBuf},
    };
    struct Observed {
        inner: MemoryResources,
        reads: RefCell<Vec<PathBuf>>,
    }
    impl Resources for Observed {
        fn root(&self) -> &Path {
            self.inner.root()
        }
        fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
            self.reads.borrow_mut().push(path.into());
            self.inner.read(path)
        }
        fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
            self.inner.kind(path)
        }
        fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
            self.inner.entries(path)
        }
    }
    let mut inner = MemoryResources::new("/project");
    inner.insert("views/lib.notc", MODULE);
    let resources = Observed {
        inner,
        reads: RefCell::new(vec![]),
    };
    let mut vault = Vault::new(&resources);
    let source =
        "#import \"../views/lib.notc\" as view;\n#view.forward(p\"./does-not-exist.json\")";
    let analysis = vault.analyze("pages/page.not", source).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let evaluation = vault.evaluate("pages/page.not", &analysis).unwrap();
    assert!(
        evaluation.diagnostics.is_empty(),
        "{:?}",
        evaluation.diagnostics
    );
    assert_eq!(
        captured(&evaluation.root).field("source"),
        Some(&expected("pages/does-not-exist.json"))
    );
    assert_eq!(
        *resources.reads.borrow(),
        vec![PathBuf::from("/project/views/lib.notc")]
    );
}

#[test]
fn paths_have_explicit_debug_json_but_are_not_html_or_component_parameters() {
    let mut vault = Vault::new(MemoryResources::new("/project"));
    let output = vault
        .render_html(
            "pages/page.not",
            r#"@!(input: (file: p"./private.json"))
Visible"#,
            notist::RenderOptions::default(),
        )
        .unwrap();
    assert!(output.analysis.diagnostics().is_empty());
    assert!(!output.rendered.diagnostics.is_empty());
    assert!(!output.rendered.html.contains("private.json"));
    assert!(!output.rendered.html.contains("/project"));
    let json = notist::json::item(output.analysis.root(), output.analysis.sources()).unwrap();
    assert_eq!(
        json["attrs"]["input"]["file"]["$path"]["path"],
        "pages/private.json"
    );
    assert!(
        notist_html::components::encode_parameter(&Value::Array(
            vec![expected("private.json")].into()
        ))
        .is_err()
    );
}

#[test]
fn before_check_can_edit_path_literals_and_metadata_stays_literal_only() {
    use notist::expr::Expr;
    use notist::extensions::{
        DocumentContext, DocumentExtension, ExtensionResult, LoweredDocument,
    };
    #[derive(Debug)]
    struct Edit;
    impl DocumentExtension for Edit {
        fn name(&self) -> &str {
            "edit-path"
        }
        fn before_check(
            &self,
            _: &DocumentContext<'_>,
            document: &mut LoweredDocument,
        ) -> ExtensionResult {
            fn visit(expr: &mut Expr) {
                match expr {
                    Expr::PathLiteral { text, .. } => *text = "./edited.json".into(),
                    Expr::Fragment { children, .. } => {
                        for child in children {
                            visit(child);
                        }
                    }
                    Expr::Call { args, .. } => {
                        for arg in args {
                            visit(arg.value_mut());
                        }
                    }
                    _ => {}
                }
            }
            for expr in &mut document.forest {
                visit(expr);
            }
            document.attrs.insert_expr(
                "not-a-literal",
                Expr::call("notist.divider", Default::default()),
            );
            Ok(())
        }
    }
    let context = context().with_extension(Edit).unwrap();
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze(
            "/project/pages/page.not",
            r#"#self.forward(p"./original.json")"#,
            &context,
        )
        .unwrap();
    assert!(
        analysis
            .diagnostics()
            .iter()
            .any(|error| error.message == "expected a data literal")
    );
    assert!(analysis.root().attrs.get("not-a-literal").is_none());
    let evaluation = pipeline.evaluate(&analysis, &context);
    assert!(
        evaluation.diagnostics.is_empty(),
        "{:?}",
        evaluation.diagnostics
    );
    assert_eq!(
        captured(&evaluation.root).field("source"),
        Some(&expected("pages/edited.json"))
    );
}

#[test]
fn content_fragments_use_the_explicit_source_not_the_dialect_placeholder() {
    let pipeline = Pipeline::default();
    let context = context();
    for dialect in ["content.not", "content.md"] {
        let (analysis, _) = pipeline
            .analyze_content_literal_at(
                dialect,
                "/project/fragments/part.notc",
                "[\n#self.capture(p\"./input.json\")\n]",
                &context,
            )
            .unwrap();
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        assert_eq!(
            analysis.forest[0].field("source"),
            Some(&expected("fragments/input.json"))
        );
    }
    assert!(ResourcePath::resolve("/project", "<detached>", "./data.json").is_err());
    assert!(ResourcePath::resolve("/project", "/project/pages/", "./data.json").is_err());
    assert!(ResourcePath::resolve("/project", "/project", "/data.json").is_err());
}

#[test]
fn unterminated_or_multiline_paths_do_not_consume_following_declarations() {
    use notist::syntax::{code, markup::MarkupContentParser};
    for broken in ["p\"unfinished", "p\"\"\"unsupported"] {
        let source = format!("fn bad() -> Path {{ {broken}\n}}\nfn good() -> Path {{ p\"./ok\" }}");
        let parsed = code::parse_module(&source, &MarkupContentParser);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert!(!parsed.diagnostics.is_empty());
        let module = code::ast::Module::cast(parsed.syntax()).unwrap();
        let names: Vec<_> = module
            .functions()
            .map(|f| f.name().unwrap().text().to_string())
            .collect();
        assert_eq!(names, ["bad", "good"]);
    }
}

#[test]
fn filesystem_and_memory_hosts_bind_identical_logical_paths_without_target_files() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("views")).unwrap();
    std::fs::write(directory.path().join("views/lib.notc"), MODULE).unwrap();
    let mut memory = MemoryResources::new(directory.path());
    memory.insert("views/lib.notc", MODULE);
    let source = "#import \"../views/lib.notc\" as view;\n#view.defaulted()";
    let mut filesystem = Vault::new(notist::FsResources::new(directory.path()));
    let analysis = filesystem.analyze("pages/page.not", source).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let a = filesystem.evaluate("pages/page.not", &analysis).unwrap();
    let mut memory = Vault::new(memory);
    let analysis = memory.analyze("pages/page.not", source).unwrap();
    let b = memory.evaluate("pages/page.not", &analysis).unwrap();
    assert!(a.diagnostics.is_empty() && b.diagnostics.is_empty());
    assert_eq!(captured(&a.root).fields(), captured(&b.root).fields());
    assert!(!directory.path().join("views/default.json").exists());
}

#[cfg(any(feature = "serde", feature = "command-extensions"))]
#[test]
fn serialized_path_values_round_trip_and_reject_noncanonical_identity() {
    let value = expected("views/data.json");
    let serialized = serde_json::to_value(&value).unwrap();
    assert_eq!(serde_json::from_value::<Value>(serialized).unwrap(), value);
    for path in [
        "../escape",
        "x/../data",
        "/absolute",
        "bad\\path",
        "bad\npath",
    ] {
        assert!(
            serde_json::from_value::<ResourcePath>(
                serde_json::json!({"root":"/project", "path":path})
            )
            .is_err()
        );
    }
}

#[cfg(any(feature = "serde", feature = "command-extensions"))]
#[test]
fn metadata_wire_keeps_ordinary_values_and_preserves_unresolved_path_ranges() {
    let output = notist::NotistFrontend.compile(
        r#"@!(label: "plain", file: p"./data.json")
Text"#,
        FrontendOptions::default(),
    );
    let serialized = serde_json::to_value(&output.module_attrs).unwrap();
    assert_eq!(serialized[0], serde_json::json!(["label", {"Str":"plain"}]));
    assert_eq!(
        serialized[1][1]["expression"]["PathLiteral"]["text"],
        "./data.json"
    );
    let restored: notist::LiteralDict = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored.get("file"), output.module_attrs.get("file"));
    let previous: notist::LiteralDict =
        serde_json::from_value(serde_json::json!([["label", {"Str":"plain"}]])).unwrap();
    assert_eq!(
        previous
            .get("label")
            .and_then(notist::expr::Expr::literal_value),
        Some(&Value::Str("plain".into()))
    );
}

#[test]
fn component_parameters_containing_paths_are_diagnosed_and_never_serialized() {
    let pipeline = Pipeline::default();
    let context = context();
    let analysis = pipeline
        .analyze(
            "/project/pages/page.not",
            r#"#self.capture(p"./secret.json")"#,
            &context,
        )
        .unwrap();
    let evaluation = pipeline.evaluate(&analysis, &context);
    let mut registry = notist_html::HtmlRegistry::default();
    registry
        .bind_component(
            context.registry().function("view", "capture").unwrap(),
            "https://example.test/component.js",
        )
        .unwrap();
    let rendered =
        notist_html::Renderer::from_registry(registry).render_with_diagnostics(&evaluation.root);
    assert!(
        rendered
            .diagnostics
            .iter()
            .any(|error| error.message.contains("Path"))
    );
    assert!(!rendered.html.contains("secret.json") && !rendered.html.contains("/project"));
}

#[test]
fn each_module_retains_its_declared_root_even_when_source_labels_match() {
    let pipeline = Pipeline::default();
    let source = r#"fn capture(source: Path) -> BlockContent;
fn show() -> BlockContent { [ #capture(p"/value.json") ] }"#;
    let mut context = Context::default().with_resource_root("/document");
    for (module, root) in [("one", "/virtual"), ("two", "/")] {
        context = context
            .with_module(
                pipeline
                    .compile_module_with_root(
                        "content.not",
                        module,
                        "/virtual/lib.notc",
                        source,
                        root,
                    )
                    .unwrap(),
            )
            .unwrap();
    }
    context = context.with_scope(
        "doc",
        [
            ("one".into(), notist::ImportBinding::Module("one".into())),
            ("two".into(), notist::ImportBinding::Module("two".into())),
        ],
    );
    let analysis = pipeline
        .analyze("/document/page.not", "#one.show()\n\n#two.show()", &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let evaluation = pipeline.evaluate(&analysis, &context);
    assert!(
        evaluation.diagnostics.is_empty(),
        "{:?}",
        evaluation.diagnostics
    );
    for (module, root) in [("one", "/virtual"), ("two", "/")] {
        let item = evaluation
            .root
            .find(|node| node.ctor().function_id() == Some(FunctionId::new(module, "capture")))
            .unwrap();
        assert_eq!(
            item.field("source"),
            Some(&Value::Path(
                ResourcePath::resolve(root, "/virtual/lib.notc", "/value.json").unwrap()
            ))
        );
    }
}
