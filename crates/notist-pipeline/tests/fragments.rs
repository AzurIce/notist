use notist_core::{
    evaluation::PendingEvaluation,
    expr::Expr,
    item::{Ctor, EvaluationState, Value},
};
use notist_pipeline::{Context, Pipeline};

#[test]
fn lowering_records_fragments_without_paragraph_invocations() {
    let pipeline = Pipeline::default();
    for path in ["doc.not", "doc.md"] {
        let (_, inspection) = pipeline
            .inspect(
                path,
                "#emph[a]\n#strong[b]\n\n#divider()",
                &Context::default(),
            )
            .unwrap();
        assert_eq!(
            inspection.lowered.len(),
            2,
            "{path}: {:?}",
            inspection.lowered
        );
        assert!(
            matches!(&inspection.lowered[0], Expr::Fragment { children, .. } if children.len() == 2)
        );
        assert!(
            matches!(&inspection.lowered[1], Expr::Fragment { children, .. } if children.len() == 1)
        );
    }
}

#[test]
fn soft_newlines_keep_inline_calls_in_one_paragraph() {
    for path in ["doc.not", "doc.md"] {
        let analysis = Pipeline::default()
            .analyze(path, "#emph[a]\n#strong[b]", &Context::default())
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{path}: {:?}",
            analysis.diagnostics()
        );
        let paragraphs = &analysis.root().children;
        assert_eq!(paragraphs.len(), 1, "{path}");
        assert_eq!(paragraphs[0].ctor, Ctor::Paragraph);
        assert_eq!(paragraphs[0].children.len(), 2);
    }
}

#[test]
fn separate_blocks_require_a_hard_separator() {
    let pipeline = Pipeline::default();
    let context = Context::default();
    for path in ["doc.not", "doc.md"] {
        for source in [
            "#divider()\n#divider()",
            "before #divider() after",
            "#emph[a]\n#divider()",
            "#divider()\nafter",
            "\n\n#callout[\nbody\n] after",
        ] {
            let analysis = pipeline.analyze(path, source, &context).unwrap();
            assert!(
                analysis
                    .diagnostics()
                    .iter()
                    .any(|d| d.message.contains("separate blocks with a blank line")),
                "{path} {source:?}: {:?}",
                analysis.diagnostics()
            );
            // Recovery keeps the source fragment together instead of silently
            // exposing block children as valid siblings.
            assert_eq!(analysis.root().children.len(), 1, "{path} {source:?}");
        }
        for separator in ["\n\n", "\r\n \t\r\n", "\\\n"] {
            let source = format!("#divider(){separator}#divider()");
            let analysis = pipeline.analyze(path, &source, &context).unwrap();
            assert!(
                analysis.diagnostics().is_empty(),
                "{path} {source:?}: {:?}; lowered={:?}",
                analysis.diagnostics(),
                pipeline.inspect(path, &source, &context).unwrap().1.lowered
            );
            assert_eq!(analysis.root().children.len(), 2, "{path} {source:?}");
            assert!(
                analysis
                    .root()
                    .children
                    .iter()
                    .all(|item| item.ctor == Ctor::Divider)
            );
        }
    }
}

#[test]
fn invalid_fragment_recovers_without_discarding_the_next_fragment() {
    let pipeline = Pipeline::default();
    let context = Context::default();
    for path in ["doc.not", "doc.md"] {
        let analysis = pipeline
            .analyze(
                path,
                "before #divider() after\n\n#emph[still here]",
                &context,
            )
            .unwrap();
        let evaluation = pipeline.evaluate(&analysis, &context);
        assert!(matches!(
            evaluation.root.children[0].evaluation,
            Some(EvaluationState::Error(_))
        ));
        assert!(
            evaluation.root.children[1]
                .find(|item| item.fields.get("text") == Some(&Value::Str("still here".into())))
                .is_some()
        );
    }
}

#[test]
fn block_written_bodies_use_the_same_fragment_rules() {
    for path in ["doc.not", "doc.md"] {
        for (body, valid) in [
            ("#divider()\n#divider()", false),
            ("#divider()\n\n#divider()", true),
            ("#emph[a]\n#strong[b]", true),
        ] {
            let source = format!("#callout[\n{body}\n]");
            let analysis = Pipeline::default()
                .analyze(path, &source, &Context::default())
                .unwrap();
            assert_eq!(
                analysis.diagnostics().is_empty(),
                valid,
                "{path}: {:?}",
                analysis.diagnostics()
            );
        }
    }
}

