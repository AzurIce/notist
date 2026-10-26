#![cfg(all(feature = "command-extensions", unix))]
use notist::expr::Expr;
use notist::extensions::LoweredDocument;
use notist::{Ctor, FsResources, Item, RenderOptions, Vault};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn command_config(stage: &str, args: &[&str]) -> String {
    toml::to_string(
        &toml::Value::try_from(json!({"extensions": [{
            "kind": "command", "command": args, "stages": [stage]
        }]}))
        .unwrap(),
    )
    .unwrap()
}
fn setup(root: &Path, stage: &str, response: &str) {
    fs::write(
        root.join("edit.sh"),
        "cat > request.json\ncat response.json\n",
    )
    .unwrap();
    fs::write(root.join("response.json"), response).unwrap();
    fs::write(
        root.join("Notist.toml"),
        command_config(stage, &["sh", "edit.sh"]),
    )
    .unwrap();
}

#[test]
fn commands_edit_lowered_content_then_the_output_tree_and_preserve_ranges() {
    let root = tempfile::tempdir().unwrap();
    let source = "removed";
    let document = LoweredDocument {
        forest: vec![Expr::fragment(
            vec![Expr::text(
                "generated".into(),
                notist::TextRange::new(1.into(), 3.into()),
            )],
            Default::default(),
        )],
        ..Default::default()
    };
    setup(
        root.path(),
        "before_check",
        &json!({"version": 1, "document": document}).to_string(),
    );
    let mut vault = Vault::new(FsResources::new(root.path()));
    let output = vault
        .render_html("doc.not", source, RenderOptions::default())
        .unwrap();
    assert!(
        output.analysis.diagnostics().is_empty(),
        "{:?}",
        output.analysis.diagnostics()
    );
    assert!(output.rendered.html.contains("generated"));
    assert!(!output.rendered.html.contains("removed"));
    let request: Value =
        serde_json::from_str(&fs::read_to_string(root.path().join("request.json")).unwrap())
            .unwrap();
    assert_eq!(request["stage"], "before_check");
    assert_eq!(request["version"], 1);
    let path = root.path().join("doc.not").to_string_lossy().into_owned();
    assert_eq!(request["path"], path);
    assert_eq!(request["sources"][&path], source);
    assert!(
        request["functions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["id"]["name"] == "paragraph")
    );
    assert_eq!(
        u32::from(
            output
                .analysis
                .root()
                .find(|i| i.ctor == Ctor::Text)
                .unwrap()
                .span
                .start()
        ),
        1
    );

    let mut tree = output.evaluation.root;
    tree.children
        .push(Item::new(Ctor::Divider, Default::default()));
    setup(
        root.path(),
        "after_evaluate",
        &json!({"version": 1, "document": tree}).to_string(),
    );
    let output = Vault::new(FsResources::new(root.path()))
        .render_html("doc.not", source, RenderOptions::default())
        .unwrap();
    assert!(
        output.evaluation.diagnostics.is_empty(),
        "{:?}",
        output.evaluation.diagnostics
    );
    assert!(output.rendered.html.contains("generated"));
    assert!(output.rendered.html.contains("<hr"));
    assert_eq!(output.analysis.root().children.len(), 1);
    let request: Value =
        serde_json::from_str(&fs::read_to_string(root.path().join("request.json")).unwrap())
            .unwrap();
    assert_eq!(request["stage"], "after_evaluate");
}

#[test]
fn command_protocol_errors_and_diagnostics_roll_back_edits() {
    let root = tempfile::tempdir().unwrap();
    for (stage, response, message) in [
        ("before_check", "not json", "invalid JSON response"),
        (
            "before_check",
            r#"{"version":2}"#,
            "unsupported response version",
        ),
        (
            "before_check",
            r#"{"version":1,"document":{}}"#,
            "invalid lowered document",
        ),
        (
            "before_check",
            r#"{"version":1,"diagnostics":[{"phase":"Semantic","span":[0,0],"message":"script rejected"}]}"#,
            "script rejected",
        ),
        ("after_check", r#"{"version":1,"document":{}}"#, "read-only"),
        (
            "after_evaluate",
            r#"{"version":1,"document":{}}"#,
            "invalid output document",
        ),
    ] {
        setup(root.path(), stage, response);
        let output = Vault::new(FsResources::new(root.path()))
            .render_html("doc.md", "kept", RenderOptions::default())
            .unwrap();
        let errors: Vec<_> = output
            .analysis
            .diagnostics()
            .iter()
            .chain(&output.evaluation.diagnostics)
            .collect();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].message.contains(message), "{:?}", errors[0]);
        assert!(!errors[0].recovered);
        assert_eq!(
            errors[0].source.as_deref(),
            root.path().join("doc.md").to_str()
        );
        assert!(output.rendered.html.contains("kept"));
    }
}

#[test]
fn command_start_and_exit_failures_are_reported_at_the_requested_stage() {
    let root = tempfile::tempdir().unwrap();
    for args in [vec!["./not-a-program"], vec!["sh", "-c", "exit 7"]] {
        fs::write(
            root.path().join("Notist.toml"),
            command_config("before_check", &args),
        )
        .unwrap();
        let analysis = Vault::new(FsResources::new(root.path()))
            .analyze("doc.not", "kept")
            .unwrap();
        assert_eq!(analysis.diagnostics().len(), 1);
        let message = &analysis.diagnostics()[0].message;
        assert!(message.contains("before_check"));
        assert!(
            message.contains("cannot start command") || message.contains("exit status: 7"),
            "{message}"
        );
    }
}

#[test]
fn configuration_rejects_invalid_kinds_stages_fields_and_old_tables() {
    for source in [
        "[[extensions]]\nkind='missing'",
        "[[extensions]]\nkind='command'\ncommand=[]\nstages=['before_check']",
        "[[extensions]]\nkind='command'\ncommand=['sh']\nstages=[]",
        "[[extensions]]\nkind='command'\ncommand=['sh']\nstages=['before_check','before_check']",
        "[[extensions]]\nkind='command'\ncommand=['sh']\nstages=['unknown']",
        "[[extensions]]\nkind='command'\ncommand=['sh']\nstages=['before_check']\nfrom='text'",
        "[[extensions]]\nkind='replace'\nfrom='text'\nto='text'\ncommand=['sh']",
        "[[extensions]]\nkind='replace'\nfrom='text'",
        "[[transforms]]\nkind='replace'\nfrom='text'\nto='text'",
    ] {
        assert!(
            notist::environment::parse_config(source).is_err(),
            "{source}"
        );
    }
}
