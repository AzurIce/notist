use notist_core::ide::{CompletionKind, WorkspaceAnalysis};
use notist_core::{
    BlockKind, CustomValue, InlineKind, NoDataHost, ReplacementErrorKind, ReplacementSpec, Value,
    check_project, parse_document,
};

#[test]
fn type_param_defaults_work_for_notist_math_and_user_generics() {
    let src = r#"
        fn wrap<A: Type, B: Type = Inline>(x: A, y: B) -> B {
            y
        }

        fn inline_default() -> Inline {
            math("x + 1")
        }

        fn block_explicit() -> Block {
            math<Block>("y = x^2")
        }

        fn notist_qualified() -> Inline {
            notist.math("z")
        }

        fn partial_explicit() -> Inline {
            wrap<Int>(42, text("ok"))
        }

        fn full_explicit() -> Block {
            wrap<Int, Block>(42, para(text("ok")))
        }
    "#;
    let project = check_project(&[("main.not", src)]).expect("defaults should type-check");

    let Value::Inline(inl) = project.run("main.not", "inline_default").unwrap() else {
        panic!("expected Inline");
    };
    assert!(matches!(&inl.node().kind, InlineKind::Math(s) if s.as_ref() == "x + 1"));

    let Value::Block(blk) = project.run("main.not", "block_explicit").unwrap() else {
        panic!("expected Block");
    };
    assert!(matches!(&blk.node().kind, BlockKind::Math(s) if s.as_ref() == "y = x^2"));

    let Value::Inline(q) = project.run("main.not", "notist_qualified").unwrap() else {
        panic!("expected Inline");
    };
    assert!(matches!(&q.node().kind, InlineKind::Math(s) if s.as_ref() == "z"));

    assert!(matches!(
        project.run("main.not", "partial_explicit").unwrap(),
        Value::Inline(_)
    ));
    assert!(matches!(
        project.run("main.not", "full_explicit").unwrap(),
        Value::Block(_)
    ));
}

#[test]
fn replaces_notist_math_with_extern_component_across_markup_calls_and_standalone_docs() {
    let katex = r#"
        fn math<C: Type = Inline>(text: String) -> C
        where C == Inline || C == Block;
    "#;
    let project = check_project(&[("modules/katex/lib.not", katex)])
        .expect("katex module should check");
    let replaced = project
        .with_replacements(&[ReplacementSpec {
            from_module: "notist".into(),
            from_name: "math".into(),
            to_module: "modules/katex/lib.not".into(),
            to_name: "math".into(),
        }])
        .expect("replacement should succeed");

    let doc_src = "Inline $a + b$ and #math(\"c\") and #notist.math(\"d\").\n\n$$\ny = x^2\n$$\n\n#display_math(\"e\")\n";
    let parsed = parse_document(doc_src);
    assert!(parsed.diagnostics.is_empty(), "diags: {:?}", parsed.diagnostics);
    let checked_doc = replaced
        .check_standalone_document("intro.md", &parsed.program.root)
        .expect("standalone doc should check");
    assert_eq!(
        checked_doc.extra_function_count(),
        0,
        "pre-lowered replaced math instances should be reused without extra document functions"
    );

    let root = replaced
        .eval_checked_document(&checked_doc, &NoDataHost)
        .expect("standalone doc should eval");
    let BlockKind::Fragment(blocks) = &root.node().kind else {
        panic!("expected root fragment, got {root}");
    };
    assert_eq!(blocks.len(), 3);

    // 1. Paragraph with 3 inline custom math nodes, each preserving call-site span.
    let BlockKind::Paragraph(p) = &blocks[0].node().kind else {
        panic!("expected paragraph, got {:?}", blocks[0]);
    };
    let InlineKind::Fragment(inlines) = &p.node().kind else {
        panic!("expected inline fragment, got {p:?}");
    };
    let custom_inlines: Vec<_> = inlines
        .iter()
        .filter_map(|i| match &i.node().kind {
            InlineKind::Custom(node) => Some((node, i.node().span)),
            _ => None,
        })
        .collect();
    assert_eq!(custom_inlines.len(), 3);
    for (idx, expected_text) in ["a + b", "c", "d"].iter().enumerate() {
        let (node, span) = &custom_inlines[idx];
        assert_eq!(node.module.as_ref(), "modules/katex/lib.not");
        assert_eq!(node.name.as_ref(), "math");
        assert_eq!(
            node.props.as_ref(),
            &[(
                std::sync::Arc::from("text"),
                CustomValue::String(std::sync::Arc::from(*expected_text))
            )]
        );
        assert!(
            !span.is_empty(),
            "custom node should preserve non-empty call-site span"
        );
    }

    // 2. Display math `$$...$$` -> block custom node.
    let BlockKind::Custom(node) = &blocks[1].node().kind else {
        panic!("expected block custom node, got {:?}", blocks[1]);
    };
    assert_eq!(node.module.as_ref(), "modules/katex/lib.not");
    assert_eq!(node.name.as_ref(), "math");
    assert_eq!(
        node.props.as_ref(),
        &[(
            std::sync::Arc::from("text"),
            CustomValue::String(std::sync::Arc::from("y = x^2"))
        )]
    );
    assert!(!blocks[1].node().span.is_empty());

    // 3. `#display_math("e")` delegates through `notist.math<Block>` -> replaced too.
    let BlockKind::Custom(node) = &blocks[2].node().kind else {
        panic!("expected display_math to route to replaced katex.math, got {:?}", blocks[2]);
    };
    assert_eq!(node.module.as_ref(), "modules/katex/lib.not");
    assert_eq!(node.name.as_ref(), "math");
    assert_eq!(
        node.props.as_ref(),
        &[(
            std::sync::Arc::from("text"),
            CustomValue::String(std::sync::Arc::from("e"))
        )]
    );
}

