use notist_core::diag::Phase;
use notist_core::expr::RExpr;
use notist_core::expr::{BodyFlavor, CallArg, Expr};
use notist_core::frontend::{Frontend, FrontendOptions};
use notist_core::item::{Ctor, Dict, Value};
use notist_pipeline::{Analysis, Pipeline};
use rowan::TextRange;

fn span(start: u32, end: u32) -> TextRange {
    TextRange::new(start.into(), end.into())
}

fn analyze(forest: Vec<Expr>) -> (Analysis, Vec<RExpr>) {
    let len = forest
        .iter()
        .map(|e| usize::from(e.span().end()))
        .max()
        .unwrap_or(0);
    let (analysis, inspection) = Pipeline::default().analyze_lowered(
        std::path::Path::new("test.not"),
        " ".repeat(len).into(),
        notist_core::frontend::FrontendOutput {
            forest,
            ..Default::default()
        },
        &notist_pipeline::Context::default(),
        true,
    );
    (analysis, inspection.shaped)
}

#[test]
fn a_reference_to_a_constructor_does_not_invoke_it() {
    let (analysis, forest) = analyze(vec![
        Expr::reference("divider", span(0, 7)),
        Expr::call("divider", span(8, 18)),
    ]);
    let diagnostics = analysis.diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].phase, Phase::Type);
    assert_eq!(diagnostics[0].message, "unbound reference `divider`");
    assert_eq!(forest[0].ctor, Ctor::Interpolation);
    assert!(matches!(
        forest[0].pending.as_deref(),
        Some(notist_core::evaluation::PendingEvaluation::Splice(expr))
            if matches!(&expr.kind, notist_core::evaluation::ExprKind::Failure(message)
                if message == "unbound reference `divider`")
    ));
    assert_eq!(forest[1].ctor, Ctor::Divider);
}

