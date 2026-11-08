#[test]
fn query_selectors() {
    let src = "@(id: \"x\", tags: (\"a\",))#emph[目标]\n\n*粗*\n";
    let (item, diags, _sources) = notist::Pipeline::default()
        .analyze("test.not", src, &notist::Context::default())
        .unwrap()
        .into_parts();
    assert!(diags.is_empty());
    assert_eq!(
        notist::query::select(&item, "ctor:strong", &_sources)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        notist::query::select(&item, "id:x", &_sources)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        notist::query::select(&item, "tag:a", &_sources)
            .unwrap()
            .len(),
        1
    );
    assert!(
        notist::query::select(&item, "tag:zz", &_sources)
            .unwrap()
            .is_empty()
    );
}
