use notist_core::{
    definitions::FunctionId,
    item::{Ctor, Value},
};
use notist_pipeline::{Context, Pipeline};
fn context(src: &str) -> Context {
    Context::default()
        .with_module(
            notist_pipeline::Pipeline::default()
                .compile_module("content.not", "demo", "demo/lib.notc", src)
                .unwrap(),
        )
        .unwrap()
}
#[test]
fn underline_is_available_to_code_content_functions() {
    let c = context(
        "fn underlined[children: InlineContent] -> InlineContent { [#underline[#children]] }",
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.underlined[内容]", &c).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    let underline = e
        .root
        .find(|i| i.ctor() == notist_core::item::Ctor::Underline)
        .unwrap();
    assert_eq!(
        underline.function_id(),
        Some(FunctionId::new("notist", "underline").def_id())
    );
    assert!(
        underline
            .descendants()
            .any(|i| i.field("text") == Some(&Value::Str("内容".into())))
    );
}

#[test]
fn code_bodies_are_opaque_until_evaluation() {
    let c = context("fn greet(name: String) -> InlineContent { [Hello #text(text: name)] }");
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.greet(\"world\")", &c).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let original = a
        .root()
        .find(|i| i.function_id() == Some(FunctionId::new("demo", "greet").def_id()))
        .unwrap();
    assert!(original.children().is_empty());
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    assert!(
        e.root
            .find(|i| i.function_id() == Some(FunctionId::new("demo", "greet").def_id()))
            .is_none()
    );
    assert!(
        e.root
            .descendants()
            .any(|i| i.field("text") == Some(&Value::Str("world".into())))
    );
    assert!(original.children().is_empty());
}
#[test]
fn concrete_type_branch_preserves_category_and_splices_forests() {
    let c = context(
        "fn draft<C: Type>[children: C] -> C { if C == InlineContent { [#emph[#children]] } else { [ #children ] } }",
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.draft[one]", &c).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    assert!(
        e.root
            .find(|i| i.ctor() == notist_core::item::Ctor::Emph)
            .is_some()
    );
}
#[test]
fn value_functions_runtime_arguments_and_failure_recovery() {
    let c = context(
        "fn twice(x: Int) -> Int { x * 2 } fn bad() -> InlineContent { compile_error(\"bad content\") } fn good(n: Int) -> InlineContent { [#text(text: if twice(n) == 6 { \"ok\" } else { \"no\" })] }",
    );
    let p = Pipeline::default();
    let a = p
        .analyze("doc.not", "#self.bad() #self.good(3)", &c)
        .unwrap();
    let e = p.evaluate(&a, &c);
    assert!(!e.diagnostics.is_empty());
    assert!(
        e.root
            .descendants()
            .any(|i| i.field("text") == Some(&Value::Str("ok".into()))),
        "{:?} {:?}",
        a.diagnostics(),
        e
    );
}
#[test]
fn runtime_branches_cannot_widen_return_types_or_specialize_on_arguments() {
    for src in [
        "fn f(flag: Bool) -> InlineContent { if flag { [x] } else { [ x ] } }",
        "fn f(flag: Bool) -> InlineContent { if flag { [x] } else { compile_error(\"unsupported\") } }",
        "fn f(flag: Bool) -> InlineContent { let x = comptime flag; [x] }",
    ] {
        let c = context(src);
        let p = Pipeline::default();
        let a = p.analyze("doc.not", "#self.f(true)", &c).unwrap();
        assert!(!a.diagnostics().is_empty(), "{src}");
        let e = p.evaluate(&a, &c);
        assert!(e.root.descendants().any(|i| i.is_error()));
    }
}
#[test]
fn compile_time_values_are_explicit_or_inferred_from_input_types() {
    let c = context(
        "fn identity<T: Type>(x: T) -> T { x } fn choose<b: Bool>(x: Int) -> Int { if b { x } else { x + 1 } } fn show() -> InlineContent { let n = comptime choose<true>(identity(2)); [#text(text: if n == 2 { \"yes\" } else { \"no\" })] }",
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.show()", &c).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    assert!(
        e.root
            .descendants()
            .any(|i| i.field("text") == Some(&Value::Str("yes".into())))
    );
}
#[test]
fn content_implementation_is_checked_per_concrete_instance() {
    let c = context(
        "fn mark<C: Type>[children: C] -> C { match C { InlineContent => [#strong[#children]], BlockContent => [ #children ], _ => compile_error(\"not content\"), } }",
    );
    let p = Pipeline::default();
    for src in ["#self.mark[x]", "#self.mark[ x ]"] {
        let a = p.analyze("doc.not", src, &c).unwrap();
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        assert!(p.evaluate(&a, &c).diagnostics.is_empty());
    }
}
#[test]
fn type_values_cannot_escape_comptime_and_categories_do_not_convert() {
    assert!(
        notist_pipeline::Pipeline::default()
            .compile_module(
                "content.not",
                "bad",
                "bad/lib.notc",
                "fn f() -> Type { Type }"
            )
            .is_err()
    );
    assert!(
        notist_pipeline::Pipeline::default()
            .compile_module(
                "content.not",
                "bad",
                "bad/lib.notc",
                "fn f(x: Type) -> Int { 1 }"
            )
            .is_err()
    );
    let c = context(
        "fn block()[children: BlockContent] -> BlockContent { children } fn same<C: Type>(x: C) -> C { x }",
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.block[inline]", &c).unwrap();
    assert!(!a.diagnostics().is_empty());
    let a = p.analyze("doc.not", "#self.same<Type>(Type)", &c).unwrap();
    assert!(!a.diagnostics().is_empty());
}
#[test]
fn code_literals_use_selected_frontend_and_preserve_definition_origins() {
    let p = Pipeline::default();
    let m = p
        .compile_module(
            "dialect.md",
            "demo",
            "widgets/widget.mdc",
            "fn f() -> InlineContent { [**bold**] }",
        )
        .unwrap();
    let c = Context::default().with_module(m).unwrap();
    let a = p.analyze("doc.not", "#self.f()", &c).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate(&a, &c);
    let strong = e
        .root
        .find(|i| i.ctor() == notist_core::item::Ctor::Strong)
        .unwrap();
    let o = strong.origin.as_ref().unwrap();
    assert_eq!(e.sources.name(o.source).unwrap(), "widgets/widget.mdc");
    assert_eq!(e.sources.name(o.expansion[0].source).unwrap(), "doc.not");
}
#[test]
fn recursive_calls_are_bounded_and_siblings_survive() {
    let c = context(
        "fn spin(n: Int) -> String { spin(n) } fn bad() -> InlineContent { [#text(text: spin(1))] } fn good() -> InlineContent { [fine] }",
    );
    let p = Pipeline::default();
    let a = p
        .analyze("doc.not", "#self.bad() #self.good()", &c)
        .unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate_with_limits(
        &a,
        &c,
        notist_pipeline::Limits {
            steps: 300,
            call_depth: 8,
            ..notist_pipeline::Limits::default()
        },
    );
    assert!(!e.diagnostics.is_empty());
    assert!(
        e.root
            .descendants()
            .any(|i| i.field("text") == Some(&Value::Str("fine".into())))
    );
}
#[test]
fn shape_is_finished_before_repeated_block_forest_splicing() {
    let c = context(
        "fn twice<C: Type>[children: C] -> C { if C == InlineContent { [#children#children] } else { [ #children\n\n#children ] } }",
    );
    let p = Pipeline::default();
    let a = p
        .analyze("doc.not", "= Outer\n\n#self.twice[\n= Inner\n\nText\n]", &c)
        .unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    assert_eq!(e.root.children().len(), 1);
    assert_eq!(
        e.root.children()[0]
            .descendants()
            .skip(1)
            .filter(|i| i.ctor() == notist_core::item::Ctor::Section)
            .count(),
        2
    );
}

#[test]
fn unannotated_nested_literals_dissolve_their_mount_groups() {
    let c = context(
        "fn f()[children: InlineContent] -> InlineContent { let a = [#children]; let b = [#a]; [#b] }",
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.f[x]", &c).unwrap();
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    let paragraph = &e.root.children()[0];
    assert_eq!(paragraph.children().len(), 1);
    assert_eq!(paragraph.children()[0].ctor(), Ctor::Text);
    assert!(e.root.find(|n| n.ctor() == Ctor::Group).is_none());
}
#[test]
fn shared_interpolation_grammar_has_context_specific_validation() {
    let c = context(
        "fn value() -> String { \"ok\" } fn show() -> InlineContent { [#text(text: value())] }",
    );
    let p = Pipeline::default();
    let a = p
        .analyze("doc.not", "#text(text: self.value())", &c)
        .unwrap();
    assert!(!a.diagnostics().is_empty());
    let a = p.analyze("doc.not", "#self.show()", &c).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    assert!(p.evaluate(&a, &c).diagnostics.is_empty());
}

#[test]
fn type_constructors_are_canonical_comptime_values() {
    let c = context(
        "fn first<T: Type>(x: T) -> Bool { T == Array<Int> } fn show() -> InlineContent { let b = comptime first<Array<Int>>((1, 2)); [#text(text: if b { \"array\" } else { \"other\" })] }",
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.show()", &c).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
}
#[test]
fn default_limits_recover_recursive_content_expansion() {
    let c = context("fn spin() -> InlineContent { [#spin()] }");
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.spin()", &c).unwrap();
    let e = p.evaluate(&a, &c);
    assert!(e.root.descendants().any(|i| i.is_error()));
}

#[test]
fn malformed_body_does_not_remove_unrelated_public_definitions() {
    let c = context("fn bad() -> InlineContent { fn good() -> InlineContent { [survives] }");
    let p = Pipeline::default();
    let a = p
        .analyze("doc.not", "#self.bad() #self.good()", &c)
        .unwrap();
    let e = p.evaluate(&a, &c);
    assert!(
        e.root
            .descendants()
            .any(|i| i.field("text") == Some(&Value::Str("survives".into())))
    );
    assert!(!a.diagnostics().is_empty());
}
#[test]
fn native_comptime_constraints_fail_at_content_boundaries() {
    let p = Pipeline::default();
    let c = Context::default();
    let a = p
        .analyze("doc.not", "#heading<level: 0>[bad]\n\nnormal", &c)
        .unwrap();
    let e = p.evaluate(&a, &c);
    assert!(e.root.descendants().any(|i| i.is_error()));
}

#[test]
fn comptime_expressions_share_normal_operators_and_parsing() {
    let c = context(
        "fn select<b: Bool>() -> InlineContent { if b { [yes] } else { [no] } } fn f<n: Int>() -> InlineContent { [#select<(n > 0)>()] }",
    );
    let p = Pipeline::default();
    let a = p
        .analyze("doc.not", "#self.f<1>() #self.select<(2 < 3)>()", &c)
        .unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    assert_eq!(
        e.root
            .descendants()
            .filter(|i| i.field("text") == Some(&Value::Str("yes".into())))
            .count(),
        2
    );
}

#[test]
fn checking_failure_recovers_at_nested_literal_interpolation_not_parent_function() {
    let c = context(
        "fn bad() -> InlineContent { compile_error(\"bad child\") } fn good() -> InlineContent { [good child] } fn outer() -> InlineContent { [#bad() #good()] }",
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.outer()", &c).unwrap();
    assert!(!a.diagnostics().is_empty());
    let e = p.evaluate(&a, &c);
    assert!(e.root.descendants().all(|n| n.as_expr().is_none()));
    assert!(
        e.root
            .descendants()
            .any(|i| i.field("text") == Some(&Value::Str("good child".into())))
    );
    assert!(
        e.root
            .find(|i| matches!(i.ctor(), Ctor::Error(ref message) if message.contains("bad child")))
            .is_some()
    );
}

#[test]
fn generic_defaults_are_checked_after_concrete_type_binding() {
    let c = context(
        "fn value<T: Type>(x: T = 1) -> T { x } fn show() -> InlineContent { let n = comptime value<Int>(); [#text(text: if n == 1 { \"default\" } else { \"wrong\" })] }",
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.show()", &c).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    assert!(p.evaluate(&a, &c).diagnostics.is_empty());
}

#[test]
fn content_bindings_stay_opaque_until_interpolated_and_recover_failures() {
    let c = context(
        "fn bad() -> InlineContent { compile_error(\"bad binding\") } fn good() -> InlineContent { [good binding] } fn outer() -> InlineContent { let content = bad(); [#content #good()] }",
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.outer()", &c).unwrap();
    let e = p.evaluate(&a, &c);
    assert!(
        e.root
            .find(
                |i| matches!(i.ctor(), Ctor::Error(ref message) if message.contains("bad binding"))
            )
            .is_some(),
        "{e:?}"
    );
    assert!(
        e.root
            .descendants()
            .any(|i| i.field("text") == Some(&Value::Str("good binding".into())))
    );
}
#[test]
fn discarded_content_is_not_evaluated() {
    let c = context(
        "fn spin() -> InlineContent { [#spin()] } fn keep()[children: InlineContent] -> InlineContent { [kept] } fn outer() -> InlineContent { let discarded = spin(); keep()[#spin()] }",
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.outer()", &c).unwrap();
    assert!(a.diagnostics().is_empty());
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    assert!(
        e.root
            .descendants()
            .any(|i| i.field("text") == Some(&Value::Str("kept".into())))
    );
}

#[test]
fn direct_calls_and_literal_interpolations_share_structural_validation() {
    use notist_core::item::Ctor;
    for dialect in ["dialect.not", "dialect.md"] {
        let p = Pipeline::default();
        let module = p
            .compile_module(
                dialect,
                "demo",
                "demo/lib.notc",
                r#"
            fn direct() -> BlockContent { list()[ #text(text: "invalid") ] }
            fn literal() -> BlockContent { [ #list[ #text(text: "invalid") ] ] }
            fn valid() -> BlockContent { table()[ #row[ #cell[x] ] ] }
        "#,
            )
            .unwrap();
        let c = Context::default().with_module(module).unwrap();
        for name in ["direct", "literal"] {
            let a = p.analyze(dialect, &format!("#self.{name}()"), &c).unwrap();
            assert!(
                a.diagnostics()
                    .iter()
                    .any(|d| d.message.contains("takes only item")),
                "{dialect}: {:?}",
                a.diagnostics()
            );
            let e = p.evaluate(&a, &c);
            let list = e.root.find(|n| n.is_error()).unwrap();
            assert!(list.children().is_empty());
        }
        let a = p.analyze(dialect, "#self.valid()", &c).unwrap();
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        let e = p.evaluate(&a, &c);
        assert!(e.diagnostics.is_empty(), "{e:#?}");
        let table = e.root.find(|n| n.ctor() == Ctor::Table).unwrap();
        assert_eq!(table.children()[0].ctor(), Ctor::TableRow);
        assert_eq!(table.children()[0].children()[0].ctor(), Ctor::TableCell);
    }
}

#[test]
fn incomplete_binding_preserves_known_call_contract_and_recovers_at_its_boundary() {
    use notist_core::{builtins::Level, item::Ctor};
    let c = context(
        r#"
        fn required<C: Type, n: Int>(label: String)[children: C] -> BlockContent { [ #children ] }
        fn forwarded() -> BlockContent { let failed = required("kept")[x]; [ #failed ] }
    "#,
    );
    let p = Pipeline::default();
    for path in ["doc.not", "doc.md"] {
        for src in [
            "@(id: \"failed\")#self.required(label: \"kept\")[x]\n\nend",
            "#self.forwarded()\n\nend",
        ] {
            let a = p.analyze(path, src, &c).unwrap();
            let e = p.evaluate(&a, &c);
            let required = e.root.find(|n| n.is_error()).unwrap();
            assert_eq!(required.level, Level::Block);
            assert!(required.children().is_empty());
            assert!(
                e.sources
                    .name(required.origin.as_ref().unwrap().source)
                    .unwrap()
                    .ends_with(if src.contains("forwarded") {
                        "lib.notc"
                    } else {
                        path
                    })
            );
            if !src.contains("forwarded") {
                assert_eq!(required.attrs.get("id"), Some(&Value::Str("failed".into())));
                let semantic = a
                    .root()
                    .find(|n| n.attrs.get("id") == Some(&Value::Str("failed".into())))
                    .unwrap();
                assert_eq!(semantic.level, Level::Block);
                assert!(semantic.is_error());
            }
            assert!(
                e.root.descendants().any(|n| n.ctor() == Ctor::Text
                    && n.field("text") == Some(&Value::Str("end".into())))
            );
        }
    }
}

#[test]
fn nested_content_in_interpolation_arguments_uses_the_selected_frontend() {
    for (dialect, bold) in [("doc.not", "*bold*"), ("doc.md", "**bold**")] {
        let source = format!(
            r#"
            fn label()[children: InlineContent] -> String {{ "label" }}
            fn direct() -> InlineContent {{ text(text: label()[{bold}]) }}
            fn literal() -> InlineContent {{ [#text(text: label()[{bold}])] }}
        "#
        );
        let p = Pipeline::default();
        let c = Context::default()
            .with_module(
                p.compile_module(dialect, "demo", "demo/lib.notc", &source)
                    .unwrap(),
            )
            .unwrap();
        for name in ["direct", "literal"] {
            let a = p.analyze(dialect, &format!("#self.{name}()"), &c).unwrap();
            assert!(
                a.diagnostics().is_empty(),
                "{dialect}: {:?}",
                a.diagnostics()
            );
            let e = p.evaluate(&a, &c);
            assert!(e.diagnostics.is_empty(), "{dialect}: {:?}", e.diagnostics);
            assert!(
                e.root
                    .descendants()
                    .any(|n| n.field("text") == Some(&Value::Str("label".into())))
            );
        }
    }
}

#[test]
fn exhausted_content_budget_does_not_disable_independent_siblings() {
    let c = context(
        r#"
        fn burn(n: Int) -> Int { if n == 0 { 1 } else { burn(n - 1) + burn(n - 1) } }
        fn exhaust() -> InlineContent { [#text(text: if burn(10) > 0 { "done" } else { "none" })] }
        fn good() -> InlineContent { [survives] }
        fn outer() -> InlineContent { [#exhaust() #good()] }
    "#,
    );
    let p = Pipeline::default();
    for src in ["#self.exhaust() #self.good()", "#self.outer()"] {
        let a = p.analyze("doc.not", src, &c).unwrap();
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        let e = p.evaluate_with_limits(
            &a,
            &c,
            notist_pipeline::Limits {
                steps: 300,
                call_depth: 32,
                ..notist_pipeline::Limits::default()
            },
        );
        assert!(
            e.diagnostics
                .iter()
                .any(|d| d.message.contains("step limit"))
        );
        assert!(
            e.root
                .descendants()
                .any(|n| n.field("text") == Some(&Value::Str("survives".into())))
        );
    }
}

#[test]
fn recovered_dependency_diagnostics_do_not_invalidate_a_successful_argument_call() {
    for (call, diagnostic) in [
        ("bad()", "discarded failure"),
        ("missing()", "unknown constructor"),
    ] {
        let c = context(&format!(
            r#"
            fn bad() -> InlineContent {{ compile_error("discarded failure") }}
            fn val() -> String {{ let discarded = [#{call}]; "ok" }}
            fn show() -> InlineContent {{ [#text(text: val())] }}
        "#
        ));
        let p = Pipeline::default();
        let a = p.analyze("doc.not", "#self.show()", &c).unwrap();
        assert!(
            a.diagnostics()
                .iter()
                .any(|d| d.message.contains(diagnostic)),
            "{:?}",
            a.diagnostics()
        );
        let e = p.evaluate(&a, &c);
        assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
        assert!(
            e.root
                .descendants()
                .any(|n| n.field("text") == Some(&Value::Str("ok".into())))
        );
        assert!(!e.root.descendants().any(|n| n.is_error()));
    }
}

#[test]
fn erased_array_elements_cannot_be_narrowed_but_known_empty_arrays_can_be_checked() {
    let p = Pipeline::default();
    let bad = context(
        r#"
        fn erase(x: Array) -> Array<Int> { x }
        fn show() -> InlineContent { let a = erase(("oops",)); [bad] }
    "#,
    );
    let a = p.analyze("doc.not", "#self.show()", &bad).unwrap();
    assert!(
        a.diagnostics()
            .iter()
            .any(|d| d.message.contains("expected array of integer")),
        "{:?}",
        a.diagnostics()
    );
    let good = context(
        r#"
        fn empty() -> Array<Int> { (,) }
        fn nested() -> Array<Array<Int>> { ((,),) }
        fn accept(x: Array<Int>) -> String { "ok" }
        fn show() -> InlineContent { let a = nested(); [#text(text: accept(empty())) #text(text: accept((,)))] }
    "#,
    );
    let a = p.analyze("doc.not", "#self.show()", &good).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate(&a, &good);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
}

#[test]
fn comptime_only_call_suffixes_work_in_code_and_both_content_dialects() {
    for dialect in ["doc.not", "doc.md"] {
        let p = Pipeline::default();
        let c = Context::default().with_module(p.compile_module(dialect, "demo", "demo/lib.notc", r#"
            fn label<n: Int>() -> InlineContent { [#text(text: if n == 1 { "one" } else { "two" })] }
            fn direct() -> InlineContent { label<1> }
            fn literal() -> InlineContent { [#label<1>] }
            fn number<n: Int>() -> Int { n }
            fn arithmetic() -> InlineContent {
                let n = number<1> /* known suffix */ + 1;
                [#text(text: if n == 2 { "one" } else { "two" })]
            }
        "#).unwrap()).unwrap();
        for name in [
            "label<1>",
            "label<self.number<1>>",
            "direct()",
            "literal()",
            "arithmetic()",
        ] {
            let a = p.analyze(dialect, &format!("#self.{name}"), &c).unwrap();
            assert!(
                a.diagnostics().is_empty(),
                "{dialect}: {:?}",
                a.diagnostics()
            );
            let e = p.evaluate(&a, &c);
            assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
            assert!(
                e.root
                    .descendants()
                    .any(|n| n.field("text") == Some(&Value::Str("one".into())))
            );
        }
    }
}

#[test]
fn comparisons_and_attached_comptime_calls_are_unambiguous() {
    let source = r#"
        fn range(x: Int) -> Bool { x < 10 && x > -1 }
        fn positive(x: Int) -> Bool { x < 10 && x > (0) }
        fn separated(x: Int) -> Bool { x/* comparison */<10 && x > -1 }
        fn number<n: Int>() -> Int { n }
        fn show() -> InlineContent {
            let n = comptime number<(1 + 1)>;
            let nested = comptime number<number<2>>;
            if range(5) && !range(-1) && positive(1) && separated(2) && n == 2 && nested == 2 {
                [passed]
            } else { [failed] }
        }
    "#;
    let p = Pipeline::default();
    for dialect in ["doc.not", "doc.md"] {
        let c = Context::default()
            .with_module(
                p.compile_module(dialect, "demo", "demo/lib.notc", source)
                    .unwrap(),
            )
            .unwrap();
        let a = p.analyze(dialect, "#self.show()", &c).unwrap();
        assert!(
            a.diagnostics().is_empty(),
            "{dialect}: {:?}",
            a.diagnostics()
        );
        let e = p.evaluate(&a, &c);
        assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
        assert!(
            e.root
                .descendants()
                .any(|n| n.field("text") == Some(&Value::Str("passed".into())))
        );
    }
}

#[test]
fn comptime_and_runtime_match_share_numeric_equality() {
    let c = context(
        r#"
        fn zero(x: Float) -> String { match x { 0.0 => "zero", _ => "other" } }
        fn nan(x: Float) -> String { match x { (0.0 / 0.0) => "nan", _ => "other" } }
        fn show() -> InlineContent {
            let a = match -0.0 { 0.0 => "zero", _ => "other" };
            let b = match (0.0 / 0.0) { (0.0 / 0.0) => "nan", _ => "other" };
            [#text(text: a)#text(text: zero(-0.0))#text(text: b)#text(text: nan(0.0 / 0.0))]
        }
    "#,
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.show()", &c).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    let text: Vec<_> = e
        .root
        .descendants()
        .filter_map(|n| match n.field("text") {
            Some(Value::Str(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, ["zero", "zero", "other", "other"]);
}

#[test]
fn expanded_content_size_recovery_preserves_later_siblings() {
    let c = context(&format!(
        r#"
        fn leaf() -> InlineContent {{ [#strong[#strong[x]]] }}
        fn chunk() -> InlineContent {{ [{}] }}
        fn good() -> InlineContent {{ [survives] }}
        fn outer() -> InlineContent {{ [#chunk() #chunk() #good()] }}
    "#,
        "#leaf()".repeat(15)
    ));
    let p = Pipeline::default();
    for src in ["#self.chunk() #self.chunk() #self.good()", "#self.outer()"] {
        let a = p.analyze("doc.not", src, &c).unwrap();
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        let e = p.evaluate_with_limits(
            &a,
            &c,
            notist_pipeline::Limits {
                content_nodes: 80,
                ..Default::default()
            },
        );
        assert!(e.root.find(|n| n.ctor() == Ctor::Strong).is_some());
        assert!(e.root.find(|n| matches!(n.ctor(), Ctor::Error(ref message) if message.contains("size limit"))).is_some());
        assert!(
            e.root
                .descendants()
                .any(|n| n.field("text") == Some(&Value::Str("survives".into())))
        );
        assert!(e.root.descendants().filter(|n| !n.is_error()).count() <= 80);
    }
}

#[test]
fn spliced_forest_size_recovery_keeps_small_later_content() {
    let c = context(
        r#"
        fn many() -> InlineContent {
            let one = [x];
            let four = [#one#one#one#one];
            let sixteen = [#four#four#four#four];
            [#sixteen#sixteen#sixteen#sixteen#sixteen#sixteen#text(text: "tail")]
        }
    "#,
    );
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.many()", &c).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate_with_limits(
        &a,
        &c,
        notist_pipeline::Limits {
            content_nodes: 75,
            ..Default::default()
        },
    );
    assert!(
        e.diagnostics
            .iter()
            .any(|d| d.message.contains("size limit"))
    );
    assert!(
        e.root
            .descendants()
            .any(|n| n.field("text") == Some(&Value::Str("tail".into())))
    );
    assert!(e.root.descendants().filter(|n| !n.is_error()).count() <= 75);
}

#[test]
fn comptime_arrays_check_all_elements_and_accept_known_empty_values() {
    let p = Pipeline::default();
    let c = context(
        r#"
        fn accept<a: Array<Int>>() -> InlineContent { [accepted] }
        fn bad<a: Array = (1, "wrong")>() -> InlineContent { accept<a>() }
        fn defaulted<T: Type>(value: T = (1, "wrong")) -> T { value }
        fn bad_default() -> InlineContent { let x = defaulted<Array<Int>>(); [unused] }
    "#,
    );
    for call in ["#self.bad()", "#self.bad_default()"] {
        let a = p.analyze("doc.not", call, &c).unwrap();
        assert!(!a.diagnostics().is_empty(), "{call}");
        assert!(p.evaluate(&a, &c).root.descendants().any(|i| i.is_error()));
    }
    for path in ["doc.not", "doc.md"] {
        let a = p.analyze(path, "#self.accept<(,)>", &c).unwrap();
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        let e = p.evaluate(&a, &c);
        assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
        assert!(
            e.root
                .descendants()
                .any(|i| i.field("text") == Some(&Value::Str("accepted".into())))
        );
    }
}

#[test]
fn markup_sugar_keeps_builtin_identity_inside_a_shadowing_module() {
    let p = Pipeline::default();
    for (dialect, body) in [
        ("content.not", "= Title\n\n*bold*"),
        ("content.md", "# Title\n\n**bold**"),
    ] {
        let source = format!(
            "fn text(x: Int) -> Int {{ x }} fn paragraph() -> Int {{ 0 }} fn heading() -> Int {{ 0 }} fn strong() -> Int {{ 0 }} fn view() -> BlockContent {{ [\n{body}\n] }}"
        );
        let module = p
            .compile_module(dialect, "demo", "lib.notc", &source)
            .unwrap();
        let c = Context::default().with_module(module).unwrap();
        let a = p.analyze("doc.not", "#self.view()", &c).unwrap();
        assert!(
            a.diagnostics().is_empty(),
            "{dialect}: {:?}",
            a.diagnostics()
        );
        let e = p.evaluate(&a, &c);
        assert!(e.diagnostics.is_empty(), "{dialect}: {:?}", e.diagnostics);
        for name in ["text", "paragraph", "heading", "strong"] {
            assert!(
                e.root
                    .find(|i| i.function_id() == Some(FunctionId::new("notist", name).def_id()))
                    .is_some(),
                "{dialect}: {name}"
            );
        }
    }
}

#[test]
fn reused_content_items_capture_each_calls_lexical_inputs_without_executing_unused_arguments() {
    let c = context(
        r#"
        fn spin() -> String { spin() }
        fn wrap()[children: InlineContent] -> InlineContent { [#children #children] }
        fn consume()[children: InlineContent] -> InlineContent { [kept] }
        fn make(name: String, flag: Bool) -> InlineContent {
            let content = [#text(text: { let suffix = if flag { "!" } else { "?" }; name + suffix })];
            [#wrap()[#content] #consume()[#text(text: spin())]]
        }
    "#,
    );
    let p = Pipeline::default();
    for dialect in ["doc.not", "doc.md"] {
        let a = p
            .analyze(
                dialect,
                "#self.make(\"first\", true) #self.make(\"second\", false)",
                &c,
            )
            .unwrap();
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        for _ in 0..2 {
            let result = p.evaluate(&a, &c);
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            let texts: Vec<_> = result
                .root
                .descendants()
                .filter_map(|i| match i.field("text") {
                    Some(Value::Str(s)) if s == "first!" || s == "second?" => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(texts, ["first!", "first!", "second?", "second?"]);
            assert!(result.root.descendants().all(|i| i.as_expr().is_none()));
        }
        assert!(a.root().descendants().any(|i| i.as_expr().is_some()));
    }
}
