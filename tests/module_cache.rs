use notist::{InputLimits, MemoryResources, Vault};

fn resources() -> MemoryResources {
    let mut files = MemoryResources::new("/cache-budget");
    files.insert(
        "a.notc",
        br#"import "b.notc": value; fn show() -> InlineContent { [#text(text: value())] }"#
            .to_vec(),
    );
    files.insert("b.notc", br#"fn value() -> String { "hello" }"#.to_vec());
    files
}

#[test]
fn warm_module_caches_never_skip_host_source_loading_or_request_budgets() {
    let source = "#import \"a.notc\" as a;\n#a.show()";
    let limited = || {
        Vault::new(resources()).with_input_limits(InputLimits {
            requests: 1,
            ..Default::default()
        })
    };
    let cold = limited()
        .analyze("doc.not", source)
        .unwrap_err()
        .to_string();
    assert!(cold.contains("request limit exceeded"), "{cold}");
    let mut warmup = Vault::new(resources());
    let analysis = warmup.analyze("doc.not", source).unwrap();
    assert!(analysis.diagnostics().is_empty());
    assert!(
        warmup
            .evaluate("doc.not", &analysis)
            .unwrap()
            .diagnostics
            .is_empty()
    );
    let warm = limited()
        .analyze("doc.not", source)
        .unwrap_err()
        .to_string();
    assert_eq!(cold, warm);
}

#[test]
fn shared_graph_assembly_still_charges_each_documents_source_requests() {
    let mut vault = Vault::new(resources()).with_input_limits(InputLimits {
        requests: 3,
        ..Default::default()
    });
    let source = "#import \"a.notc\" as a;\n#a.show()";
    assert!(
        vault
            .analyze("first.not", source)
            .unwrap()
            .diagnostics()
            .is_empty()
    );
    let error = vault.analyze("second.not", source).unwrap_err().to_string();
    assert!(error.contains("request limit exceeded"), "{error}");
}

#[test]
fn checked_plans_do_not_collapse_module_identity_or_document_sources() {
    let mut files = resources();
    files.insert("other/b.notc", "fn value() -> String { \"other\" }");
    files.insert(
        "other/a.notc",
        "import \"b.notc\": value; fn show() -> InlineContent { [#text(text: value())] }",
    );
    let mut vault = Vault::new(files);
    for (document, module, expected) in [
        ("first.not", "a.notc", "hello"),
        ("second.not", "a.notc", "hello"),
        ("third.not", "other/a.notc", "other"),
        ("fourth.not", "a.notc", "hello"),
    ] {
        let source = format!("#import \"{module}\" as a;\n#a.show()");
        let output = vault
            .render_html(document, &source, Default::default())
            .unwrap();
        assert!(output.analysis.diagnostics().is_empty());
        assert!(output.evaluation.diagnostics.is_empty());
        assert_eq!(output.rendered.html.matches(expected).count(), 1);
        assert_eq!(
            output.analysis.sources()[&format!("/cache-budget/{document}")].as_ref(),
            source
        );
    }
}
