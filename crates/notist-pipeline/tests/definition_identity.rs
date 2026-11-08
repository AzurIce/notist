use notist_pipeline::{Context, Pipeline};

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
        .find(|record| record.definition.id.name == "show")
        .unwrap();
    std::sync::Arc::make_mut(record).definition.id = other;
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
