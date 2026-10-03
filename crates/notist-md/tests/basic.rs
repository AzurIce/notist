fn analyze(src: &str) -> (notist_core::item::Item, Vec<notist_core::diag::Diagnostic>) {
    let (forest, attrs, mut diags) = notist_md::lower(src);
    let span = rowan::TextRange::new(0.into(), (src.len() as u32).into());
    let item = notist_core::analyze(forest, span, attrs, &mut diags);
    (item, diags)
}

#[test]
fn markdown_lowering() {
    let src = "# 标题 *em*\n\n段落 **粗** _斜_ `代码` [链接](x.not)。\n\n- 项一\n- 项二\n\n```rust\nfn main() {}\n```\n";
    let (item, diags) = analyze(src);
    assert!(diags.is_empty(), "{diags:?}");
    let dump = notist_core::dump::dump(&item);
    assert!(dump.contains("(heading @0..13 :level 1"), "{dump}");
    assert!(dump.contains("(strong"), "{dump}");
    assert!(dump.contains("(emph"), "{dump}");
    assert!(dump.contains(":lang \"rust\""), "{dump}");
    assert!(dump.contains(":target \"x.not\""), "{dump}");
    assert!(dump.contains("(list"), "{dump}");
}

#[test]
fn markdown_table_and_quote() {
    let src = "> 引用\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
    let (item, diags) = analyze(src);
    assert!(diags.is_empty(), "{diags:?}");
    let dump = notist_core::dump::dump(&item);
    assert!(dump.contains("blockquote"), "{dump}");
    assert!(dump.contains("table"), "{dump}");
    assert!(dump.contains("cell"), "{dump}");
}
