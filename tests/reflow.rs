#[test]
fn reflow_splits_paragraphs_around_block_calls() {
    // 块 flavor 的 Custom 调用把段落切成兄弟
    let (item, diags) = notist::analyze("文字\n#note[\n块 body\n]\n后续\n");
    assert!(diags.is_empty());
    let dump = notist::dump::dump(&item);
    assert_eq!(
        dump,
        "\
(doc @0..32
  (paragraph @0..6
    (text @0..6 :text \"文字\")
  )
  (note @7..24
    (paragraph @14..22
      (text @14..22 :text \"块 body\")
    )
  )
  (paragraph @25..31
    (text @25..31 :text \"后续\")
  )
)
"
    );
}

#[test]
fn reflow_keeps_inline_calls_inside_paragraph() {
    // inline flavor 的调用不切
    let (item, diags) = notist::analyze("文字 #note[x] 后续\n");
    assert!(diags.is_empty());
    let dump = notist::dump::dump(&item);
    assert_eq!(dump.matches("(paragraph").count(), 1, "{dump}");
}
