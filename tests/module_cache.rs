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
