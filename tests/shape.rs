#[test]
fn block_calls_require_source_fragment_boundaries() {
    let src = "文字\n\n#callout[\n块 body\n]\n\n后续\n";
    let analysis = notist::Pipeline::default()
        .analyze("test.not", src, &notist::Context::default())
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    assert_eq!(
        analysis
            .root()
            .children
            .iter()
            .map(|item| &item.ctor)
            .collect::<Vec<_>>(),
        [
            &notist::Ctor::Paragraph,
            &notist::Ctor::Callout,
            &notist::Ctor::Paragraph
        ]
    );
}

#[test]
fn soft_newlines_keep_inline_calls_inside_paragraph() {
    // 行内调用不改变片段边界。
    let (item, diags) = notist::Pipeline::default()
        .analyze(
            "test.not",
            "文字 #note[x] 后续\n",
            &notist::Context::default(),
        )
        .unwrap()
        .into_parts();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].message.contains("unknown constructor"));
    let dump = notist::dump::dump(&item);
    assert_eq!(dump.matches("(paragraph").count(), 1, "{dump}");
}
#[test]
fn shape_groups_sections_by_heading_level() {
    let src = "序言\n\n= 一\n\n内容\n\n== 一点一\n\n细节\n\n= 二\n";
    let (item, diags) = notist::Pipeline::default()
        .analyze("test.not", src, &notist::Context::default())
        .unwrap()
        .into_parts();
    assert!(diags.is_empty());
    let out = notist::dump::dump(&item);
    assert_eq!(
        out,
        "\
(doc @0..51
  (paragraph @0..6
    (text @0..6 :text \"序言\")
  )
  (section @8..43
    (heading @8..13 :level 1
      (text @10..13 :text \"一\")
    )
    (paragraph @15..21
      (text @15..21 :text \"内容\")
    )
    (section @23..43
      (heading @23..35 :level 2
        (text @26..35 :text \"一点一\")
      )
      (paragraph @37..43
        (text @37..43 :text \"细节\")
      )
    )
  )
  (section @45..50
    (heading @45..50 :level 1
      (text @47..50 :text \"二\")
    )
  )
)
"
    );
}

#[test]
fn shape_recurses_into_block_mounts() {
    let src = "#callout[\n前\n\n#divider()\n\n后\n\n= 节\n]\n";
    let analysis = notist::Pipeline::default()
        .analyze("test.not", src, &notist::Context::default())
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let callout = &analysis.root().children[0];
    assert_eq!(callout.ctor, notist::Ctor::Callout);
    assert_eq!(
        callout
            .children
            .iter()
            .map(|item| &item.ctor)
            .collect::<Vec<_>>(),
        [
            &notist::Ctor::Paragraph,
            &notist::Ctor::Divider,
            &notist::Ctor::Paragraph,
            &notist::Ctor::Section
        ]
    );
}

#[test]
fn shape_transfers_heading_attrs_to_section() {
    // heading 的注解转移到其开启的 section 上（一个 heading 恰开启一个 section）
    let src = "@(id: \"intro\", tags: (\"a\",))\n= 一\n\n内容\n";
    let (item, diags) = notist::Pipeline::default()
        .analyze("test.not", src, &notist::Context::default())
        .unwrap()
        .into_parts();
    assert!(diags.is_empty());
    let out = notist::dump::dump(&item);
    assert_eq!(
        out,
        "\
(doc @0..43
  (section @29..42 @id \"intro\" @tags (\"a\")
    (heading @29..34 :level 1
      (text @31..34 :text \"一\")
    )
    (paragraph @36..42
      (text @36..42 :text \"内容\")
    )
  )
)
"
    );
}
