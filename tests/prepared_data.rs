use notist::{PreparedFailure, PreparedInputs};
use serde_json::{Value, json};
const PAGE: &str = include_str!("../examples/structured-data/page.not");
fn inputs() -> PreparedInputs {
    PreparedInputs {
        root: "/project".into(),
        incomplete: true,
        files: [
            ("/project/page.not", PAGE),
            (
                "/project/views.notc",
                include_str!("../examples/structured-data/views.notc"),
            ),
        ]
        .into_iter()
        .map(|(name, text)| (name.into(), text.as_bytes().to_vec()))
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
        PAGE,
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
fn json_then_toml_prepare_as_distinct_immutable_retries_and_never_publish_partial_html() {
    let mut inputs = inputs();
    let json_request = json!({"need_resources":[{"root":"/project","path":"private/items.json"}]});
    assert_eq!(describe(&inputs), json_request);
    for debug in [false, true] {
        assert_eq!(render(&inputs, debug), json_request);
    }
    inputs.files.insert(
        "/project/private/items.json".into(),
        include_bytes!("../examples/structured-data/private/items.json").to_vec(),
    );
    let toml_request = json!({"need_resources":[{"root":"/project","path":"private/config.toml"}]});
    assert_eq!(describe(&inputs), toml_request);
    for debug in [false, true] {
        assert_eq!(render(&inputs, debug), toml_request);
    }
    inputs.files.insert(
        "/project/private/config.toml".into(),
        include_bytes!("../examples/structured-data/private/config.toml").to_vec(),
    );
    let plain = render(&inputs, false);
    let debug = render(&inputs, true);
    assert_eq!(plain["html"], debug["html"]);
    assert_eq!(plain["diagnostics"], json!([]));
    assert_eq!(plain["diagnostic_sources"], json!({}));
    assert_eq!(plain["used_components"], json!([]));
    let html = plain["html"].as_str().unwrap();
    assert!(html.contains("记录：共享数据视图"));
    assert!(!html.contains("private_note"));
    assert!(!html.contains("items.json"));
    assert!(!debug.to_string().contains("这一字段没有被选中"));
}
#[test]
fn prepared_data_diagnostics_attach_exact_text_and_fresh_views_do_not_reuse_old_trees() {
    let mut inputs = inputs();
    let broken = r#"{"42":{"name":12}}"#;
    inputs.files.insert(
        "/project/private/items.json".into(),
        broken.as_bytes().to_vec(),
    );
    assert!(describe(&inputs).get("need_resources").is_none());
    for debug in [false, true] {
        let output = render(&inputs, debug);
        let error = &output["diagnostics"][0];
        assert_eq!(error["path"], "/project/private/items.json");
        assert_eq!(
            output["diagnostic_sources"]["/project/private/items.json"],
            broken
        );
        assert!(error["message"].as_str().unwrap().contains("/42/name"));
    }
    inputs.files.insert(
        "/project/private/items.json".into(),
        br#"{"42":{"name":"new"}}"#.to_vec(),
    );
    inputs.files.insert(
        "/project/private/config.toml".into(),
        b"title='fresh:'".to_vec(),
    );
    let output = render(&inputs, false);
    assert_eq!(output["diagnostics"], json!([]));
    assert!(output["html"].as_str().unwrap().contains("fresh:new"));
}
#[test]
fn confirmed_input_errors_can_fallback_but_pending_and_hard_limits_cannot() {
    let mut inputs = inputs();
    inputs.files.insert("/project/views.notc".into(),br#"fn item(source:Path,id:String)->InlineContent{let label=match read_json(source){err(e)=>data_error_kind(e),ok(d)=>"loaded"};[#text(text:label)]}"#.to_vec());
    for debug in [false, true] {
        assert!(render(&inputs, debug).get("need_resources").is_some());
    }
    for (failure, label) in [
        (PreparedFailure::NotFound, "not_found"),
        (PreparedFailure::AccessDenied, "access_denied"),
        (PreparedFailure::NotFile, "not_file"),
    ] {
        inputs
            .failures
            .insert("/project/private/items.json".into(), failure);
        let output = render(&inputs, false);
        assert_eq!(output["diagnostics"], json!([]));
        assert!(output["html"].as_str().unwrap().contains(label));
    }
    inputs.failures.insert(
        "/project/private/items.json".into(),
        PreparedFailure::TooLarge,
    );
    let output = render(&inputs, false);
    assert!(
        output["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["message"].as_str().unwrap().contains("size limit"))
    );
}
