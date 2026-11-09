use notist::{Executor, MemoryResources, Vault};
use std::num::NonZeroUsize;

fn vault() -> Vault<MemoryResources> {
    let mut files = MemoryResources::new("/project");
    files.insert("view.notc", "fn show(source:Path)->InlineContent{let d=data_expect<Data>(read_json(source));let name=data_expect<String>(data_require<String>(d,\"name\"));[#text(text:name)]}");
    files.insert("data.json", r#"{"name":"shared result"}"#);
    files.insert("bad.json", r#"{"name":false}"#);
    Vault::new(files)
}

#[test]
fn batch_matches_serial_including_data_errors_dependencies_and_sources() {
    for workers in [1, 2, 4, 8] {
        let executor = Executor::new(NonZeroUsize::new(workers).unwrap()).unwrap();
        let mut vault = vault();
        // More than eight batches, with an uneven final batch.
        let documents: Vec<_> = (0..1041)
            .map(|i| {
                let data = if i % 7 == 0 { "bad.json" } else { "data.json" };
                let source =
                    format!("#import \"view.notc\" as view;\nPage {i}: #view.show(p\"{data}\")");
                vault
                    .analyze_document_with_resource_root(
                        format!("page-{i}.not"),
                        &source,
                        "/project",
                    )
                    .unwrap()
            })
            .collect();
        let jobs: Vec<_> = documents
            .iter()
            .map(|(analysis, handle)| (handle, analysis))
            .collect();
        // Evaluate concurrently with a cold parsed-data cache.
        let parallel = vault.evaluate_documents(&jobs, &executor).unwrap();
        assert_eq!(parallel.len(), documents.len());
        assert_eq!(vault.input_session().data_cache_stats().parses, 2);
        for ((analysis, handle), actual) in documents.iter().zip(parallel) {
            let expected = vault.evaluate_document(handle, analysis).unwrap();
            assert_eq!(actual.unwrap(), expected);
        }
        assert_eq!(vault.input_session().data_cache_stats().parses, 2);
    }
}

#[test]
fn batch_rejects_foreign_handles_and_accepts_empty_input() {
    let executor = Executor::new(NonZeroUsize::new(4).unwrap()).unwrap();
    let mut a = vault();
    let mut b = vault();
    let (analysis, handle) = a
        .analyze_document_with_resource_root("a.not", "hello", "/project")
        .unwrap();
    assert!(
        b.evaluate_documents(&[(&handle, &analysis)], &executor)
            .is_err()
    );
    assert!(a.evaluate_documents(&[], &executor).unwrap().is_empty());
}

#[test]
fn batch_preserves_each_pages_content_budget_recovery() {
    let executor = Executor::new(NonZeroUsize::new(4).unwrap()).unwrap();
    let mut vault = vault().with_evaluation_limits(notist::Limits {
        content_nodes: 1,
        ..Default::default()
    });
    let documents: Vec<_> = (0..257)
        .map(|i| {
            vault
                .analyze_document_with_resource_root(
                    format!("page-{i}.not"),
                    "before *nested* after",
                    "/project",
                )
                .unwrap()
        })
        .collect();
    let jobs: Vec<_> = documents
        .iter()
        .map(|(analysis, handle)| (handle, analysis))
        .collect();
    let results = vault.evaluate_documents(&jobs, &executor).unwrap();
    for ((analysis, handle), result) in documents.iter().zip(results) {
        let expected = vault.evaluate_document(handle, analysis).unwrap();
        assert!(!expected.diagnostics.is_empty());
        assert_eq!(result.unwrap(), expected);
    }
}
