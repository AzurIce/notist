#[test]
fn index_ids_and_tags() {
    let src = "@(id: \"a\", tags: (\"x\", \"y\"))#strong[第一]\n\n@(id: \"b\", tags: (\"x\",))#strong[第二]\n";
    let (item, diags) = notist::analyze(src);
    assert!(diags.is_empty());
    let mut d = Vec::new();
    let index = notist::index::Index::build(&item, &mut d);
    assert!(d.is_empty());
    assert!(index.by_id("a").is_some());
    assert_eq!(index.by_id("z"), None);
    assert_eq!(index.by_tag("x").len(), 2);
    assert_eq!(index.by_tag("y").len(), 1);
    assert!(index.by_tag("z").is_empty());
    let mut ids: Vec<_> = index.ids().collect();
    ids.sort();
    assert_eq!(ids, ["a", "b"]);
}

#[test]
fn index_duplicate_id_is_diagnosed() {
    let src = "@(id: \"a\")#strong[第一]\n\n@(id: \"a\")#strong[第二]\n";
    let (item, _) = notist::analyze(src);
    let mut d = Vec::new();
    let index = notist::index::Index::build(&item, &mut d);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].message, "duplicate id `a`");
    assert!(index.by_id("a").is_some());
}
