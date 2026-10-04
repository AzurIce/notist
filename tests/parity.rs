use notist::{Ctor, Item, Notist, TextRange, Value};

fn analyze(extension: &str, src: &str) -> Item {
    if extension == "not" {
        let parse = notist::syntax::parser::parse(src);
        assert_eq!(parse.syntax().to_string(), src, "lossless CST: {src:?}");
    }
    let document = Notist::default()
        .analyze(format!("test.{extension}"), src)
        .unwrap();
    assert!(
        document.diagnostics().is_empty(),
        "{extension}: {src:?}: {:?}",
        document.diagnostics()
    );
    document.into_parts().0
}

fn normalize(mut item: Item) -> Item {
    item.span = TextRange::empty(0.into());
    item.children = item.children.into_iter().map(normalize).collect();
    item
}

fn equal(not: &str, md: &str) {
    assert_eq!(
        normalize(analyze("not", not)),
        normalize(analyze("md", md)),
        "{not:?} / {md:?}"
    );
}

#[test]
fn lists_share_block_content_across_sugar_functions_and_frontends() {
    for (not, md) in [
        ("- xxx\n  continued\n", "- xxx\n  continued\n"),
        (
            "- first\n\n  second\n- next\n",
            "- first\n\n  second\n- next\n",
        ),
        ("- first\n\n- second\n", "- first\n\n- second\n"),
        ("+ first\n+ second\n", "1. first\n2. second\n"),
        (
            "- first\n  - child\n  after\n- next\n",
            "- first\n  - child\n\n  after\n- next\n",
        ),
        (
            "- first\n  - child\n\n  after\n- next\n",
            "- first\n  - child\n\n  after\n- next\n",
        ),
        (
            "- first\n  - child\n    + grandchild\n  - child2\n- next\n",
            "- first\n  - child\n    1. grandchild\n  - child2\n- next\n",
        ),
        (
            "- first\n\n  ```rust\n  x\n\n  y\n  ```\n- next\n",
            "- first\n\n  ```rust\n  x\n\n  y\n  ```\n- next\n",
        ),
        (
            "- first\n\n  | a | b |\n  | - | - |\n  | x | y |\n- next\n",
            "- first\n\n  | a | b |\n  | - | - |\n  | x | y |\n- next\n",
        ),
        (
            "- = heading\n  body\n- next\n",
            "- # heading\n  body\n- next\n",
        ),
        (
            "- *first\n  continued*\n- next\n",
            "- **first\n  continued**\n- next\n",
        ),
        (
            "- first\\\n  second\n- next\n",
            "- first\\\n  second\n- next\n",
        ),
        (
            "- first\r\n\r\n  second\r\n- next\r\n",
            "- first\r\n\r\n  second\r\n- next\r\n",
        ),
        (
            "-\tfirst\n\tcontinued\n-\tsecond\n",
            "-\tfirst\n\tcontinued\n-\tsecond\n",
        ),
        (
            "-\tfirst\n\n\t```\n\tx\n\t```\n",
            "-\tfirst\n\n\t```\n\tx\n\t```\n",
        ),
        ("- $a\n  b$\n- next\n", "- $a\n  b$\n- next\n"),
        ("- ", "- "),
    ] {
        equal(not, md);
    }
    let sugar = analyze("not", "- first\n\n  second\n- next");
    let explicit = analyze(
        "not",
        "#list(ordered: false, start: 1)[\n#item[\nfirst\n\nsecond\n]\n#item[next]\n]",
    );
    assert_eq!(normalize(sugar), normalize(explicit));
    equal(
        "#list(ordered: true, start: 3)[#item[first] #item[second]]",
        "3. first\n4. second\n",
    );
    assert_eq!(
        normalize(analyze("not", "- ")),
        normalize(analyze("not", "#list[#item[]]"))
    );
}

#[test]
fn list_boundaries_types_and_inline_pairs_are_preserved() {
    let root = analyze("not", "- bullet\n+ numbered\n- bullet2\n");
    assert_eq!(root.children.len(), 3);
    for (list, ordered) in root.children.iter().zip([false, true, false]) {
        assert_eq!(list.ctor, Ctor::List);
        assert_eq!(list.fields.get("ordered"), Some(&Value::Bool(ordered)));
        assert_eq!(list.fields.get("start"), Some(&Value::Int(1)));
        assert!(
            list.children
                .iter()
                .all(|child| child.ctor == Ctor::ListItem)
        );
    }
    let root = analyze("not", "- first\n\noutside\n\n- next\n");
    assert_eq!(
        root.children
            .iter()
            .map(|node| &node.ctor)
            .collect::<Vec<_>>(),
        [&Ctor::List, &Ctor::Paragraph, &Ctor::List]
    );
    let root = analyze("not", "- *first\n- second*\n");
    assert!(root.find(|node| node.ctor == Ctor::Strong).is_none());
    let root = analyze("not", "#callout[\n- first\n\n  second\n- next\n]");
    assert_eq!(root.children[0].children[0].children[0].children.len(), 2);
    let root = analyze("not", "- first\n\n  @(id: \"detail\")\n  second\n- next\n");
    assert_eq!(
        notist::query::select(&root, "id:detail")[0].ctor,
        Ctor::Paragraph
    );
    let root = analyze(
        "not",
        "- #callout[\n  - first\n\n    second\n  - next\n  ]\n- outside\n",
    );
    let callout = root.find(|node| node.ctor == Ctor::Callout).unwrap();
    assert_eq!(callout.children.len(), 1);
    assert_eq!(callout.children[0].ctor, Ctor::List);
    assert_eq!(callout.children[0].children.len(), 2);
    assert_eq!(callout.children[0].children[0].children.len(), 2);
}

