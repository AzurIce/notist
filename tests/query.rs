#[test]
fn query_selectors() {
    let src = "@(id: \"x\", tags: (\"a\",))#note[目标]\n\n*粗*\n";
    let (item, diags) = notist::analyze(src);
    assert!(diags.is_empty());
    assert_eq!(notist::query::select(&item, "ctor:strong").len(), 1);
    assert_eq!(notist::query::select(&item, "id:x").len(), 1);
    assert_eq!(notist::query::select(&item, "tag:a").len(), 1);
    assert!(notist::query::select(&item, "tag:zz").is_empty());
    let json = notist::query::render_json(src, &notist::query::select(&item, "id:x"));
    assert!(json.contains("\"ctor\":\"note\""));
    assert!(json.contains("\"tags\":[\"a\"]"));
}
