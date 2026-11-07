//! Compiler/evaluator integration regressions. Runtime-only tests live in
//! notist-eval; these tests deliberately exercise real checked programs.
use super::*;
use crate::{Context, Pipeline};
use notist_core::item::{Ctor, EvaluationState, SourceLocation, Value};
use notist_eval::{evaluate, test_support::*};

#[test]
fn present_checkpoints_match_full_backups_across_budgets_and_replacements() {
    use notist_core::{
        definitions::FunctionId,
        extensions::{Replace, Replacement},
    };
    fn decorate(item: &mut Item) {
        item.fields
            .insert("checkpoint_sentinel", Value::Str("original".into()));
        item.attrs
            .insert("checkpoint_attr", Value::Str("kept".into()));
        if let Some(origin) = &mut item.origin {
            origin.expansion.push(SourceLocation {
                source: "outer.notc".into(),
                start: 2,
                end: 8,
            });
        }
        if matches!(item.pending.as_deref(), Some(PendingEvaluation::Call(_)))
            || item.ctor == Ctor::Strong
        {
            // Recovery must preserve a target even when the host omitted instance.
            item.instance = None;
            item.target = Some(
                FunctionInstance {
                    function: FunctionId::new("host", "original"),
                    arguments: vec![],
                }
                .into(),
            );
        }
        for child in &mut item.children {
            decorate(child);
        }
    }
    let fixtures = [
        (
            "fn leaf() -> InlineContent { [#strong[A]#emph[B]#text(text: \"C\")] }",
            "#self.leaf()#self.leaf()#self.leaf()",
            false,
        ),
        (
            "fn pass()[children: InlineContent] -> InlineContent { [#children#children] }",
            "#self.pass[one]#self.pass[two]",
            false,
        ),
        (
            "fn looped() -> InlineContent { for n in (1,2,3) [#text(text: str(n))#emph[X]] }",
            "#self.looped()#self.looped()",
            false,
        ),
        (
            "fn leaf() -> InlineContent { [#text(text: panic(\"inner failure\")) kept] }",
            "#text(text: \"prefix\")#self.leaf()#self.leaf()",
            false,
        ),
        (
            "fn view() -> InlineContent { let v = [#strong[A]#emph[B]]; [#v#v] }",
            "#self.view()#self.view()",
            false,
        ),
        (
            "fn rows() -> BlockContent { [\n#table()[#row[#cell[A]#cell[B]]#row[#cell[C]]]\n] }",
            "#self.rows()\n\n#self.rows()",
            false,
        ),
        (
            "fn strong()[children: InlineContent] -> InlineContent { [#emph[#children]#emph[X]] }",
            "**a** **b** **c**",
            true,
        ),
    ];
    let pipeline = Pipeline::default();
    reset_checkpoint_restores();
    for dialect in ["doc.not", "doc.md"] {
        for (source, document, replacement) in fixtures {
            let mut context = Context::default()
                .with_module(
                    pipeline
                        .compile_module(dialect, "demo", "demo/lib.notc", source)
                        .unwrap(),
                )
                .unwrap();
            if replacement {
                context = context
                    .with_extension(Replace::new([Replacement {
                        from: FunctionId::new("notist", "strong"),
                        to: FunctionId::new("demo", "strong"),
                        span: Default::default(),
                    }]))
                    .unwrap();
            }
            let analysis = pipeline.analyze(dialect, document, &context).unwrap();
            assert!(
                analysis.diagnostics().is_empty(),
                "{:?}: {source}",
                analysis.diagnostics()
            );
            let mut root = analysis.root().clone();
            decorate(&mut root);
            if replacement {
                fn make_synthetic(item: &mut Item) {
                    if item.ctor == Ctor::Strong {
                        item.pending = None;
                    }
                    for child in &mut item.children {
                        make_synthetic(child);
                    }
                }
                make_synthetic(&mut root);
            }
            for content_nodes in 0..64 {
                for constrained in [false, true] {
                    let limits = Limits {
                        content_nodes,
                        iterations: if constrained {
                            content_nodes % 5
                        } else {
                            100_000
                        },
                        call_depth: if constrained { content_nodes % 4 } else { 32 },
                        steps: if constrained {
                            content_nodes % 8
                        } else {
                            100_000
                        },
                        ..Limits::default()
                    };
                    let actual = evaluate(&root, &analysis.program, &context, limits);
                    let expected =
                        reference_present(|| evaluate(&root, &analysis.program, &context, limits));
                    assert_eq!(
                        actual, expected,
                        "{dialect}, {content_nodes}, constrained={constrained}, {source}"
                    );
                }
            }
        }
    }
    assert!(
        checkpoint_restores() > 0,
        "must exercise rollback after a successful call"
    );
}

