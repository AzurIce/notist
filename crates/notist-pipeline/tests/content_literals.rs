use notist_core::diag::Phase;
use notist_core::expr::{BodyFlavor, Expr};
use notist_core::item::Value;
use notist_pipeline::Pipeline;
use notist_syntax::tree::SyntaxKind;

#[test]
fn host_selects_content_syntax_and_lowering_without_a_document_root() {
    let pipeline = Pipeline::default();
    for (path, source, target) in [
        ("literal.not", "[*native*] remaining code", "notist::strong"),
        (
            "literal.md",
            "[**markdown**] remaining code",
            "notist::strong",
        ),
    ] {
        let (lowered, parsed) = pipeline.lower_content_literal(path, source).unwrap();
        assert!(
            lowered.diagnostics.is_empty(),
            "{path}: {:?}",
            lowered.diagnostics
        );
        assert_eq!(lowered.body, BodyFlavor::Inline);
        assert_eq!(lowered.forest.len(), 1);
        assert_eq!(lowered.forest[0].callee_name(), Some(target));
        assert_eq!(parsed.syntax().kind(), SyntaxKind::ContentLiteral);
        assert!(parsed.syntax().children().next().is_none());
        assert_eq!(parsed.contents.len(), 1);
        let end = source.find("] remaining").unwrap() + 1;
        assert_eq!(usize::from(parsed.consumed()), end);
        let Expr::Call { children, .. } = &lowered.forest[0] else {
            panic!()
        };
        let text = &children[0];
        let value = text
            .named_arg("text")
            .and_then(Expr::literal_value)
            .unwrap();
        let Value::Str(value) = value else { panic!() };
        assert_eq!(
            &source[usize::from(text.span().start())..usize::from(text.span().end())],
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
        let (lowered, parsed) = pipeline.lower_content_literal(path, source).unwrap();
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
            .lower_content_literal(path, source)
            .unwrap();
        assert!(
            lowered.diagnostics.is_empty(),
            "{path}: {:?}",
            lowered.diagnostics
        );
        assert_eq!(lowered.body, BodyFlavor::Block);
        assert_eq!(lowered.forest.len(), 1);
        assert!(matches!(lowered.forest[0], Expr::Fragment { .. }));
        assert_eq!(lowered.annotations.len(), 1);
        assert_eq!(
            lowered.annotations[0].kind,
            notist_core::annotation::AnnotationKind::Block
        );
        assert_eq!(
            lowered.annotations[0].dict.get("status"),
            Some(&Value::Str("draft".into()))
        );
    }
}

#[test]
fn unclosed_literals_preserve_content_and_report_failure_once() {
    for path in ["literal.not", "literal.md"] {
        let (lowered, parsed) = Pipeline::default()
            .lower_content_literal(path, "[中文")
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
                .lower_content_literal(path, source)
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
                .lower_content_literal(path, prefix)
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
