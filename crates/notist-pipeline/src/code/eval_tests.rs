//! Compiler/evaluator integration regressions. Runtime-only tests live in
//! notist-eval; these tests deliberately exercise real checked programs.
use super::*;
use crate::{Context, Pipeline};
use notist_core::item::{Ctor, SourceLocation, Value};
use notist_eval::{evaluate, test_support::*};

#[test]
fn immutable_content_replays_deterministically_across_budgets_and_replacements() {
    use notist_core::{
        definitions::FunctionId,
        extensions::{Replace, Replacement},
    };
    fn decorate(item: &mut Item, source: notist_core::source::SourceId) {
        if let Some(value) = item.as_item_mut() {
            value
                .fields
                .insert("checkpoint_sentinel", Value::Str("original".into()));
        }
        item.attrs
            .insert("checkpoint_attr", Value::Str("kept".into()));
        if let Some(origin) = &mut item.origin {
            origin.expansion.push(SourceLocation {
                source,
                start: 2,
                end: 8,
            });
        }
        for child in item.children_mut().into_iter().flatten() {
            decorate(child, source);
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
            let owner = notist_core::source::Source::metadata("outer.notc").unwrap();
            let mut sources = analysis.sources().clone();
            sources.insert(owner.clone()).unwrap();
            decorate(&mut root, owner.id());
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
                    let actual = evaluate(&root, &analysis.program, &sources, &context, limits);
                    let before = root.clone();
                    let expected = evaluate(&root, &analysis.program, &sources, &context, limits);
                    assert_eq!(root, before, "execution must not mutate shared code");
                    assert_summaries(&root, &analysis.program, &sources, &context, limits);
                    assert_eq!(
                        actual, expected,
                        "{dialect}, {content_nodes}, constrained={constrained}, {source}"
                    );
                }
            }
        }
    }
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
            let owner = notist_core::source::Source::metadata("outer.notc").unwrap();
            let mut sources = analysis.sources().clone();
            sources.insert(owner.clone()).unwrap();
            let mut root = analysis.root().clone();
            let boundary = &mut root.children_mut().unwrap()[0].children_mut().unwrap()[0];
            boundary.attrs.insert("id", Value::Str("recovery".into()));
            boundary
                .origin
                .as_mut()
                .unwrap()
                .expansion
                .push(SourceLocation {
                    source: owner.id(),
                    start: 3,
                    end: 7,
                });
            let before = item_shell(boundary);
            let call = boundary.call_mut().expect("content call expression");
            match failure {
                0 => call.invalid = Some("deliberate failure".into()),
                1 => {
                    let mut arguments: Vec<_> = call
                        .arguments()
                        .map(|(name, value)| (name.into(), value.clone()))
                        .collect();
                    arguments[0].1.known = None;
                    arguments[0].1.kind = ExprKind::Failure("argument failed".into());
                    call.set_arguments(arguments);
                }
                _ => {
                    let mut definition = call.definition().clone();
                    definition.returns = if failure == 2 {
                        Type::BlockContent
                    } else {
                        Type::String
                    };
                    *call = call.clone().with_definition(definition);
                }
            }
            let evaluated = evaluate(
                &root,
                &analysis.program,
                &sources,
                &context,
                Limits::default(),
            );
            assert!(!evaluated.diagnostics.is_empty());
            let recovered = &evaluated.root.children()[0].children()[0];
            assert!(recovered.is_error());
            assert_eq!(recovered.attrs, before.attrs);
            assert_eq!(recovered.origin, before.origin);
            assert_eq!(recovered.span, before.span);
            assert_eq!(recovered.level, before.level);
            assert!(recovered.children().is_empty());
            assert!(recovered.as_expr().is_none());
        }
    }
}

