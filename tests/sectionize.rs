#[test]
fn sectionize_groups_by_heading_level() {
    let src = "序言\n\n= 一\n\n内容\n\n== 一点一\n\n细节\n\n= 二\n";
    let (item, diags) = notist::analyze(src);
    assert!(diags.is_empty());
    let doc = notist::sectionize::sectionize(&item);
    let out = notist::dump::dump(&doc);
    assert_eq!(
        out,
        "\
(doc @0..51
  (paragraph @0..6
    (text @0..6 :text \"序言\")
  )
  (section @8..13
    (heading @8..13 :level 1
      (text @10..13 :text \"一\")
    )
    (paragraph @15..21
      (text @15..21 :text \"内容\")
    )
    (section @23..35
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