#[test]
fn nonliteral_arguments_are_diagnosed_instead_of_dropped_silently() {
    let mut call = Expr::call("raw", span(0, 30))
        .with_named_arg("lang", Expr::reference("language", span(20, 28)));
    let Expr::Call { args, .. } = &mut call else {
        unreachable!()
    };
    args.insert(
        0,
        CallArg::Positional(Expr::reference("source", span(4, 10))),
    );
    let (analysis, _) = analyze(vec![call]);
    let diagnostics = analysis.diagnostics();
    for (name, range) in [("source", span(4, 10)), ("language", span(20, 28))] {
        assert!(
            diagnostics
                .iter()
                .any(|d| { d.span == range && d.message == format!("unbound reference `{name}`") }),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn duplicate_named_arguments_survive_lowering_and_are_diagnosed() {
    let src = "#raw(text: \"first\", text: \"last\")";
    for path in ["x.not", "x.md"] {
        let (analysis, inspection) = Pipeline::default()
            .inspect(path, src, &notist_pipeline::Context::default())
            .unwrap();
        fn find_raw(expr: &Expr) -> Option<&[CallArg]> {
            if let Expr::Fragment { children, .. } = expr {
                return children.iter().find_map(find_raw);
            }
            let Expr::Call { args, children, .. } = expr else {
                return None;
            };
            if expr.callee_path().as_deref() == Some("raw") {
                return Some(args);
            }
            children.iter().find_map(find_raw)
        }
        let args = inspection.lowered.iter().find_map(find_raw).unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(
            analysis.diagnostics().len(),
            1,
            "{path}: {:?}",
            analysis.diagnostics()
        );
        let diagnostic = &analysis.diagnostics()[0];
        assert_eq!(
            diagnostic.message,
            "argument `text` supplied more than once"
        );
        assert_eq!(
            &src[usize::from(diagnostic.span.start())..usize::from(diagnostic.span.end())],
            "\"last\""
        );
        let raw = analysis
            .root()
            .find(|node| node.ctor == Ctor::RawInline)
            .unwrap();
        assert_eq!(raw.fields.get("text"), Some(&Value::Str("last".into())));
    }
}

#[test]
fn annotations_attach_to_the_target_call_and_module_without_extra_nodes() {
    let src = "@!(id: \"doc\")\n@(id: \"range\")\nbefore @(status: \"outdate\")#raw(\"x\") after";
    for path in ["x.not", "x.md"] {
        let analysis = Pipeline::default()
            .analyze(path, src, &notist_pipeline::Context::default())
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{path}: {:?}",
            analysis.diagnostics()
        );
        let root = analysis.root();
        assert_eq!(root.attrs.get("id"), Some(&Value::Str("doc".into())));
        assert_eq!(root.children.len(), 1);
        let paragraph = &root.children[0];
        assert_eq!(paragraph.ctor, Ctor::Paragraph);
        assert_eq!(paragraph.attrs.get("id"), Some(&Value::Str("range".into())));
        assert_eq!(paragraph.children.len(), 3);
        let raw = &paragraph.children[1];
        assert_eq!(raw.ctor, Ctor::RawInline);
        assert_eq!(raw.attrs.get("status"), Some(&Value::Str("outdate".into())));
        assert!(raw.fields.get("status").is_none());
    }
    let mut reference = Expr::reference("content", span(0, 7));
    let mut attrs = Dict::default();
    attrs.insert("status", Value::Str("draft".into()));
    assert_eq!(reference.set_attrs(attrs.clone()), Err(attrs));
}

#[test]
fn unsupported_callee_expressions_recover_without_losing_content_or_metadata() {
    let mut attrs = Dict::default();
    attrs.insert("id", Value::Str("call".into()));
    let call = Expr::Call {
        comptime: Vec::new(),
        callee: Box::new(Expr::Literal(Value::Int(1), span(0, 1))),
        args: Vec::new(),
        children: vec![Expr::text("child".into(), span(2, 7))],
        body: BodyFlavor::Block,
        attrs: attrs.clone(),
        span: span(0, 8),
    };
    let (analysis, forest) = analyze(vec![call, Expr::call("divider", span(9, 19))]);
    let diagnostics = analysis.diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].span, span(0, 1));
    assert_eq!(forest[0].attrs, attrs);
    assert_eq!(forest[0].body, BodyFlavor::Block);
    assert_eq!(forest[0].children.len(), 1);
    assert_eq!(forest[1].ctor, Ctor::Divider);
    let root = analysis.root();
    assert_eq!(
        root.children[0].attrs.get("id"),
        Some(&Value::Str("call".into()))
    );
    assert_eq!(root.children[1].ctor, Ctor::Divider);
}

#[test]
fn markdown_prefix_remapping_covers_callees_named_arguments_and_nested_calls() {
    let src = "> #link(\"目标\", title: \"标题\")[#strong[内容]]";
    let output = notist_pipeline::frontend::markdown::MarkdownFrontend
        .compile(src, FrontendOptions::default());
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    fn check(expr: &Expr, src: &str, targets: &mut Vec<String>) {
        if let Expr::Fragment { children, .. } = expr {
            for child in children {
                check(child, src, targets);
            }
            return;
        }
        let slice = &src[usize::from(expr.span().start())..usize::from(expr.span().end())];
        if let Expr::Call {
            callee,
            args,
            children,
            ..
        } = expr
        {
            if matches!(expr.callee_path().as_deref(), Some("link" | "strong")) {
                let name =
                    &src[usize::from(callee.span().start())..usize::from(callee.span().end())];
                assert_eq!(Some(name), expr.callee_path().as_deref());
                targets.push(name.into());
                for arg in args {
                    let value = arg.value();
                    let slice =
                        &src[usize::from(value.span().start())..usize::from(value.span().end())];
                    assert!(matches!(slice, "\"目标\"" | "\"标题\""), "{slice}");
                }
            }
            assert!(!slice.is_empty());
            for child in children {
                check(child, src, targets);
            }
        }
    }
    let mut targets = Vec::new();
    for expr in &output.forest {
        check(expr, src, &mut targets);
    }
    assert_eq!(targets, ["link", "strong"]);
}