#[test]
fn wrapper_not_function_can_replace_notist_math_and_call_original_without_recursion() {
    let wrap = r#"
        fn math<C: Type = Inline>(text: String) -> C
        where C == Inline || C == Block
        {
            match C {
                Inline => strong(notist.math<Inline>(text)),
                Block  => callout("math", notist.math<Block>(text)),
                _      => compile_error("unreachable"),
            }
        }
    "#;
    let doc = "Check $x$.\n\n$$\ny\n$$\n";
    let project = check_project(&[("wrap.not", wrap), ("doc.md", doc)])
        .expect("wrapper project should check");
    let replaced = project
        .with_replacements(&[ReplacementSpec {
            from_module: "notist".into(),
            from_name: "math".into(),
            to_module: "wrap.not".into(),
            to_name: "math".into(),
        }])
        .expect("replacement should succeed");

    let root = replaced.eval_document("doc.md").expect("doc should eval");
    let BlockKind::Fragment(blocks) = &root.node().kind else {
        panic!("expected fragment");
    };
    // Paragraph has Strong(Math("x"))
    let BlockKind::Paragraph(p) = &blocks[0].node().kind else {
        panic!("expected paragraph");
    };
    let InlineKind::Fragment(items) = &p.node().kind else {
        panic!("expected inline fragment");
    };
    assert!(items.iter().any(|i| matches!(
        &i.node().kind,
        InlineKind::Strong(inner) if matches!(&inner.node().kind, InlineKind::Math(s) if s.as_ref() == "x")
    )));
    // Second block is Callout("math", BlockKind::Math("y"))
    assert!(matches!(
        &blocks[1].node().kind,
        BlockKind::Callout { kind, body }
            if kind.as_ref() == "math"
                && matches!(&body.node().kind, BlockKind::Math(s) if s.as_ref() == "y")
    ));
}

