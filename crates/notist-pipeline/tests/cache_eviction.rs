use notist_pipeline::{Pipeline, evict_caches};
use std::sync::Arc;

// Isolated test executable: global eviction must not race other cache tests.
#[test]
fn eviction_releases_entries_without_invalidating_live_artifacts() {
    let pipeline = Pipeline::default();
    let compile = || {
        pipeline
            .compile_module(
                "content.not",
                "eviction",
                "eviction.notc",
                "fn value() -> Int { 1 }",
            )
            .unwrap()
    };
    let first = compile();
    let second = compile();
    assert!(Arc::ptr_eq(&first.definitions, &second.definitions));
    evict_caches(0);
    let third = compile();
    assert!(!Arc::ptr_eq(&first.definitions, &third.definitions));
    assert_eq!(first, third);
}