#[test]
fn text_links_and_raw_payloads_are_normalized_once() {
    equal(r"A & B \*literal\*", r"A &amp; B \*literal\*");
    equal(r"A &amp; &amp;", r"A \&amp; &#38;amp;");
    equal("*A & B* [C & D](asset)", "**A &amp; B** [C &amp; D](asset)");
    for (not, md, target) in [
        (
            "[label](asset(1).pdf)",
            "[label](asset(1).pdf)",
            "asset(1).pdf",
        ),
        (
            r"[label](asset\(1\).pdf)",
            r"[label](asset\(1\).pdf)",
            "asset(1).pdf",
        ),
        (
            "[label](<asset(1).pdf>)",
            "[label](<asset(1).pdf>)",
            "asset(1).pdf",
        ),
        (
            "[label](asset?a=1&b=2)",
            "[label](asset?a=1&amp;b=2)",
            "asset?a=1&b=2",
        ),
    ] {
        equal(not, md);
        assert_eq!(
            analyze("not", not)
                .find(|node| node.ctor == Ctor::Link)
                .unwrap()
                .fields
                .get("target"),
            Some(&Value::Str(target.into()))
        );
    }
    equal("[label](asset)", "[label](asset \"ignored\")");
    for src in [
        "```rust\nx\n```\n",
        "```\nx  \n\n```\n",
        "```\n```\n",
        "```rust\nx\r\n```\r\n",
    ] {
        equal(src, src);
    }
    let root = analyze(
        "md",
        "`&amp; \\*x\\*` $&amp; \\alpha$\n\n```\n&amp; \\*x\\*\n```\n",
    );
    assert_eq!(
        root.find(|node| node.ctor == Ctor::Math)
            .unwrap()
            .fields
            .get("text"),
        Some(&Value::Str(r"&amp; \alpha".into()))
    );
    assert_eq!(
        root.find(|node| node.ctor == Ctor::RawInline)
            .unwrap()
            .fields
            .get("text"),
        Some(&Value::Str(r"&amp; \*x\*".into()))
    );
}

#[test]
fn invalid_builtin_contracts_report_diagnostics_and_retain_values() {
    for (src, message) in [
        ("#list[text]", "only item children"),
        ("#list[#paragraph[text]]", "only item children"),
        (
            "#strong[before #callout[note] after]",
            "block children are not allowed",
        ),
        (
            "#link(\"asset\")[#[#callout[note]]]",
            "block children are not allowed",
        ),
        ("#heading(level: \"bad\")[heading]", "positive integer"),
        ("#heading(level: 0)[heading]", "positive integer"),
        ("#link(target: 42)[label]", "must be string"),
        ("#embed()", "requires `target`"),
        ("#raw()", "requires `text`"),
        ("#list(ordered: \"yes\")[#item[x]]", "must be boolean"),
        ("#list(start: \"three\")[#item[x]]", "must be integer"),
        (
            "#table(align: (\"bad\",))[#row[#cell[x]]]",
            "alignment array",
        ),
        ("#table[#row(header: 1)[#cell[x]]]", "must be boolean"),
    ] {
        let document = Notist::default().analyze("test.not", src).unwrap();
        assert!(
            document
                .diagnostics()
                .iter()
                .any(|diag| diag.message.contains(message)),
            "{src}: {:?}",
            document.diagnostics()
        );
    }
    let document = Notist::default()
        .analyze("test.not", "#link(target: 42)[label]")
        .unwrap();
    assert_eq!(
        document
            .root()
            .find(|node| node.ctor == Ctor::Link)
            .unwrap()
            .fields
            .get("target"),
        Some(&Value::Int(42))
    );
}

#[test]
fn reflow_keeps_one_identity_for_an_annotated_split_range() {
    let src = "@(id: \"keep\", tags: (\"range\",))\nbefore #callout[note] after";
    let root = analyze("not", src);
    let matches = notist::query::select(&root, "id:keep");
    assert_eq!(matches.len(), 1);
    let group = matches[0];
    assert_eq!(group.ctor, Ctor::Group);
    assert_eq!(
        group
            .children
            .iter()
            .map(|node| &node.ctor)
            .collect::<Vec<_>>(),
        [&Ctor::Paragraph, &Ctor::Callout, &Ctor::Paragraph]
    );
    assert_eq!(
        &src[usize::from(group.span.start())..usize::from(group.span.end())],
        "before #callout[note] after"
    );
    assert_eq!(notist::query::select(&root, "tag:range").len(), 1);
    for child in &group.children {
        assert!(child.span.start() >= group.span.start() && child.span.end() <= group.span.end());
    }
}