#[test]
fn replaces_user_module_component_and_is_immune_to_lexical_shadowing() {
    let components = r#"
        fn diagram(source: String) -> Block;
    "#;
    let pele = r#"
        fn diagram(source: String) -> Block;
    "#;
    let helper = r#"
        import "./components.not" as components;

        fn shadow_math(s: String) -> Inline {
            text("shadow")
        }

        fn demo() -> Block {
            let math = shadow_math;
            [
                #para([#math("local")$real_math$])
                #components.diagram("A -> B")
            ]
        }
    "#;
    let katex = r#"
        fn math<C: Type = Inline>(text: String) -> C
        where C == Inline || C == Block;
    "#;

    let project = check_project(&[
        ("components.not", components),
        ("pele.not", pele),
        ("katex.not", katex),
        ("helper.not", helper),
    ])
    .expect("project should check");

    let replaced = project
        .with_replacements(&[
            ReplacementSpec {
                from_module: "components.not".into(),
                from_name: "diagram".into(),
                to_module: "pele.not".into(),
                to_name: "diagram".into(),
            },
            ReplacementSpec {
                from_module: "notist".into(),
                from_name: "math".into(),
                to_module: "katex.not".into(),
                to_name: "math".into(),
            },
        ])
        .expect("replacements should succeed");

    let Value::Block(root) = replaced.run("helper.not", "demo").expect("demo should run") else {
        panic!("expected Block");
    };
    let BlockKind::Fragment(blocks) = &root.node().kind else {
        panic!("expected fragment");
    };
    // First block: local `math("local")` stayed `Text("shadow")`, while `$real_math$` became `katex.not::math`.
    let BlockKind::Paragraph(p) = &blocks[0].node().kind else {
        panic!("expected para");
    };
    let InlineKind::Fragment(inlines) = &p.node().kind else {
        panic!("expected inline fragment");
    };
    assert!(matches!(&inlines[0].node().kind, InlineKind::Text(s) if s.as_ref() == "shadow"));
    assert!(matches!(
        &inlines[1].node().kind,
        InlineKind::Custom(node)
            if node.module.as_ref() == "katex.not" && node.name.as_ref() == "math"
    ));

    // Second block: `components.diagram` was rebound to `pele.not::diagram`.
    assert!(matches!(
        &blocks[1].node().kind,
        BlockKind::Custom(node)
            if node.module.as_ref() == "pele.not" && node.name.as_ref() == "diagram"
    ));
}

#[test]
fn sequential_chaining_respects_rule_order_without_fixed_point() {
    let m = r#"
        fn a() -> String { "a" }
        fn b() -> String { "b" }
        fn c() -> String { "c" }
        fn call_a() -> String { a() }
        fn call_b() -> String { b() }
    "#;
    let project = check_project(&[("m.not", m)]).expect("module should check");

    let expect_str = |val: Value, expected: &str| {
        let Value::String(s) = val else {
            panic!("expected string");
        };
        assert_eq!(s.as_ref(), expected);
    };

    // Forward order: a -> b, b -> c  =>  a -> c, b -> c
    let forward = project
        .with_replacements(&[
            ReplacementSpec {
                from_module: "m.not".into(),
                from_name: "a".into(),
                to_module: "m.not".into(),
                to_name: "b".into(),
            },
            ReplacementSpec {
                from_module: "m.not".into(),
                from_name: "b".into(),
                to_module: "m.not".into(),
                to_name: "c".into(),
            },
        ])
        .expect("forward chain");
    expect_str(forward.run("m.not", "call_a").unwrap(), "c");
    expect_str(forward.run("m.not", "call_b").unwrap(), "c");

    // Reverse order: b -> c, a -> b  =>  a -> b, b -> c
    let reverse = project
        .with_replacements(&[
            ReplacementSpec {
                from_module: "m.not".into(),
                from_name: "b".into(),
                to_module: "m.not".into(),
                to_name: "c".into(),
            },
            ReplacementSpec {
                from_module: "m.not".into(),
                from_name: "a".into(),
                to_module: "m.not".into(),
                to_name: "b".into(),
            },
        ])
        .expect("reverse chain");
    expect_str(reverse.run("m.not", "call_a").unwrap(), "b");
    expect_str(reverse.run("m.not", "call_b").unwrap(), "c");
}

