#[test]
fn pipeline_dispatch_by_extension() {
    let pipeline = notist::Frontends::default();
    let context = notist::Context::default();
    let (item, diags) = pipeline
        .analyze(std::path::Path::new("x.md"), "# 标题\n", &context)
        .expect("md frontend");
    assert!(diags.is_empty());
    assert!(
        item.descendants()
            .any(|i| i.ctor == notist::item::Ctor::Heading)
    );
    assert!(
        pipeline
            .analyze(std::path::Path::new("x.not"), "正文\n", &context)
            .is_some()
    );
    assert!(
        pipeline
            .analyze(std::path::Path::new("x.txt"), "正文\n", &context)
            .is_none()
    );
}

#[test]
fn default_pipeline_produces_renderable_ir_for_both_formats() {
    use notist::{Ctor, Pipeline, Value};

    let notist = Pipeline::default();
    for (path, src) in [
        ("document.not", "= 标题\n\n正文 *强调*\n"),
        ("document.md", "# 标题\n\n正文 **强调**\n"),
        ("document.markdown", "# 标题\n\n正文 **强调**\n"),
        ("document.notmd", "# 标题\n\n正文 **强调**\n"),
        ("document.nmd", "# 标题\n\n正文 **强调**\n"),
    ] {
        let document = notist
            .analyze(path, src, &notist::Context::default())
            .unwrap();
        assert!(document.diagnostics().is_empty(), "{path}");
        assert_eq!(document.root().ctor, Ctor::Doc);
        let section = &document.root().children[0];
        assert_eq!(section.ctor, Ctor::Section);
        assert_eq!(section.children[0].ctor, Ctor::Heading);
        let strong = document
            .root()
            .find(|node| node.ctor == Ctor::Strong)
            .unwrap();
        assert_eq!(
            strong.children[0].fields.get("text"),
            Some(&Value::Str("强调".into()))
        );
    }
}

#[test]
fn pipeline_can_install_only_selected_frontends() {
    use notist::{NotistFrontend, Pipeline};

    assert!(
        Pipeline::new()
            .analyze("document.not", "正文", &notist::Context::default())
            .is_err()
    );
    let notist = Pipeline::new().with_frontend(NotistFrontend);
    assert!(
        notist
            .analyze("document.not", "正文", &notist::Context::default())
            .is_ok()
    );
    let error = notist
        .analyze("document.md", "正文", &notist::Context::default())
        .unwrap_err();
    assert_eq!(error.path, std::path::Path::new("document.md"));
    assert!(
        notist
            .analyze("untitled", "正文", &notist::Context::default())
            .is_err()
    );
}

struct PlainTextFrontend {
    extensions: Vec<&'static str>,
}

impl notist::syntax::content::ContentParser for PlainTextFrontend {
    fn parse_content(
        &self,
        source: &str,
        open: notist::TextSize,
    ) -> notist::syntax::content::ParsedContent {
        use notist::syntax::content::{ContentFlavor, ParsedContent};
        let start = usize::from(open) + 1;
        let end = source[start..].find(']').map(|i| start + i);
        let body_end = end.unwrap_or(source.len());
        ParsedContent {
            range: notist::TextRange::new(
                open,
                ((body_end + usize::from(end.is_some())) as u32).into(),
            ),
            body: notist::TextRange::new((start as u32).into(), (body_end as u32).into()),
            flavor: ContentFlavor::Inline,
            closed: end.is_some(),
            syntax: Box::new(source[start..body_end].to_string()),
            diagnostics: Vec::new(),
        }
    }
}

impl notist::ContentLowering for PlainTextFrontend {
    fn lower_content(
        &self,
        content: &notist::syntax::content::ParsedContent,
        _: &mut Vec<notist::Diagnostic>,
    ) -> notist::Lowered {
        let text = content
            .syntax
            .as_ref()
            .as_any()
            .downcast_ref::<String>()
            .unwrap();
        notist::Lowered {
            forest: vec![notist::expr::Expr::text(text.clone(), content.body)],
            annotations: Vec::new(),
        }
    }
}

impl notist::Frontend for PlainTextFrontend {
    fn extensions(&self) -> &[&str] {
        &self.extensions
    }

