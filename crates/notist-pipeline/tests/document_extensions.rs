use notist_core::{
    diag::{Diagnostic, Phase},
    expr::Expr,
    extensions::{DocumentContext, DocumentExtension, ExtensionResult, LoweredDocument},
    item::{Ctor, Item, Value},
};
use notist_pipeline::{Context, Limits, Pipeline};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct Edit {
    label: &'static str,
    events: Arc<Mutex<Vec<String>>>,
    fail: Option<&'static str>,
}
impl Edit {
    fn event(&self, stage: &str) -> ExtensionResult {
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:{stage}", self.label));
        if self.fail == Some(stage) {
            Err(vec![Diagnostic::new(
                Phase::Semantic,
                Default::default(),
                "cannot edit",
            )])
        } else {
            Ok(())
        }
    }
}
impl DocumentExtension for Edit {
    fn name(&self) -> &str {
        self.label
    }
    fn before_check(
        &self,
        context: &DocumentContext<'_>,
        document: &mut LoweredDocument,
    ) -> ExtensionResult {
        assert!(context.sources.contains_key(context.path));
        assert!(context.registry.function("notist", "paragraph").is_some());
        document.attrs.insert(self.label, Value::Bool(true));
        document.forest.push(Expr::fragment(
            vec![Expr::text(self.label.into(), Default::default())],
            Default::default(),
        ));
        self.event("before_check")
    }
    fn after_check(&self, _: &DocumentContext<'_>, document: &Item) -> ExtensionResult {
        assert_eq!(document.attrs.get(self.label), Some(&Value::Bool(true)));
        self.event("after_check")
    }
    fn after_evaluate(&self, _: &DocumentContext<'_>, document: &mut Item) -> ExtensionResult {
        assert!(document.descendants().all(|i| i.pending.is_none()));
        document.children.push(
            Item::new(Ctor::Paragraph, Default::default()).with_children(vec![Item::text(
                format!("{}!", self.label),
                Default::default(),
            )]),
        );
        self.event("after_evaluate")
    }
}
fn context(fail: Option<&'static str>) -> (Context, Arc<Mutex<Vec<String>>>) {
    let events = Arc::new(Mutex::new(vec![]));
    let mut context = Context::default();
    for label in ["a", "b", "c"] {
        context = context
            .with_extension(Edit {
                label,
                events: events.clone(),
                fail: if label == "b" { fail } else { None },
            })
            .unwrap();
    }
    (context, events)
}
fn text(root: &Item) -> String {
    root.descendants()
        .filter_map(|node| match node.fields.get("text") {
            Some(Value::Str(value)) => Some(value.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn ordered_stages_edit_content_in_both_frontends() {
    let pipeline = Pipeline::default();
    for path in ["doc.not", "doc.md"] {
        let (context, events) = context(None);
        let (analysis, inspection) = pipeline.inspect(path, "original", &context).unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        assert_eq!(inspection.lowered.len(), 4);
        assert_eq!(text(analysis.root()), "originalabc");
        let evaluated = pipeline.evaluate(&analysis, &context);
        assert!(
            evaluated.diagnostics.is_empty(),
            "{:?}",
            evaluated.diagnostics
        );
        assert_eq!(text(&evaluated.root), "originalabca!b!c!");
        assert_eq!(text(analysis.root()), "originalabc");
        assert_eq!(
            *events.lock().unwrap(),
            [
                "a:before_check",
                "b:before_check",
                "c:before_check",
                "a:after_check",
                "b:after_check",
                "c:after_check",
                "a:after_evaluate",
                "b:after_evaluate",
                "c:after_evaluate"
            ]
        );
        events.lock().unwrap().clear();
        let ordinary = pipeline.analyze(path, "original", &context).unwrap();
        assert_eq!(analysis, ordinary);
    }
}

#[test]
fn failures_roll_back_only_the_current_hook_and_stop_later_hooks() {
    let pipeline = Pipeline::default();
    for stage in ["before_check", "after_check", "after_evaluate"] {
        let (context, events) = context(Some(stage));
        let analysis = pipeline.analyze("doc.not", "original", &context).unwrap();
        let evaluation = pipeline.evaluate(&analysis, &context);
        let diagnostics: Vec<_> = analysis
            .diagnostics()
            .iter()
            .chain(&evaluation.diagnostics)
            .collect();
        assert_eq!(diagnostics.len(), 1);
        let error = diagnostics[0];
        assert!(error.message.contains(&format!("extension `b` ({stage})")));
        assert_eq!(error.source.as_deref(), Some("doc.not"));
        assert!(!error.recovered);
        let events = events.lock().unwrap();
        assert_eq!(events.last().unwrap(), &format!("b:{stage}"));
        if stage == "before_check" {
            assert_eq!(text(analysis.root()), "originala");
            assert_eq!(analysis.root().attrs.get("b"), None);
        } else {
            assert_eq!(text(analysis.root()), "originalabc");
        }
        assert_eq!(
            text(&evaluation.root),
            if stage == "after_evaluate" {
                "originalabca!"
            } else if stage == "before_check" {
                "originala"
            } else {
                "originalabc"
            }
        );
    }
}

#[derive(Debug)]
struct Generate(Expr);
impl DocumentExtension for Generate {
    fn name(&self) -> &str {
        "generate"
    }
    fn before_check(
        &self,
        _: &DocumentContext<'_>,
        document: &mut LoweredDocument,
    ) -> ExtensionResult {
        document.forest = vec![self.0.clone()];
        Ok(())
    }
}

#[test]
fn generated_expressions_replace_source_and_pass_through_normal_checking() {
    let pipeline = Pipeline::default();
    for (expression, expected) in [
        (
            Expr::call("notist.heading", Default::default())
                .with_comptime("level", Expr::Literal(Value::Int(2), Default::default()))
                .with_children(vec![Expr::text("generated".into(), Default::default())]),
            None,
        ),
        (
            Expr::call("missing", Default::default()),
            Some("unknown constructor or function"),
        ),
        (
            Expr::call("notist.text", Default::default())
                .with_literal_arg("text", Value::Bool(true)),
            Some("must be string"),
        ),
    ] {
        let context = Context::default()
            .with_extension(Generate(expression))
            .unwrap();
        let analysis = pipeline.analyze("doc.not", "removed", &context).unwrap();
        if let Some(message) = expected {
            assert!(
                analysis
                    .diagnostics()
                    .iter()
                    .any(|d| d.message.contains(message)),
                "{:?}",
                analysis.diagnostics()
            );
        } else {
            assert!(
                analysis.diagnostics().is_empty(),
                "{:?}",
                analysis.diagnostics()
            );
            assert_eq!(text(analysis.root()), "generated");
            assert_eq!(analysis.root().children[0].ctor, Ctor::Section);
        }
    }
}

#[test]
fn invalid_generated_ranges_are_rejected_before_checking_and_keep_the_source_tree() {
    let pipeline = Pipeline::default();
    for span in [
        rowan::TextRange::empty(1.into()),
        rowan::TextRange::empty(99.into()),
    ] {
        let context = Context::default()
            .with_extension(Generate(Expr::text("bad".into(), span)))
            .unwrap();
        let analysis = pipeline.analyze("doc.not", "中", &context).unwrap();
        assert_eq!(text(analysis.root()), "中");
        assert_eq!(analysis.diagnostics().len(), 1);
        assert!(
            analysis.diagnostics()[0]
                .message
                .contains("UTF-8 boundaries")
        );
    }
}

#[derive(Debug)]
struct InvalidOutput(&'static str);
impl DocumentExtension for InvalidOutput {
    fn name(&self) -> &str {
        "invalid-output"
    }
    fn after_evaluate(&self, _: &DocumentContext<'_>, root: &mut Item) -> ExtensionResult {
        match self.0 {
            "root" => root.ctor = Ctor::Text,
            "nested" => root.children.push(Item::new(Ctor::Doc, Default::default())),
            "pending" => root
                .children
                .push(Item::new(Ctor::Interpolation, Default::default())),
            "limit" => root
                .children
                .extend((0..10).map(|_| Item::new(Ctor::Divider, Default::default()))),
            "ids" => {
                let node = Item::new(Ctor::Divider, Default::default());
                let mut first = node.clone();
                first.attrs.insert("id", Value::Str("duplicate".into()));
                root.children.extend([first.clone(), first]);
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
#[test]
fn invalid_output_is_rejected_without_losing_evaluated_content() {
    let pipeline = Pipeline::default();
    for kind in ["root", "nested", "pending", "limit", "ids"] {
        let context = Context::default()
            .with_extension(InvalidOutput(kind))
            .unwrap();
        let analysis = pipeline.analyze("doc.not", "kept", &context).unwrap();
        let evaluation = pipeline.evaluate_with_limits(
            &analysis,
            &context,
            Limits {
                content_nodes: 5,
                ..Limits::default()
            },
        );
        assert_eq!(
            evaluation.diagnostics.len(),
            1,
            "{kind}: {:?}",
            evaluation.diagnostics
        );
        assert!(!evaluation.diagnostics[0].recovered);
        assert_eq!(text(&evaluation.root), "kept");
        assert_eq!(evaluation.root.children.len(), 1);
        assert_eq!(evaluation.root.ctor, Ctor::Doc);
    }
}

#[derive(Debug)]
struct InvalidConfiguration;
impl DocumentExtension for InvalidConfiguration {
    fn name(&self) -> &str {
        "invalid-config"
    }
    fn validate(&self, _: &notist_core::registry::Registry) -> ExtensionResult {
        Err(vec![])
    }
}
#[test]
fn registration_failure_always_contains_a_diagnostic() {
    let errors = Context::default()
        .with_extension(InvalidConfiguration)
        .unwrap_err();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("invalid-config"));
}