#[test]
fn rejects_incompatible_replacement_contracts() {
    let bad = r#"
        fn wrong_arity(text: String) -> Inline;
        fn wrong_default<C: Type = Block>(text: String) -> C where C == Inline || C == Block;
        fn wrong_param<C: Type = Inline>(n: Int) -> C where C == Inline || C == Block;
        fn wrong_value_default<C: Type = Inline>(text: String, extra: String) -> C
        where C == Inline || C == Block;
        fn origin_target<C: Type = Inline>(text: String, extra: String = "") -> C
        where C == Inline || C == Block;
        fn stricter_where<C: Type = Inline>(text: String) -> C where C == Inline;
    "#;
    let project = check_project(&[("bad.not", bad)]).expect("bad module checks on its own");

    let check_err = |name: &str| {
        project
            .with_replacements(&[ReplacementSpec {
                from_module: "notist".into(),
                from_name: "math".into(),
                to_module: "bad.not".into(),
                to_name: name.into(),
            }])
            .expect_err(&format!("{name} should be rejected"))
    };

    assert!(matches!(
        check_err("wrong_arity")[0].kind,
        ReplacementErrorKind::TypeArityMismatch { .. }
    ));
    assert!(matches!(
        check_err("wrong_default")[0].kind,
        ReplacementErrorKind::TypeParamDefaultMismatch { .. }
    ));
    assert!(matches!(
        check_err("wrong_param")[0].kind,
        ReplacementErrorKind::SignatureMismatch { .. }
    ));
    let value_default_err = project
        .with_replacements(&[ReplacementSpec {
            from_module: "bad.not".into(),
            from_name: "origin_target".into(),
            to_module: "bad.not".into(),
            to_name: "wrong_value_default".into(),
        }])
        .expect_err("different value-default arity should be rejected");
    assert!(matches!(
        value_default_err[0].kind,
        ReplacementErrorKind::ValueDefaultsMismatch { .. }
    ));
    assert!(matches!(
        check_err("stricter_where")[0].kind,
        ReplacementErrorKind::WhereClauseMismatch { .. }
    ));
}

#[test]
fn ide_supports_notist_namespace_and_default_type_param_signatures() {
    let lib = r#"
fn math<C: Type = Inline>(text: String) -> C
where C == Inline || C == Block;
"#;
    let doc = "#notist.math(\"x\")\n\n#notist.\n";
    let analysis = WorkspaceAnalysis::analyze(&[("lib.not", lib), ("doc.md", doc)]);
    assert!(
        analysis.diagnostics().iter().all(|d| d.module != "lib.not"),
        "lib.not should have no diagnostics: {:?}",
        analysis.diagnostics()
    );

    // Hover on lib.not's math shows `= Inline`.
    let def_hover = analysis
        .hover("lib.not", lib.find("math").unwrap() as u32)
        .expect("hover on lib math");
    assert!(
        def_hover.contents.contains("fn math<C: Type = Inline>(text: String) -> C"),
        "got: {}",
        def_hover.contents
    );

    // Hover on `#notist.math` shows built-in `notist.math` signature.
    let call_hover = analysis
        .hover("doc.md", doc.find("math").unwrap() as u32)
        .expect("hover on notist.math");
    assert!(
        call_hover.contents.contains("fn math<C: Type = Inline>(text: String) -> C"),
        "got: {}",
        call_hover.contents
    );

    // Completion after `#notist.` lists built-in functions including `math` and `list`.
    let dot_pos = (doc.rfind("notist.").unwrap() + "notist.".len()) as u32;
    let completions = analysis.completions("doc.md", dot_pos);
    assert!(
        completions
            .iter()
            .any(|c| c.label == "math" && c.kind == CompletionKind::Function),
        "completions: {completions:?}"
    );
    assert!(
        completions
            .iter()
            .any(|c| c.label == "list" && c.kind == CompletionKind::Function),
        "completions: {completions:?}"
    );
}
