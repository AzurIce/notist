#[test]
fn pipeline_dispatch_by_extension() {
    let pipeline = notist::frontend::Frontends::default();
    let (item, diags) = pipeline
        .analyze(std::path::Path::new("x.md"), "# 标题\n")
        .expect("md frontend");
    assert!(diags.is_empty());
    assert!(
        item.descendants()
            .any(|i| i.ctor == notist::item::Ctor::Heading)
    );
    assert!(
        pipeline
            .analyze(std::path::Path::new("x.not"), "正文\n")
            .is_some()
    );
    assert!(
        pipeline
            .analyze(std::path::Path::new("x.txt"), "正文\n")
            .is_none()
    );
}
