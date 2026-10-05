#[test]
fn query_selectors() {
    let src = "@(id: \"x\", tags: (\"a\",))#emph[目标]\n\n*粗*\n";
    let (item, diags) = notist::Pipeline::default()
        .analyze("test.not", src, notist::builtins::registry())
        .unwrap()
        .into_parts();
    assert!(diags.is_empty());
    assert_eq!(notist::query::select(&item, "ctor:strong").len(), 1);
    assert_eq!(notist::query::select(&item, "id:x").len(), 1);
    assert_eq!(notist::query::select(&item, "tag:a").len(), 1);
    assert!(notist::query::select(&item, "tag:zz").is_empty());
}