#[test]
fn failed_calls_retain_the_owned_boundary_until_execution_commits() {
    let pipeline = Pipeline::default();
    let context = Context::default().with_module(pipeline.compile_module("doc.not", "demo", "demo/lib.notc", "fn wrap(name: String)[children: InlineContent] -> InlineContent { [#text(text: name)#children] }").unwrap()).unwrap();
    for dialect in ["doc.not", "doc.md"] {
        let analysis = pipeline
            .analyze(dialect, "#self.wrap(\"input\")[#emph[child]]", &context)
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        for failure in 0..4 {
            let mut root = analysis.root().clone();
            let boundary = &mut root.children[0].children[0];
            boundary
                .fields
                .insert("sentinel", Value::Str("original".into()));
            boundary.attrs.insert("id", Value::Str("recovery".into()));
            boundary
                .origin
                .as_mut()
                .unwrap()
                .expansion
                .push(SourceLocation {
                    source: "outer.notc".into(),
                    start: 3,
                    end: 7,
                });
            let before = item_shell(boundary);
            let Some(PendingEvaluation::Call(call)) = boundary.pending.as_deref_mut() else {
                panic!("expected a content call")
            };
            match failure {
                0 => call.invalid = Some("deliberate failure".into()),
                1 => {
                    call.arguments[0].1.known = None;
                    call.arguments[0].1.kind = ExprKind::Failure("argument failed".into());
                }
                2 => call.definition.returns = Type::BlockContent,
                _ => call.definition.returns = Type::String,
            }
            let evaluated = evaluate(&root, &analysis.program, &context, Limits::default());
            assert!(!evaluated.diagnostics.is_empty());
            let recovered = &evaluated.root.children[0].children[0];
            assert!(matches!(
                recovered.evaluation,
                Some(EvaluationState::Error(_))
            ));
            assert_eq!(recovered.ctor, before.ctor);
            assert_eq!(recovered.instance, before.instance);
            assert_eq!(recovered.fields, before.fields);
            assert_eq!(recovered.attrs, before.attrs);
            assert_eq!(recovered.origin, before.origin);
            assert_eq!(recovered.span, before.span);
            assert_eq!(recovered.level, before.level);
            assert!(recovered.children.is_empty());
            assert!(recovered.pending.is_none());
        }
    }
}

#[test]
fn returned_summaries_match_trees_across_mounts_failures_and_limits() {
    let pipeline = Pipeline::default();
    for dialect in ["doc.not", "doc.md"] {
        for source in [
            "fn view() -> InlineContent { let empty = []; let value = [#strong[A]#emph[B]]; [#value#empty#value] }",
            "fn pass()[children: InlineContent] -> InlineContent { [#children#children] } fn view() -> InlineContent { [#pass[ok]] }",
            "fn view() -> InlineContent { for n in (1,2,3) [#text(text: str(n))] }",
            "fn leaf(n: Int) -> InlineContent { [#text(text: if n == 2 { panic(\"bad\") } else { str(n) })] } fn view() -> InlineContent { for n in (1,2,3) [#leaf(n)] }",
            "fn view() -> BlockContent { [\n#table()[#row[#cell[A]#cell[B]]#row[#cell[C]]]\n] }",
        ] {
            let context = Context::default()
                .with_module(
                    pipeline
                        .compile_module(dialect, "demo", "demo/lib.notc", source)
                        .unwrap(),
                )
                .unwrap();
            let analysis = pipeline.analyze(dialect, "#self.view()", &context).unwrap();
            assert!(
                analysis.diagnostics().is_empty(),
                "{:?}",
                analysis.diagnostics()
            );
            let before = analysis.clone();
            for limit in 0..64 {
                let mut root = analysis.root().clone();
                if limit % 7 == 0 {
                    // Binding must count even the children of supplied recovery nodes.
                    let mut recovery = Item::text("discarded".into(), Default::default());
                    recovery.evaluation = Some(EvaluationState::Error("recovery".into()));
                    recovery
                        .children
                        .push(Item::text("child".into(), Default::default()));
                    root.children.push(recovery);
                }
                assert_summaries(
                    &root,
                    &analysis.program,
                    &context,
                    Limits {
                        content_nodes: limit,
                        iterations: limit % 5,
                        call_depth: limit % 4,
                        ..Limits::default()
                    },
                );
            }
            assert_eq!(analysis, before);
        }
    }
}