    fn compile(&self, src: &str, options: notist::FrontendOptions) -> notist::FrontendOutput {
        let span = notist::TextRange::new(0.into(), (src.len() as u32).into());
        let paragraph = notist::expr::Expr::call("notist::paragraph", span)
            .with_children(vec![notist::expr::Expr::text(src.to_owned(), span)]);
        notist::FrontendOutput {
            forest: vec![paragraph],
            syntax: options
                .capture_syntax
                .then(|| Box::new(src.to_owned()) as _),
            ..Default::default()
        }
    }
}

#[test]
fn custom_frontend_extends_defaults_and_can_override_an_extension() {
    use notist::{Ctor, Pipeline, Value};

    let notist = Pipeline::default().with_frontend(PlainTextFrontend {
        extensions: vec!["txt", "md"],
    });
    for path in ["document.txt", "document.md"] {
        let document = notist
            .analyze(path, "# literal", &notist::Context::default())
            .unwrap();
        assert!(document.diagnostics().is_empty());
        assert_eq!(document.root().children[0].ctor, Ctor::Paragraph);
        assert_eq!(
            document.root().children[0].children[0].fields.get("text"),
            Some(&Value::Str("# literal".into()))
        );
    }
    let (analysis, inspection) = notist
        .inspect("document.md", "# literal", &notist::Context::default())
        .unwrap();
    assert!(analysis.diagnostics().is_empty());
    assert_eq!(
        inspection
            .syntax
            .as_deref()
            .unwrap()
            .as_any()
            .downcast_ref::<String>(),
        Some(&"# literal".to_owned()),
    );
    for (path, src) in [
        ("document.not", "= Heading"),
        ("document.markdown", "# Heading"),
    ] {
        let document = notist
            .analyze(path, src, &notist::Context::default())
            .unwrap();
        assert!(
            document
                .root()
                .find(|node| node.ctor == Ctor::Heading)
                .is_some()
        );
    }
}

#[test]
fn source_errors_return_diagnostics_and_a_recovery_tree() {
    use notist::{Ctor, Phase, Pipeline};

    let document = Pipeline::default()
        .analyze("document.not", "#missing[x]", &notist::Context::default())
        .unwrap();
    assert_eq!(document.diagnostics().len(), 1);
    assert_eq!(document.diagnostics()[0].phase, Phase::Type);
    assert!(
        document
            .root()
            .find(|node| node.ctor == Ctor::Custom("missing".into()))
            .is_some()
    );
    let (root, diagnostics) = document.into_parts();
    assert_eq!(root.ctor, Ctor::Doc);
    assert_eq!(diagnostics.len(), 1);
}

#[test]
fn inspection_distinguishes_reference_callees_and_bound_content_nodes() {
    let json: serde_json::Value = serde_json::from_str(&notist::cst_json::analyze_json(
        "@(status: \"draft\")#raw(text: \"hello\")",
    ))
    .unwrap();
    let fragment = &json["ir1"]["children"][0];
    assert_eq!(fragment["kind"], "Fragment");
    let call = &fragment["children"][0];
    assert_eq!(call["kind"], "Call");
    assert_eq!(call["children"][0]["kind"], "Callee");
    let reference = &call["children"][0]["children"][0];
    assert_eq!(reference["kind"], "Ref");
    assert_eq!(reference["label"], "raw");
    let argument = &call["children"][1];
    assert_eq!(argument["label"], "text");
    assert_eq!(argument["children"][0]["kind"], "Literal");
    assert_eq!(json["ir2"]["children"][0]["kind"], "Node");
    assert_eq!(json["diagnostics"].as_array().unwrap().len(), 0);
}

#[test]
fn custom_content_dialect_keeps_its_own_syntax_and_lowering() {
    let pipeline = notist::Pipeline::new().with_frontend(PlainTextFrontend {
        extensions: vec!["txt"],
    });
    let source = "[#unknown(\"中文\") plain] next";
    let (lowered, parsed) = pipeline
        .lower_content_literal("literal.txt", source)
        .unwrap();
    assert!(lowered.diagnostics.is_empty());
    assert_eq!(lowered.forest.len(), 1);
    assert_eq!(lowered.forest[0].callee_name(), Some("notist::text"));
    assert_eq!(
        lowered.forest[0]
            .named_arg("text")
            .and_then(notist::expr::Expr::literal_value),
        Some(&notist::Value::Str("#unknown(\"中文\") plain".into()))
    );
    assert!(parsed.contents[0].syntax.as_ref().as_any().is::<String>());
    assert!(
        pipeline
            .lower_content_literal("literal.md", source)
            .is_err()
    );
}