#[test]
fn code_literals_keep_fragments_and_splice_a_whole_block_forest() {
    let pipeline = Pipeline::default();
    for path in ["literal.not", "literal.md"] {
        let module = pipeline
            .compile_module(
                path,
                "demo",
                "demo/lib.notc",
                r#"
fn forest() -> BlockContent {
    let blocks = [ first

second ];
    [ #blocks ]
}
fn invalid() -> BlockContent { [ #divider()
#divider() ] }
"#,
            )
            .unwrap();
        let context = Context::default().with_module(module).unwrap();
        let good = pipeline
            .analyze("doc.not", "#self.forest()", &context)
            .unwrap();
        assert!(
            good.diagnostics().is_empty(),
            "{path}: {:?}",
            good.diagnostics()
        );
        let result = pipeline.evaluate(&good, &context);
        assert!(
            result.diagnostics.is_empty(),
            "{path}: {:?}",
            result.diagnostics
        );
        let paragraphs: Vec<_> = result
            .root
            .descendants()
            .filter(|item| item.ctor == Ctor::Paragraph)
            .collect();
        assert_eq!(paragraphs.len(), 2, "{path}");
        let bad = pipeline
            .analyze("doc.not", "#self.invalid()", &context)
            .unwrap();
        assert!(
            bad.diagnostics()
                .iter()
                .any(|d| d.message.contains("separate blocks with a blank line")),
            "{path}: {:?}",
            bad.diagnostics()
        );
    }
}

#[test]
fn fragment_placement_uses_output_type_independently_of_body_flavor() {
    let context = Context::default()
        .with_module(
            notist_pipeline::Pipeline::default()
                .compile_module(
                    "content.not",
                    "demo",
                    "demo/lib.notc",
                    r#"
fn block()[children: InlineContent] -> BlockContent { [ #children ] }
fn inline()[children: BlockContent] -> InlineContent { [inline] }
"#,
                )
                .unwrap(),
        )
        .unwrap();
    for path in ["doc.not", "doc.md"] {
        let good = Pipeline::default()
            .analyze(path, "before #self.inline[ body ] after", &context)
            .unwrap();
        assert!(
            good.diagnostics().is_empty(),
            "{path}: {:?}",
            good.diagnostics()
        );
        let bad = Pipeline::default()
            .analyze(path, "before #self.block[body] after", &context)
            .unwrap();
        assert!(
            bad.diagnostics()
                .iter()
                .any(|d| d.message.contains("separate blocks with a blank line")),
            "{path}: {:?}",
            bad.diagnostics()
        );
    }
}

#[test]
fn explicit_paragraphs_are_checked_and_evaluated_as_calls() {
    let pipeline = Pipeline::default();
    let context = Context::default();
    for path in ["doc.not", "doc.md"] {
        let good = pipeline.analyze(path, "#paragraph()", &context).unwrap();
        assert!(
            good.diagnostics().is_empty(),
            "{path}: {:?}",
            good.diagnostics()
        );
        assert!(matches!(
            good.root().children[0].pending.as_deref(),
            Some(PendingEvaluation::Call(_))
        ));
        assert_eq!(
            pipeline.evaluate(&good, &context).root.children[0].evaluation,
            Some(EvaluationState::Native)
        );
        let bad = pipeline
            .analyze(path, "#paragraph<true>()\n\n#emph[ok]", &context)
            .unwrap();
        assert!(!bad.diagnostics().is_empty());
        let result = pipeline.evaluate(&bad, &context);
        assert!(matches!(
            result.root.children[0].evaluation,
            Some(EvaluationState::Error(_))
        ));
        assert_eq!(result.root.children.len(), 2);
        let invalid_mount = pipeline
            .analyze(path, "#paragraph()[#divider()]", &context)
            .unwrap();
        assert!(
            invalid_mount
                .diagnostics()
                .iter()
                .any(|d| d.message.contains("takes inline content"))
        );
        let evaluated = pipeline.evaluate(&invalid_mount, &context);
        assert_eq!(evaluated.root.children.len(), 1);
        assert_eq!(evaluated.root.children[0].ctor, Ctor::Paragraph);
        assert!(matches!(
            evaluated.root.children[0].evaluation,
            Some(EvaluationState::Error(_))
        ));
    }
}

#[test]
fn nested_block_slots_keep_their_own_fragment_boundaries() {
    let pipeline = Pipeline::default();
    for dialect in ["literal.not", "literal.md"] {
        let module = pipeline
            .compile_module(
                dialect,
                "demo",
                "demo/lib.notc",
                r#"
fn inline()[children: BlockContent] -> InlineContent { [inline] }
fn nested() -> InlineContent { [before #inline[ first

second ] after] }
"#,
            )
            .unwrap();
        let context = Context::default().with_module(module).unwrap();
        let analysis = pipeline
            .analyze("doc.not", "#self.nested()", &context)
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{dialect}: {:?}",
            analysis.diagnostics()
        );
        assert!(
            pipeline
                .evaluate(&analysis, &context)
                .diagnostics
                .is_empty()
        );
    }
}

#[test]
fn pipe_cells_obey_content_rules_and_structural_mounts_keep_their_roles() {
    let pipeline = Pipeline::default();
    for path in ["doc.not", "doc.md"] {
        let context = Context::default();
        let mixed_cell = pipeline
            .analyze(
                path,
                "| cell |\n| --- |\n| before #divider() after |",
                &context,
            )
            .unwrap();
        assert!(
            mixed_cell
                .diagnostics()
                .iter()
                .any(|d| d.message.contains("separate blocks with a blank line")),
            "{path}: {:?}",
            mixed_cell.diagnostics()
        );
        let table = pipeline
            .analyze(path, "#table[ #row[ #cell[ a ] #cell[ b ] ] ]", &context)
            .unwrap();
        assert!(
            table.diagnostics().is_empty(),
            "{path}: {:?}",
            table.diagnostics()
        );
        assert_eq!(table.root().children[0].ctor, Ctor::Table);
        assert_eq!(table.root().children[0].children[0].children.len(), 2);
        let paragraph = pipeline
            .analyze(path, "#table[ #paragraph() ]", &context)
            .unwrap();
        assert!(
            paragraph
                .diagnostics()
                .iter()
                .any(|d| d.message.contains("takes only row children")),
            "{path}: {:?}",
            paragraph.diagnostics()
        );
    }
}
