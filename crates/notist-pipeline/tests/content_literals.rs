use notist_core::diag::Phase;
use notist_core::expr::BodyFlavor;
use notist_core::item::Ctor;
use notist_core::item::Value;
use notist_pipeline::Pipeline;
use notist_syntax::tree::SyntaxKind;

#[test]
fn host_selects_content_syntax_and_compiles_items_without_a_document_root() {
    let pipeline = Pipeline::default();
    for (path, source, target) in [
        ("literal.not", "[*native*] remaining code", Ctor::Strong),
        ("literal.md", "[**markdown**] remaining code", Ctor::Strong),
    ] {
        let (lowered, parsed) = pipeline
            .analyze_content_literal(path, source, &notist_pipeline::Context::default())
            .unwrap();
        assert!(
            lowered.diagnostics.is_empty(),
            "{path}: {:?}",
            lowered.diagnostics
        );
        assert_eq!(lowered.body, BodyFlavor::Inline);
        assert_eq!(lowered.forest.len(), 1);
        assert_eq!(lowered.forest[0].ctor, target);
        assert_eq!(parsed.syntax().kind(), SyntaxKind::ContentLiteral);
        assert!(parsed.syntax().children().next().is_none());
        assert_eq!(parsed.contents.len(), 1);
        let end = source.find("] remaining").unwrap() + 1;
        assert_eq!(usize::from(parsed.consumed()), end);
        let text = &lowered.forest[0].children[0];
        let value = text.fields.get("text").unwrap();
        let Value::Str(value) = value else { panic!() };
        assert_eq!(
            &source[usize::from(text.span.start())..usize::from(text.span.end())],
            value
        );
    }
}

#[test]
fn frontends_keep_their_own_trees_and_literal_delimiter_rules() {
    let pipeline = Pipeline::default();
    for (path, source) in [
        ("literal.not", "[`literal ]` #raw(\"more ]\")] next"),
        ("literal.md", "[`literal ]` #raw(\"more ]\")] next"),
        (
            "literal.md",
            "[ \n```text\n] still code\n```\n\n**bold**\n ] next",
        ),
    ] {
        let (lowered, parsed) = pipeline
            .analyze_content_literal(path, source, &notist_pipeline::Context::default())
            .unwrap();
        assert!(
            lowered.diagnostics.is_empty(),
            "{path}: {:?}",
            lowered.diagnostics
        );
        assert_eq!(
            usize::from(parsed.consumed()),
            source.find("] next").unwrap() + 1
        );
        let syntax = parsed.contents[0].syntax.as_ref().as_any();
        if path.ends_with(".not") {
            assert!(syntax.is::<notist_syntax::Parse>());
        } else {
            assert!(syntax.is::<notist_pipeline::frontend::markdown::MarkdownSyntax>());
        }
        assert!(!lowered.forest.is_empty());
    }
}

#[test]
fn block_literals_keep_blocks_and_annotations_without_a_literal_wrapper() {
    for (path, source) in [
        ("literal.not", "[ \n@(status: \"draft\")\n*strong*\n ]"),
        ("literal.md", "[ \n@(status: \"draft\")\n**strong**\n ]"),
    ] {
        let (lowered, _) = Pipeline::default()
            .analyze_content_literal(path, source, &notist_pipeline::Context::default())
            .unwrap();
        assert!(
            lowered.diagnostics.is_empty(),
            "{path}: {:?}",
            lowered.diagnostics
        );
        assert_eq!(lowered.body, BodyFlavor::Block);
        assert_eq!(lowered.forest.len(), 1);
        assert_eq!(lowered.forest[0].ctor, Ctor::Paragraph);
        assert_eq!(
            lowered.forest[0].attrs.get("status"),
            Some(&Value::Str("draft".into()))
        );
    }
}

#[test]
fn unclosed_literals_preserve_content_and_report_failure_once() {
    for path in ["literal.not", "literal.md"] {
        let (lowered, parsed) = Pipeline::default()
            .analyze_content_literal(path, "[中文", &notist_pipeline::Context::default())
            .unwrap();
        assert_eq!(lowered.forest.len(), 1);
        assert_eq!(
            lowered.diagnostics.len(),
            1,
            "{path}: {:?}",
            lowered.diagnostics
        );
        assert_eq!(lowered.diagnostics[0].phase, Phase::Syntax);
        assert_eq!(lowered.diagnostics[0].message, "unclosed content literal");
        assert_eq!(parsed.syntax().text().to_string(), "[中文");
    }
}

