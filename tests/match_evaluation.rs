//! Ordinary, Option and Result matches obey the same strict subject law.
use notist::{
    Context, EvaluationInputs, EvaluationOutcome, InputError, InputRead, Item, Limits, Pipeline,
    ResourcePath, Value,
};
use std::{cell::Cell, sync::Arc};

struct Inputs {
    response: InputRead,
    calls: Cell<usize>,
}
impl EvaluationInputs for Inputs {
    fn file(&self, _: &ResourcePath) -> InputRead {
        self.calls.set(self.calls.get() + 1);
        self.response.clone()
    }
}

fn context(source: &str) -> Context {
    Context::default()
        .with_resource_root("/match")
        .with_module(
            Pipeline::default()
                .compile_module_with_root(
                    "body.not",
                    "match-laws",
                    "/match/lib.notc",
                    source,
                    "/match",
                )
                .unwrap(),
        )
        .unwrap()
}

fn text(root: &Item) -> String {
    root.descendants()
        .filter_map(|node| match node.field("text") {
            Some(Value::Str(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn subjects_run_once_even_when_the_payload_is_reused_or_discarded() {
    let context = context(
        r#"
fn optional(source: Path) -> Option<String> { some(read_text(source)) }
fn scalar(source: Path) -> String { match read_text(source) { "a" => "a", "b" => "b", _ => "other" } }
fn option(source: Path) -> String { match optional(source) { some(value) => value + value, none => "other" } }
fn wildcard(source: Path) -> String { match read_text(source) { _ => "other" } }
fn result(source: Path) -> String { match ok<String, String>(read_text(source)) { ok(value) => value + value, err(error) => error } }
fn show(source: Path, kind: Int) -> InlineContent {
    [#text(text: match kind { 0 => scalar(source), 1 => option(source), 2 => wildcard(source), _ => result(source) })]
}
"#,
    );
    let pipeline = Pipeline::default();
    for (kind, expected) in [
        (0, "other"),
        (1, "hellohello"),
        (2, "other"),
        (3, "hellohello"),
    ] {
        let analysis = pipeline
            .analyze(
                "/match/doc.not",
                &format!("#self.show(p\"data.txt\", {kind})"),
                &context,
            )
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        for response in [
            InputRead::Ready(Arc::from(b"hello".as_slice())),
            InputRead::Failed(InputError::NotFound),
            InputRead::Pending,
        ] {
            let inputs = Inputs {
                response: response.clone(),
                calls: Cell::new(0),
            };
            let outcome = pipeline.evaluate_with_inputs_and_limits(
                &analysis,
                &context,
                &inputs,
                Limits {
                    read_requests: 1,
                    ..Default::default()
                },
            );
            assert_eq!(inputs.calls.get(), 1, "kind={kind}");
            match (response, outcome) {
                (InputRead::Ready(_), EvaluationOutcome::Complete(output)) => {
                    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
                    assert_eq!(text(&output.root), expected);
                    assert_eq!(output.dependencies.len(), 1);
                }
                (InputRead::Failed(_), EvaluationOutcome::Complete(output)) => {
                    assert!(
                        output
                            .diagnostics
                            .iter()
                            .any(|d| d.message.contains("resource not found"))
                    );
                    assert!(!text(&output.root).contains("other"));
                }
                (InputRead::Pending, EvaluationOutcome::NeedResources(needed)) => {
                    assert_eq!(needed.len(), 1)
                }
                (_, outcome) => panic!("unexpected match outcome: {outcome:?}"),
            }
        }
    }
}

#[test]
fn wildcard_cannot_erase_a_failing_subject_or_execute_unselected_arms() {
    let context = context(
        r#"
fn fail() -> String { panic("subject must run") }
fn direct() -> String { match fail() { _ => "incorrect success" } }
fn bound() -> String { let value = fail(); match value { _ => "incorrect success" } }
fn selected(value: String) -> String { match value { "yes" => "selected", _ => panic("unselected") } }
fn show(kind: Int) -> InlineContent { [#text(text: match kind { 0 => direct(), 1 => bound(), _ => selected("yes") })] }
"#,
    );
    let pipeline = Pipeline::default();
    for kind in 0..3 {
        let analysis = pipeline
            .analyze("/match/doc.not", &format!("#self.show({kind})"), &context)
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let output = pipeline.evaluate(&analysis, &context);
        if kind == 2 {
            assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
            assert_eq!(text(&output.root), "selected");
        } else {
            assert!(
                output
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("subject must run"))
            );
            assert!(!text(&output.root).contains("incorrect success"));
        }
    }
}
