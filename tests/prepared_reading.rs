use notist::{PreparedFailure, PreparedInputs};
use serde_json::{Value, json};

const SOURCE: &str = "#import \"view.notc\" as view;\n#view.show(p\"./data.txt\")";
fn inputs() -> PreparedInputs {
    PreparedInputs {
        root: "/project".into(),
        incomplete: true,
        files: [
            ("/project/page.not", SOURCE),
            (
                "/project/view.notc",
                "fn show(source: Path) -> BlockContent { [ #text(text: read_text(source)) ] }",
            ),
        ]
        .into_iter()
        .map(|(path, source)| (path.into(), source.as_bytes().to_vec()))
        .collect(),
        ..Default::default()
    }
}
fn render(inputs: &PreparedInputs, debug: bool) -> Value {
    let render = if debug {
        notist::preview::analyze_prepared
    } else {
        notist::preview::render_prepared
    };
    serde_json::from_str(&render(
        "/project/page.not",
        SOURCE,
        &serde_json::to_string(inputs).unwrap(),
    ))
    .unwrap()
}
fn describe(inputs: &PreparedInputs) -> Value {
    serde_json::from_str(&notist::preview::describe_prepared(
        "/project/page.not",
        &serde_json::to_string(inputs).unwrap(),
    ))
    .unwrap()
}

#[test]
fn describe_render_and_analyze_request_the_same_missing_inputs_without_partial_results() {
    let inputs = inputs();
    let expected = json!({"need_resources": [{"root": "/project", "path": "data.txt"}]});
    assert_eq!(describe(&inputs), expected);
    for debug in [false, true] {
        assert_eq!(render(&inputs, debug), expected);
    }
}

#[test]
fn prepared_bytes_complete_a_retry_and_confirmed_failures_do_not_request_again() {
    let mut inputs = inputs();
    inputs
        .files
        .insert("/project/data.txt".into(), "正文 <&>".as_bytes().to_vec());
    assert_eq!(describe(&inputs)["functions"], json!([]));
    let plain = render(&inputs, false);
    let debug = render(&inputs, true);
    assert_eq!(plain["diagnostics"], json!([]));
    assert_eq!(plain["html"], debug["html"]);
    assert!(
        plain["html"]
            .as_str()
            .unwrap()
            .contains("正文 &lt;&amp;&gt;")
    );
    assert!(plain["used_components"].as_array().unwrap().is_empty());
    assert!(!plain["html"].as_str().unwrap().contains("data.txt"));
    inputs
        .files
        .remove(&std::path::PathBuf::from("/project/data.txt"));
    for (error, message) in [
        (PreparedFailure::NotFound, "not found"),
        (PreparedFailure::AccessDenied, "access denied"),
        (PreparedFailure::TooLarge, "size limit"),
    ] {
        inputs.failures.insert("/project/data.txt".into(), error);
        assert!(describe(&inputs).get("need_resources").is_none());
        for debug in [false, true] {
            let output = render(&inputs, debug);
            assert!(output.get("need_resources").is_none());
            let diagnostics = output["diagnostics"].as_array().unwrap();
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d["message"].as_str().unwrap().contains(message)
                        && d["path"] == "/project/view.notc"),
                "{output}"
            );
        }
    }
}

#[test]
fn closed_snapshots_and_read_grants_are_not_inferred_from_module_urls() {
    let mut inputs = inputs();
    inputs.incomplete = false;
    let output = render(&inputs, false);
    assert!(output.get("need_resources").is_none());
    assert!(
        output["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["message"].as_str().unwrap().contains("not found"))
    );
    inputs.incomplete = true;
    inputs.read_roots = Some(vec![]);
    inputs.module_urls.insert(
        "/project/data.txt".into(),
        "https://host.test/data.txt".into(),
    );
    let output = render(&inputs, false);
    assert!(output.get("need_resources").is_none());
    assert!(
        output["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["message"].as_str().unwrap().contains("access denied"))
    );
}

#[test]
fn published_component_urls_do_not_claim_empty_resource_bytes() {
    let mut inputs = inputs();
    inputs.files.insert(
        "/project/view.notc".into(),
        b"fn show(source: Path) -> BlockContent;".to_vec(),
    );
    inputs.module_urls.insert(
        "/project/show.js".into(),
        "https://host.test/show.js".into(),
    );
    let source = "#import \"view.notc\" as view;\n#view.show(p\"data.txt\")";
    // HTML correctly diagnoses the Path parameter rather than serializing it;
    // the binding is still found without dummy [] bytes for show.js.
    let result: Value = serde_json::from_str(&notist::preview::render_prepared(
        "/project/page.not",
        source,
        &serde_json::to_string(&inputs).unwrap(),
    ))
    .unwrap();
    assert!(result.get("error").is_none(), "{result}");
    assert!(!result["html"].as_str().unwrap().contains("data.txt"));
    assert!(
        result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["message"].as_str().unwrap().contains("Path"))
    );
    let mut vault = inputs.into_vault();
    let source = "#import \"view.notc\" as view;\n#view.show(p\"show.js\")";
    assert!(
        vault
            .render_html("page.not", source, Default::default())
            .is_ok()
    );
}

#[test]
fn legacy_prepared_wire_is_still_a_closed_view_and_code_checking_never_requests_data() {
    let input = json!({"root": "/project", "files": {"view.notc": b"fn f() -> String { read_text(p\"missing\") }".to_vec()}}).to_string();
    let code: Value = serde_json::from_str(&notist::preview::analyze_prepared(
        "/project/view.notc",
        "fn f() -> String { read_text(p\"missing\") }",
        &input,
    ))
    .unwrap();
    assert_eq!(code["diagnostics"], json!([]));
    assert!(code.get("need_resources").is_none());
}