#[test]
fn content_literals_reject_module_annotations() {
    for path in ["literal.not", "literal.md"] {
        for source in ["[@!(:)#raw(\"x\")]", "[ \n@!(:)\ntext\n ]"] {
            let (lowered, _) = Pipeline::default()
                .analyze_content_literal(path, source, &notist_pipeline::Context::default())
                .unwrap();
            assert_eq!(
                lowered.diagnostics.len(),
                1,
                "{path}: {:?}",
                lowered.diagnostics
            );
            assert!(
                lowered.diagnostics[0]
                    .message
                    .starts_with("module annotation")
            );
            assert!(!lowered.forest.is_empty());
        }
    }
}

#[test]
fn both_content_dialects_recover_for_every_unicode_source_prefix() {
    for (path, source) in [
        ("literal.not", "[ \n@(id: \"中文\")\n#strong[`]`]\n ] more"),
        ("literal.md", "[ \n@(id: \"中文\")\n#strong[`]`]\n ] more"),
    ] {
        for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
            let prefix = &source[..end];
            let (lowered, parsed) = Pipeline::default()
                .analyze_content_literal(path, prefix, &notist_pipeline::Context::default())
                .unwrap();
            assert_eq!(
                parsed.syntax().text().to_string(),
                prefix[..usize::from(parsed.consumed())]
            );
            for diagnostic in &lowered.diagnostics {
                assert!(
                    usize::from(diagnostic.span.end()) <= end,
                    "{path} {prefix:?}: {diagnostic:?}"
                );
            }
        }
    }
}

#[derive(Default)]
struct ContentCounts {
    parsed: std::sync::atomic::AtomicUsize,
    lowered: std::sync::atomic::AtomicUsize,
}
struct TrackedContent(std::sync::Arc<ContentCounts>);
impl notist_syntax::content::ContentParser for TrackedContent {
    fn parse_content(
        &self,
        source: &str,
        open: rowan::TextSize,
    ) -> notist_syntax::content::ParsedContent {
        self.0
            .parsed
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        notist_syntax::content::ContentParser::parse_content(
            &notist_pipeline::frontend::markup::NotistFrontend,
            source,
            open,
        )
    }
}
impl notist_pipeline::ContentLowering for TrackedContent {
    fn lower_content(
        &self,
        content: &notist_syntax::content::ParsedContent,
        diagnostics: &mut Vec<notist_core::diag::Diagnostic>,
    ) -> notist_pipeline::Lowered {
        self.0
            .lowered
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        notist_pipeline::ContentLowering::lower_content(
            &notist_pipeline::frontend::markup::NotistFrontend,
            content,
            diagnostics,
        )
    }
}
impl notist_core::frontend::Frontend for TrackedContent {
    fn extensions(&self) -> &[&str] {
        &["not"]
    }
    fn compile(
        &self,
        source: &str,
        options: notist_core::frontend::FrontendOptions,
    ) -> notist_core::frontend::FrontendOutput {
        notist_core::frontend::Frontend::compile(
            &notist_pipeline::frontend::markup::NotistFrontend,
            source,
            options,
        )
    }
}

