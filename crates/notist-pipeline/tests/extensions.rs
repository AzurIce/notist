use notist_core::extensions::{Replace, Replacement};
use notist_core::{
    builtins::Accepts,
    definitions::{DefinitionModule, FunctionDef, FunctionId, ParameterDef, ParameterMode, Type},
    item::{Ctor, Value},
};
use notist_pipeline::{Context, Pipeline, extensions::ExtensionPlan};

fn replacement_plan(
    rules: &[Replacement],
    registry: &notist_core::registry::Registry,
) -> Result<ExtensionPlan, Vec<notist_core::diag::Diagnostic>> {
    ExtensionPlan::default().with_extension(
        std::sync::Arc::new(Replace::new(rules.iter().cloned())),
        registry,
    )
}

fn rule(a: &str, b: &str) -> Replacement {
    let id = |s: &str| {
        let (p, n) = s.split_once(".").unwrap();
        FunctionId::new(p, n)
    };
    Replacement {
        from: id(a),
        to: id(b),
        span: Default::default(),
    }
}
fn context() -> Context {
    Context::default().with_module(notist_pipeline::Pipeline::default().compile_module("content.not", "maths", "maths/lib.notc","fn first<C: Type = InlineContent>(text: String) -> C; fn second<K: Type = InlineContent>(text: String) -> K; fn wrong(source: String) -> BlockContent;").unwrap()).unwrap()
}
#[test]
fn both_frontends_preserve_identity_and_select_native_targets() {
    let c = context()
        .with_extension(Replace::new([rule("notist.math", "maths.first")]))
        .unwrap();
    let p = Pipeline::default();
    for path in ["test.not", "test.md"] {
        let a = p.analyze(path, "before $x$\n\n$ y $\n\nafter", &c).unwrap();
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        let e = p.evaluate(&a, &c);
        assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
        let before: Vec<_> = a
            .root()
            .descendants()
            .filter(|i| i.ctor == Ctor::Math)
            .collect();
        let after: Vec<_> = e
            .root
            .descendants()
            .filter(|i| i.ctor == Ctor::Math)
            .collect();
        assert_eq!(after.len(), 2);
        for (a, b) in before.into_iter().zip(after) {
            assert_eq!(a.function_id(), b.function_id());
            assert_eq!(a.fields, b.fields);
            assert_eq!(a.level, b.level);
            assert_eq!(a.span, b.span);
            assert_eq!(
                b.target.as_ref().unwrap().function,
                FunctionId::new("maths", "first")
            );
        }
    }
}
#[test]
fn rules_run_once_in_order_and_generic_names_are_normalized() {
    let c = context();
    let rules = [
        rule("notist.math", "maths.first"),
        rule("maths.first", "maths.second"),
        rule("maths.second", "notist.math"),
    ];
    let plan = replacement_plan(&rules, c.registry()).unwrap();
    assert_eq!(
        plan.target(&FunctionId::new("notist", "math")),
        FunctionId::new("notist", "math")
    );
    let plan = replacement_plan(&rules[..2], c.registry()).unwrap();
    assert_eq!(
        plan.target(&FunctionId::new("notist", "math")),
        FunctionId::new("maths", "second")
    );
    let plan = replacement_plan(&[rules[1].clone(), rules[0].clone()], c.registry()).unwrap();
    assert_eq!(
        plan.target(&FunctionId::new("notist", "math")),
        FunctionId::new("maths", "first")
    );
}
#[test]
fn incompatible_contracts_and_roles_are_rejected() {
    let c = context();
    for r in [
        rule("notist.math", "maths.wrong"),
        rule("notist.item", "notist.callout"),
        rule("notist.strong", "notist.emph"),
        rule("notist.math", "maths.missing"),
    ] {
        let result = replacement_plan(std::slice::from_ref(&r), c.registry());
        if r.from.name == "strong" {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
        }
    }
}
#[test]
fn replace_extension_can_execute_code_without_changing_semantic_call() {
    let c=Context::default().with_module(notist_pipeline::Pipeline::default().compile_module("content.not", "demo", "demo/lib.notc","fn loud()[children: InlineContent] -> InlineContent { [#text(text: \"!\") #children] }").unwrap()).unwrap().with_extension(Replace::new([rule("notist.emph","demo.loud")])).unwrap();
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "_hello_", &c).unwrap();
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    let emph = e.root.find(|i| i.ctor == Ctor::Emph).unwrap();
    assert_eq!(emph.function_id(), Some(FunctionId::new("notist", "emph")));
    assert!(
        emph.descendants()
            .any(|i| i.fields.get("text") == Some(&Value::Str("!".into())))
    );
}
#[test]
fn defaults_compare_exact_float_bits_and_parameter_binding_order() {
    let mut r = notist_core::builtins::registry().clone();
    let mut m = DefinitionModule::new("native");
    for (name, bits) in [
        ("a", 0x7ff8000000000001),
        ("b", 0x7ff8000000000001),
        ("c", 0x7ff8000000000002),
    ] {
        let mut f = FunctionDef::new(
            FunctionId::new("native", name),
            Accepts::Nothing,
            Type::InlineContent,
        );
        f.parameters.push(ParameterDef::new(
            "n",
            Type::Float,
            ParameterMode::Default(Value::Float(f64::from_bits(bits))),
        ));
        m.functions.push(f);
    }
    r.register(m).unwrap();
    assert!(replacement_plan(&[rule("native.a", "native.b")], &r).is_ok());
    assert!(replacement_plan(&[rule("native.a", "native.c")], &r).is_err());
}

