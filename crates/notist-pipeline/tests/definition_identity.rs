use notist_pipeline::{Context, Pipeline};

#[test]
fn an_owned_foreign_binding_with_the_same_name_does_not_alias_a_parameter() {
    use notist_core::{definitions::Type, evaluation::ExprKind, source::Source};
    let pipeline = Pipeline::default();
    let module = pipeline.compile_module("doc.not", "binding-no-fallback", "fallback.notc", "fn value(x: Int) -> Int { x } fn show() -> InlineContent { [#text(text: str(value(1)))] }").unwrap();
    let context = Context::default().with_module(module).unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.show()", &context)
        .unwrap();
    assert!(analysis.diagnostics().is_empty());
    let mut program = analysis.checked_program().clone();
    let owner = Source::metadata("foreign-binding.notc").unwrap();
    program.sources.insert(owner.clone()).unwrap();
    let foreign = program
        .sources
        .bindings_mut()
        .declare("x", &owner, Default::default())
        .unwrap();
    let record = std::sync::Arc::make_mut(&mut program.instances)
        .values_mut()
        .find(|record| record.definition().id.name == "value")
        .unwrap();
    let record = std::sync::Arc::make_mut(record);
    assert_ne!(
        record
            .named_inputs()
            .find(|(name, _)| *name == "x")
            .unwrap()
            .1,
        foreign
    );
    let body = record.body.as_mut().unwrap();
    body.kind = ExprKind::Input(foreign);
    body.ty = Some(Type::Int);
    body.known = None;
    let result = notist_eval::evaluate(
        analysis.root(),
        &program,
        analysis.sources(),
        &context,
        Default::default(),
    );
    assert!(
        result.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("runtime input `x` is unavailable")),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn checked_implementation_records_cannot_impersonate_another_definition_id() {
    let pipeline = Pipeline::default();
    let module = pipeline
        .compile_module(
            "doc.not",
            "numeric-record",
            "record.notc",
            "fn show() -> InlineContent { [original] } fn other() -> InlineContent { [other] }",
        )
        .unwrap();
    let context = Context::default().with_module(module).unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.show()", &context)
        .unwrap();
    let mut program = analysis.checked_program().clone();
    let other = context.resolve("self.other").unwrap().id.clone();
    let record = std::sync::Arc::make_mut(&mut program.instances)
        .values_mut()
        .find(|record| record.definition().id.name == "show")
        .unwrap();
    let mut definition = record.definition().clone();
    definition.id = other;
    *record = std::sync::Arc::new((**record).clone().with_definition(definition));
    let result = notist_eval::evaluate(
        analysis.root(),
        &program,
        analysis.sources(),
        &context,
        Default::default(),
    );
    assert!(
        result.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("function instance identity mismatch")),
        "{:?}",
        result.diagnostics
    );
}