#[test]
fn code_lowers_markup_once_then_reuses_ir_across_concrete_instances() {
    use notist_pipeline::Context;
    use std::sync::{Arc, atomic::Ordering::SeqCst};
    let counts = Arc::new(ContentCounts::default());
    let pipeline = Pipeline::new().with_frontend(TrackedContent(counts.clone()));
    let module = pipeline
        .compile_module(
            "literal.not",
            "demo",
            "demo.notc",
            r#"
        fn choose<C: Type>[children: C] -> C {
            if C == InlineContent { [#children] } else { [ #children ] }
        }
    "#,
        )
        .unwrap();
    assert_eq!(counts.parsed.load(SeqCst), 2);
    assert_eq!(
        counts.lowered.load(SeqCst),
        2,
        "module registration must lower both branches to IR₁"
    );
    let context = Context::default().with_module(module).unwrap();
    assert!(pipeline.check(&context).is_empty());
    assert_eq!(
        counts.lowered.load(SeqCst),
        2,
        "checking generic declarations must not repeat frontend lowering"
    );
    drop(pipeline);
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze(
            "doc.not",
            "#self.choose[one] #self.choose[two]\n\n#self.choose[ #paragraph[block] ]",
            &context,
        )
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    assert_eq!(
        counts.parsed.load(SeqCst),
        2,
        "semantic checking must not reparse content"
    );
    assert_eq!(
        counts.lowered.load(SeqCst),
        2,
        "concrete instances must reuse the first-stage IR₁"
    );
    let evaluated = pipeline.evaluate(&analysis, &context);
    assert!(
        evaluated.diagnostics.is_empty(),
        "{:?}",
        evaluated.diagnostics
    );
    for text in ["one", "two", "block"] {
        assert!(
            evaluated
                .root
                .descendants()
                .any(|n| n.fields.get("text") == Some(&Value::Str(text.into())))
        );
    }
    assert_eq!(
        counts.lowered.load(SeqCst),
        2,
        "evaluation reuses checked instance bodies"
    );
}

#[test]
fn content_resolution_uses_the_selected_instance_and_keeps_source_ranges() {
    use notist_pipeline::Context;
    let pipeline = Pipeline::default();
    let source = "fn choose<b: Bool>() -> InlineContent { if b { [ok] } else { [#missing()] } }";
    let module = pipeline
        .compile_module("literal.not", "demo", "demo.notc", source)
        .unwrap();
    assert!(module.diagnostics().next().is_none());
    let context = Context::default().with_module(module).unwrap();
    let valid = pipeline
        .analyze("doc.not", "#self.choose<true>()", &context)
        .unwrap();
    assert!(valid.diagnostics().is_empty(), "{:?}", valid.diagnostics());
    let invalid = pipeline
        .analyze("doc.not", "#self.choose<false>()", &context)
        .unwrap();
    let errors: Vec<_> = invalid
        .diagnostics()
        .iter()
        .filter(|d| d.message.contains("missing"))
        .collect();
    assert_eq!(errors.len(), 1, "{:?}", invalid.diagnostics());
    assert_eq!(errors[0].source.as_deref(), Some("demo.notc"));
    assert_eq!(
        &source[usize::from(errors[0].span.start())..usize::from(errors[0].span.end())],
        "#missing()"
    );
    let result = pipeline.evaluate(&invalid, &context);
    let call = result
        .root
        .find(|n| matches!(&n.ctor, Ctor::Custom(name) if name == "missing"))
        .unwrap();
    assert!(matches!(
        call.evaluation,
        Some(notist_core::item::EvaluationState::Error(_))
    ));
}

#[test]
fn direct_code_mounts_attach_annotations_after_their_contents_are_shaped() {
    use notist_pipeline::Context;
    let pipeline = Pipeline::default();
    let module = pipeline
        .compile_module(
            "literal.not",
            "demo",
            "demo.notc",
            r#"
fn inline() -> BlockContent { paragraph()[@(id: "inline")#strong[text]] }
fn block() -> BlockContent { callout()[
@(id: "section")
= Title
] }
    "#,
        )
        .unwrap();
    let context = Context::default().with_module(module).unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.inline()\n\n#self.block()", &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let result = pipeline.evaluate(&analysis, &context);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    for (id, ctor) in [("inline", Ctor::Strong), ("section", Ctor::Section)] {
        let item = result
            .root
            .find(|n| n.attrs.get("id") == Some(&Value::Str(id.into())))
            .unwrap();
        assert_eq!(item.ctor, ctor);
    }
}

#[test]
fn first_stage_nested_markdown_lowering_restores_unicode_ranges_through_quote_prefixes() {
    use notist_pipeline::Context;
    let pipeline = Pipeline::default();
    let source = "fn label()[children: InlineContent] -> String { \"ok\" }\nfn show() -> BlockContent { [\n> #text(text: label()[@!(:)中文])\n] }";
    let module = pipeline
        .compile_module("literal.md", "demo", "demo.notc", source)
        .unwrap();
    let early: Vec<_> = module.diagnostics().collect();
    assert_eq!(early.len(), 1, "{early:?}");
    assert_eq!(
        &source[usize::from(early[0].span.start())..usize::from(early[0].span.end())],
        "@!(:)"
    );
    let context = Context::default().with_module(module).unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.show()", &context)
        .unwrap();
    let errors: Vec<_> = analysis
        .diagnostics()
        .iter()
        .filter(|d| d.message.starts_with("module annotation"))
        .collect();
    assert_eq!(errors.len(), 1, "{:?}", analysis.diagnostics());
    let span = errors[0].span;
    assert_eq!(errors[0].source.as_deref(), Some("demo.notc"));
    assert_eq!(
        &source[usize::from(span.start())..usize::from(span.end())],
        "@!(:)"
    );
}

#[test]
fn complete_content_entry_uses_code_expression_rules_and_the_supplied_module_scope() {
    use notist_pipeline::Context;
    let pipeline = Pipeline::default();
    let module = pipeline
        .compile_module(
            "content.not",
            "demo",
            "demo.notc",
            "fn value() -> String { \"ok\" }",
        )
        .unwrap();
    let context = Context::default().with_module(module).unwrap();
    let (analysis, _) = pipeline
        .analyze_content_literal(
            "content.not",
            "[#text(text: self.value())] trailing",
            &context,
        )
        .unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.forest.len(), 1);
    assert_eq!(analysis.forest[0].ctor, Ctor::Text);
    assert!(analysis.forest[0].pending.is_some());
}

#[test]
fn invalid_content_lowering_in_a_direct_mount_is_recorded_before_instance_checking() {
    use notist_pipeline::Context;
    let pipeline = Pipeline::default();
    let module = pipeline
        .compile_module(
            "literal.not",
            "demo",
            "demo.notc",
            "fn view() -> InlineContent { emph()[@!(:)bad] }",
        )
        .unwrap();
    let early: Vec<_> = module.diagnostics().collect();
    assert_eq!(early.len(), 1, "{early:?}");
    assert!(early[0].message.starts_with("module annotation"));
    let context = Context::default().with_module(module).unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.view()", &context)
        .unwrap();
    assert_eq!(
        analysis
            .diagnostics()
            .iter()
            .filter(|d| d.message.starts_with("module annotation"))
            .count(),
        1
    );
    let result = pipeline.evaluate(&analysis, &context);
    let boundary = result
        .root
        .find(|n| {
            n.function_id() == Some(notist_core::definitions::FunctionId::new("demo", "view"))
        })
        .unwrap();
    assert!(matches!(
        boundary.evaluation,
        Some(notist_core::item::EvaluationState::Error(_))
    ));
}

#[test]
fn first_stage_content_expression_contains_ir1_and_annotation_side_list() {
    use notist_core::expr::{CallArg, Expr};
    let frontend = notist_pipeline::frontend::markup::NotistFrontend;
    let parsed = notist_syntax::code::parse_content_literal(
        "[@(id: \"label\")#text(text: value)]",
        &frontend,
    );
    let mut diagnostics = Vec::new();
    let expr = notist_pipeline::lowering::code::expression(
        rowan::NodeOrToken::Node(parsed.syntax()),
        Some(&parsed),
        Some(&frontend),
        &mut diagnostics,
    )
    .unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let Expr::Content {
        forest,
        annotations,
        body,
        ..
    } = expr
    else {
        panic!("expected content IR₁")
    };
    assert_eq!(body, BodyFlavor::Inline);
    assert_eq!(annotations.len(), 1);
    let [Expr::Call { callee, args, .. }] = forest.as_slice() else {
        panic!("expected lowered call")
    };
    assert_eq!(callee.name_path().as_deref(), Some("text"));
    assert!(
        matches!(&args[0], CallArg::Named { name, value: Expr::Ref { name: binding, .. } } if name == "text" && binding == "value")
    );
}

#[test]
fn first_stage_reports_markup_lowering_errors_in_unbound_generic_branches() {
    let pipeline = Pipeline::default();
    let source = "fn choose<b: Bool>() -> InlineContent { if b { [ok] } else { [@!(:)bad] } }";
    let module = pipeline
        .compile_module("literal.not", "demo", "demo.notc", source)
        .unwrap();
    let early: Vec<_> = module.diagnostics().collect();
    assert_eq!(early.len(), 1, "{early:?}");
    assert!(early[0].message.starts_with("module annotation"));
    assert_eq!(early[0].source.as_deref(), Some("demo.notc"));
    assert_eq!(
        &source[usize::from(early[0].span.start())..usize::from(early[0].span.end())],
        "@!(:)"
    );
    let context = notist_pipeline::Context::default()
        .with_module(module)
        .unwrap();
    let diagnostics = pipeline.check_module("demo", &context);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].message.starts_with("module annotation"));
}
