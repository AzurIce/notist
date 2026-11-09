use notist_pipeline::{Context, Pipeline};

fn context(source: &str) -> Context {
    let pipeline = Pipeline::default();
    Context::default()
        .with_module(
            pipeline
                .compile_module("doc.not", "demo", "demo/lib.notc", source)
                .unwrap(),
        )
        .unwrap()
}

fn messages(source: &str, document: &str) -> Vec<String> {
    let context = context(source);
    Pipeline::default()
        .analyze("doc.not", document, &context)
        .unwrap()
        .diagnostics()
        .iter()
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn inferred_and_explicit_type_predicates_work_for_code_and_native() {
    for body in [";", "{ [ok] }"] {
        let source = format!(
            "fn value<T: Type>(x: T) -> InlineContent\nwhere T == Path || T == String {body}"
        );
        for document in [
            r#"#self.value("hello")"#,
            r#"#self.value(p"./file")"#,
            r#"#self.value<String>("hello")"#,
        ] {
            assert!(messages(&source, document).is_empty(), "{document}");
        }
        let errors = messages(&source, "#self.value(42)");
        assert!(
            errors
                .iter()
                .any(|d| d.contains("where") && d.contains("false")),
            "{errors:?}"
        );
    }
}

#[test]
fn nested_generics_check_concrete_instances_without_range_proofs() {
    let source = r#"
        fn inner<T: Type>(x: T) -> InlineContent where T == String { [ok] }
        fn outer<T: Type>(x: T) -> InlineContent where T == String || T == Path {
            [#inner(x)]
        }
    "#;
    assert!(messages(source, r#"#self.outer("hello")"#).is_empty());
    let errors = messages(source, r#"#self.outer(p"./file")"#);
    assert!(
        errors
            .iter()
            .any(|d| d.contains("inner") && d.contains("where")),
        "{errors:?}"
    );
}

#[test]
fn predicates_use_comptime_value_arguments_and_defaults() {
    let source = "fn sized<n: Int = 2>() -> InlineContent where n > 0 && n < 4 { [ok] }";
    for document in ["#self.sized()", "#self.sized<1>()", "#self.sized<3>()"] {
        assert!(messages(source, document).is_empty(), "{document}");
    }
    for document in ["#self.sized<0>()", "#self.sized<4>()"] {
        assert!(
            messages(source, document)
                .iter()
                .any(|d| d.contains("where")),
            "{document}"
        );
    }
}

#[test]
fn non_bool_predicates_are_rejected() {
    let errors = messages(
        "fn f<n: Int>() -> InlineContent where n { [ok] }",
        "#self.f<1>()",
    );
    assert!(!errors.is_empty());
}

#[test]
fn forbidden_references_are_rejected_even_in_unselected_branches() {
    for condition in [
        "x",
        "children",
        "unknown",
        "true || x",
        "if true { true } else { x }",
        "helper()",
    ] {
        let source = format!(
            "fn f<T: Type>(x: T)[children: InlineContent] -> InlineContent where {condition} {{ [ok] }}"
        );
        let errors = Pipeline::default()
            .compile_module("doc.not", "demo", "demo/lib.notc", &source)
            .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|d| d.message.contains("where can only reference")),
            "{condition}: {errors:?}"
        );
    }
}

#[test]
fn failed_constraint_prevents_body_checking() {
    let errors = messages(
        "fn f<T: Type>(x: T) -> InlineContent where T == String { [#undefined()] }",
        "#self.f(42)",
    );
    assert!(errors.iter().any(|d| d.contains("where")));
    assert!(
        !errors.iter().any(|d| d.contains("undefined")),
        "{errors:?}"
    );
}

#[test]
fn type_constructor_predicates_and_local_comptime_bindings() {
    let source = "fn f<T: Type>(x: T) -> InlineContent where { let allowed = T == Array<Int>; allowed } { [ok] }";
    assert!(messages(source, "#self.f((1, 2))").is_empty());
}

#[test]
fn replacement_targets_check_their_own_concrete_constraints() {
    use notist_core::{
        definitions::FunctionId,
        extensions::{Replace, Replacement},
    };
    let context = context(
        r#"
        fn source<T: Type>(x: T) -> InlineContent { [source] }
        fn target<T: Type>(x: T) -> InlineContent where T == String { [target] }
    "#,
    )
    .with_extension(Replace::new([Replacement {
        from: FunctionId::new("demo", "source"),
        to: FunctionId::new("demo", "target"),
        span: Default::default(),
    }]))
    .unwrap();
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("doc.not", r#"#self.source(p"./file")"#, &context)
        .unwrap();
    assert!(analysis.diagnostics().is_empty());
    let output = pipeline.evaluate(&analysis, &context);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|d| d.message.contains("where")),
        "{:?}",
        output.diagnostics
    );
}

#[test]
fn deferred_pure_computation_is_evaluated_at_the_constraint_boundary() {
    let source = "fn f<values: Array<Int>>() -> InlineContent where values.len() > 0 { [ok] }";
    assert!(messages(source, "#self.f<(1, 2)>()").is_empty());
    assert!(
        messages(source, "#self.f<(,)>()")
            .iter()
            .any(|d| d.contains("where"))
    );
}