#[test]
fn evaluation_reads_the_supplied_item_tree_without_a_parallel_representation() {
    let context = Context::default()
        .with_module(
            Pipeline::default()
                .compile_module(
                    "content.not",
                    "demo",
                    "demo/lib.notc",
                    "fn greet(name: String) -> InlineContent { [#text(text: name)] }",
                )
                .unwrap(),
        )
        .unwrap();
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("doc.not", "#self.greet(\"before\")", &context)
        .unwrap();
    let mut root = analysis.root().clone();
    let call = root.children[0].children[0].pending.as_deref_mut().unwrap();
    let PendingEvaluation::Call(call) = call else {
        panic!("expected a pending call")
    };
    let argument = &mut call.arguments[0].1;
    argument.known = Some(ComptimeValue::Data(Value::Str("after".into())));
    argument.kind = ExprKind::Constant(argument.known.clone().unwrap());
    // No prechecked instances: the compiler callback must install them on demand.
    let result = evaluate(
        &root,
        &CheckedProgram::default(),
        &context,
        Limits::default(),
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert!(
        result
            .root
            .descendants()
            .any(|i| i.fields.get("text") == Some(&Value::Str("after".into())))
    );
    assert!(result.root.descendants().all(|i| i.pending.is_none()));
    assert_eq!(
        analysis.root().children[0].children[0].fields.get("name"),
        Some(&Value::Str("before".into()))
    );
}

#[test]
fn editing_a_captured_signature_detaches_and_remains_authoritative() {
    let pipeline = Pipeline::default();
    let context = Context::default()
        .with_module(
            pipeline
                .compile_module(
                    "content.not",
                    "demo",
                    "demo/lib.notc",
                    "fn show(value: Int) -> InlineContent { [#text(text: str(value))] }",
                )
                .unwrap(),
        )
        .unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.show(-1)", &context)
        .unwrap();
    assert!(analysis.diagnostics().is_empty());
    let original = analysis.root().children[0].children[0]
        .pending
        .as_deref()
        .unwrap();
    let PendingEvaluation::Call(original) = original else {
        panic!("expected a call")
    };
    let mut root = analysis.root().clone();
    let changed = root.children[0].children[0].pending.as_deref_mut().unwrap();
    let PendingEvaluation::Call(changed) = changed else {
        panic!("expected a call")
    };
    assert!(changed.definition.shares_storage(&original.definition));
    changed.definition.parameters[0].constraint =
        notist_core::definitions::ValueConstraint::PositiveInt;
    assert!(!changed.definition.shares_storage(&original.definition));
    assert_eq!(
        original.definition.parameters[0].constraint,
        notist_core::definitions::ValueConstraint::Any
    );
    let changed = evaluate(&root, &analysis.program, &context, Limits::default());
    assert!(
        changed
            .diagnostics
            .iter()
            .any(|d| d.message.contains("invalid value"))
    );
    let original = pipeline.evaluate(&analysis, &context);
    assert!(original.diagnostics.is_empty());
    assert!(
        original
            .root
            .descendants()
            .any(|item| item.fields.get("text") == Some(&Value::Str("-1".into())))
    );
}

#[test]
fn checked_function_bodies_contain_typed_item_interpolations() {
    let context = Context::default()
        .with_module(
            Pipeline::default()
                .compile_module(
                    "content.not",
                    "demo",
                    "demo/lib.notc",
                    "fn pass()[children: InlineContent] -> InlineContent { [#children] }",
                )
                .unwrap(),
        )
        .unwrap();
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("doc.not", "#self.pass[one]", &context)
        .unwrap();
    let body = analysis
        .program
        .instances
        .values()
        .find(|r| r.definition.id.module == "demo")
        .unwrap()
        .body
        .as_ref()
        .unwrap();
    let ExprKind::Block(_, value) = &body.kind else {
        panic!("expected function body")
    };
    let ExprKind::Content(forest) = &value.kind else {
        panic!("expected Item forest")
    };
    assert_eq!(forest[0].ctor, Ctor::Interpolation);
    let Some(PendingEvaluation::Splice(expr)) = forest[0].pending.as_deref() else {
        panic!("expected interpolation")
    };
    assert_eq!(expr.ty, Some(Type::InlineContent));
    assert!(matches!(&expr.kind, ExprKind::Input(name) if name == "children"));
    let evaluation = pipeline.evaluate(&analysis, &context);
    assert!(evaluation.diagnostics.is_empty());
    let pass = evaluation
        .root
        .find(|i| {
            i.function_id() == Some(notist_core::definitions::FunctionId::new("demo", "pass"))
        })
        .unwrap();
    assert_eq!(pass.children.len(), 1);
    assert!(pass.children[0].is_evaluated_interpolation());
    assert_eq!(
        pass.children[0].children[0].fields.get("text"),
        Some(&Value::Str("one".into()))
    );
    assert!(evaluation.root.descendants().all(|i| i.pending.is_none()));
}