#[test]
fn generated_paragraphs_participate_in_function_mapping() {
    let c = Context::default()
        .with_module(
            notist_pipeline::Pipeline::default()
                .compile_module(
                    "content.not",
                    "demo",
                    "demo/lib.notc",
                    "fn paragraph()[children: InlineContent] -> BlockContent { divider() }",
                )
                .unwrap(),
        )
        .unwrap()
        .with_extension(Replace::new([rule("notist.paragraph", "demo.paragraph")]))
        .unwrap();
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "ordinary text", &c).unwrap();
    let e = p.evaluate(&a, &c);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    assert_eq!(
        e.root.children[0].function_id(),
        Some(FunctionId::new("notist", "paragraph"))
    );
    assert_eq!(
        e.root.children[0].target.as_ref().unwrap().function,
        FunctionId::new("demo", "paragraph")
    );
}

#[test]
fn output_mapping_cannot_change_compile_time_source_analysis() {
    let base=Context::default().with_module(notist_pipeline::Pipeline::default().compile_module("content.not", "demo", "demo/lib.notc","fn original() -> Int { 1 } fn target() -> Int { 2 } fn show() -> BlockContent { heading<level: original()>()[title] }").unwrap()).unwrap();
    let mapped = base
        .clone()
        .with_extension(Replace::new([rule("demo.original", "demo.target")]))
        .unwrap();
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#self.show()", &base).unwrap();
    let b = p.analyze("doc.not", "#self.show()", &mapped).unwrap();
    assert_eq!(a, b);
    assert!(b.diagnostics().is_empty(), "{:?}", b.diagnostics());
    let e = p.evaluate(&b, &mapped);
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    assert_eq!(
        e.root
            .find(|i| i.ctor == Ctor::Heading)
            .unwrap()
            .fields
            .get("level"),
        Some(&Value::Int(1))
    );
}

#[test]
fn transparent_node_contract_cannot_be_replaced_by_an_opaque_code_function() {
    let c = Context::default()
        .with_module(
            notist_pipeline::Pipeline::default()
                .compile_module(
                    "content.not",
                    "demo",
                    "demo/lib.notc",
                    r#"
        fn replacement<C: Type = InlineContent>[children: C] -> C {
            if C == InlineContent { [replacement] } else { [ replacement ] }
        }
    "#,
                )
                .unwrap(),
        )
        .unwrap();
    for (from, to) in [
        ("notist.node", "demo.replacement"),
        ("demo.replacement", "notist.node"),
    ] {
        let errors = replacement_plan(&[rule(from, to)], c.registry()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("structural roles differ"))
        );
    }
    let p = Pipeline::default();
    let a = p
        .analyze("doc.not", "#list[ #node[ #item[x] ] ]", &c)
        .unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    assert!(p.evaluate(&a, &c).diagnostics.is_empty());
}
