use notist_pipeline::{Context, Pipeline, progress::timings};

#[test]
fn library_calls_record_core_stages_without_a_host() {
    let pipeline = Pipeline::default();
    let context = Context::default();
    let (_, report) = timings::capture(|| {
        let analysis = pipeline
            .analyze("example.not", "= Title\n\nText.", &context)
            .unwrap();
        pipeline.evaluate(&analysis, &context)
    });
    for stage in [
        "core.frontend",
        "core.parse",
        "core.lower",
        "core.check",
        "core.evaluate",
    ] {
        assert!(
            report.spans.iter().any(|span| span.stage == stage),
            "missing {stage}"
        );
    }
    assert!(report.spans.iter().all(|span| span.status == "finished"));
}

#[test]
fn memoized_modules_report_hits_without_fabricated_work() {
    let pipeline = Pipeline::default();
    let compile = || {
        pipeline
            .compile_module(
                "content.not",
                "timings-module-cache",
                "timing.notc",
                "fn value() -> Int { 42 }",
            )
            .unwrap()
    };
    let (_, cold) = timings::capture(compile);
    let (_, warm) = timings::capture(compile);
    let cache = |report: &timings::Report| {
        report
            .spans
            .iter()
            .find(|s| s.stage == "core.module.cache")
            .unwrap()
            .attributes["cache"]
            .clone()
    };
    assert_eq!(cache(&cold), "miss");
    assert_eq!(cache(&warm), "hit");
    assert!(cold.spans.iter().any(|span| span.stage == "core.parse"));
    assert!(
        !warm
            .spans
            .iter()
            .any(|span| span.stage == "core.parse" || span.stage == "core.module.compile.work")
    );
}