#[test]
fn dissolved_code_results_retain_preexisting_call_expansion_locations() {
    let pipeline = Pipeline::default();
    let context = Context::default()
        .with_module(
            pipeline
                .compile_module(
                    "doc.not",
                    "demo",
                    "demo/lib.notc",
                    "fn view() -> InlineContent { text(text: \"result\") }",
                )
                .unwrap(),
        )
        .unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.view()", &context)
        .unwrap();
    let mut root = analysis.root().clone();
    let call = &mut root.children_mut().unwrap()[0].children_mut().unwrap()[0];
    let owner = notist_core::source::Source::metadata("outer.notc").unwrap();
    let mut sources = analysis.sources().clone();
    sources.insert(owner.clone()).unwrap();
    let outer = SourceLocation {
        source: owner.id(),
        start: 3,
        end: 8,
    };
    call.origin.as_mut().unwrap().expansion.push(outer.clone());
    let result = evaluate(
        &root,
        &analysis.program,
        &sources,
        &context,
        Limits::default(),
    );
    assert!(result.diagnostics.is_empty());
    let text = result
        .root
        .find(|n| n.field("text") == Some(&Value::Str("result".into())))
        .unwrap();
    let chain = &text.origin.as_ref().unwrap().expansion;
    assert!(chain.contains(&outer));
    assert_eq!(result.sources.name(chain[0].source).unwrap(), "doc.not");
    assert!(result.root.descendants().all(|n| n.as_expr().is_none()));
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
                    let recovery = Item::new(Ctor::Error("recovery".into()), Default::default())
                        .with_children(vec![Item::text("child".into(), Default::default())]);
                    root.children_mut().unwrap().push(recovery);
                }
                assert_summaries(
                    &root,
                    &analysis.program,
                    analysis.sources(),
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
    let call = root.children_mut().unwrap()[0].children_mut().unwrap()[0]
        .call_mut()
        .unwrap();
    let mut arguments: Vec<_> = call
        .arguments()
        .map(|(name, value)| (name.into(), value.clone()))
        .collect();
    let argument = &mut arguments[0].1;
    argument.known = Some(ComptimeValue::Data(Value::Str("after".into())));
    argument.kind = ExprKind::Constant(argument.known.clone().unwrap());
    call.set_arguments(arguments);
    // No prechecked instances: the compiler callback must install them on demand.
    let result = evaluate(
        &root,
        &CheckedProgram::default(),
        analysis.sources(),
        &context,
        Limits::default(),
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert!(
        result
            .root
            .descendants()
            .any(|i| i.field("text") == Some(&Value::Str("after".into())))
    );
    assert!(result.root.descendants().all(|i| i.as_expr().is_none()));
    assert_eq!(
        analysis.root().children()[0].children()[0].field("name"),
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
    let original = analysis.root().children()[0].children()[0].call().unwrap();
    let mut root = analysis.root().clone();
    let changed = root.children_mut().unwrap()[0].children_mut().unwrap()[0]
        .call_mut()
        .unwrap();
    assert!(std::ptr::eq(changed.definition(), original.definition()));
    let mut definition = changed.definition().clone();
    definition.parameters[0].constraint = notist_core::definitions::ValueConstraint::PositiveInt;
    *changed = changed.clone().with_definition(definition);
    assert!(!std::ptr::eq(changed.definition(), original.definition()));
    assert_eq!(
        original.definition().parameters[0].constraint,
        notist_core::definitions::ValueConstraint::Any
    );
    let changed = evaluate(
        &root,
        &analysis.program,
        analysis.sources(),
        &context,
        Limits::default(),
    );
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
            .any(|item| item.field("text") == Some(&Value::Str("-1".into())))
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
        .find(|r| r.definition().id.module == "demo")
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
    let expr = forest[0].as_expr().expect("interpolation expression");
    assert_eq!(expr.ty, Some(Type::InlineContent));
    assert!(
        matches!(&expr.kind, ExprKind::Input(id) if analysis.sources().bindings().get(*id).unwrap().name == "children")
    );
    let evaluation = pipeline.evaluate(&analysis, &context);
    assert!(evaluation.diagnostics.is_empty());
    assert!(evaluation.root.find(|i| i.ctor() == Ctor::Group).is_none());
    assert_eq!(
        evaluation
            .root
            .find(|i| i.ctor() == Ctor::Text)
            .unwrap()
            .field("text"),
        Some(&Value::Str("one".into()))
    );
    assert!(evaluation.root.descendants().all(|i| i.as_expr().is_none()));
}
