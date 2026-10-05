#[test]
fn reflow_splits_paragraphs_around_block_calls() {
    // 未知构造器诊断 + 恢复：block flavor 把段落切成兄弟
    let (item, diags) = notist::Pipeline::default()
        .analyze(
            "test.not",
            "文字\n#note[\n块 body\n]\n后续\n",
            notist::builtins::registry(),
        )
        .unwrap()
        .into_parts();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].message.contains("unknown constructor"));
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
    let (item, diags) = notist::Pipeline::default()
        .analyze(
            "test.not",
            "文字 #note[x] 后续\n",
            notist::builtins::registry(),
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
        .analyze("test.not", src, notist::builtins::registry())
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
    // block body 内的段落候选同样切分、各自成节
    let src = "#note[\n前 #item[x] 后\n\n= 节\n]\n";
    let (item, diags) = notist::Pipeline::default()
        .analyze("test.not", src, notist::builtins::registry())
        .unwrap()
        .into_parts();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].message.contains("unknown constructor"));
    let out = notist::dump::dump(&item);
    assert_eq!(
        out,
        "\
(doc @0..33
  (note @0..32
    (paragraph @7..11
      (text @7..11 :text \"前 \")
    )
    (listitem @11..19
      (paragraph @11..19
        (text @17..18 :text \"x\")
      )
    )
    (paragraph @19..23
      (text @19..23 :text \" 后\")
    )
    (section @25..30
      (heading @25..30 :level 1
        (text @27..30 :text \"节\")
      )
    )
  )
)
"
    );
}

#[test]
fn shape_transfers_heading_attrs_to_section() {
    // heading 的注解转移到其开启的 section 上（一个 heading 恰开启一个 section）
    let src = "@(id: \"intro\", tags: (\"a\",))\n= 一\n\n内容\n";
    let (item, diags) = notist::Pipeline::default()
        .analyze("test.not", src, notist::builtins::registry())
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
