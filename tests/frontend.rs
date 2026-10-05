#[test]
fn pipeline_dispatch_by_extension() {
    let pipeline = notist::Frontends::default();
    let (item, diags) = pipeline
        .analyze(std::path::Path::new("x.md"), "# 标题\n")
        .expect("md frontend");
    assert!(diags.is_empty());
    assert!(
        item.descendants()
            .any(|i| i.ctor == notist::item::Ctor::Heading)
    );
    assert!(
        pipeline
            .analyze(std::path::Path::new("x.not"), "正文\n")
            .is_some()
    );
    assert!(
        pipeline
            .analyze(std::path::Path::new("x.txt"), "正文\n")
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
    ] {
        let document = notist
            .analyze(path, src, notist::builtins::registry())
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
    use notist::{Frontend, Pipeline};

    assert!(
        Pipeline::new()
            .analyze("document.not", "正文", notist::builtins::registry())
            .is_err()
    );
    let notist = Pipeline::new().with_frontend(Frontend::notist());
    assert!(
        notist
            .analyze("document.not", "正文", notist::builtins::registry())
            .is_ok()
    );
    let error = notist
        .analyze("document.md", "正文", notist::builtins::registry())
        .unwrap_err();
    assert_eq!(error.path, std::path::Path::new("document.md"));
    assert!(
        notist
            .analyze("untitled", "正文", notist::builtins::registry())
            .is_err()
    );
}

fn lower_plain_text(
    src: &str,
) -> (
    Vec<notist::expr::Expr>,
    notist::Dict,
    Vec<notist::Diagnostic>,
) {
    let span = notist::TextRange::new(0.into(), (src.len() as u32).into());
    let paragraph = notist::expr::Expr::call("paragraph", span)
        .with_children(vec![notist::expr::Expr::text(src.to_owned(), span)]);
    (vec![paragraph], notist::Dict::default(), Vec::new())
}

#[test]
fn custom_frontend_extends_defaults_and_can_override_an_extension() {
    use notist::{Ctor, Frontend, Pipeline, Value};

    let notist = Pipeline::default().with_frontend(Frontend {
        extensions: &["txt", "md"],
        lower: lower_plain_text,
    });
    for path in ["document.txt", "document.md"] {
        let document = notist
            .analyze(path, "# literal", notist::builtins::registry())
            .unwrap();
        assert!(document.diagnostics().is_empty());
        assert_eq!(document.root().children[0].ctor, Ctor::Paragraph);
        assert_eq!(
            document.root().children[0].children[0].fields.get("text"),
            Some(&Value::Str("# literal".into()))
        );
    }
    for (path, src) in [
        ("document.not", "= Heading"),
        ("document.markdown", "# Heading"),
    ] {
        let document = notist
            .analyze(path, src, notist::builtins::registry())
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
        .analyze("document.not", "#missing[x]", notist::builtins::registry())
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
