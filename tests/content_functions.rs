use notist::builtins::{Accepts, Level};
use notist::{Ctor, Item, Pipeline, TextRange, Value};

fn analyze(src: &str) -> Item {
    let parse = notist::syntax::parser::parse_document(src);
    assert!(
        parse.diagnostics.is_empty(),
        "{src:?}: {:?}",
        parse.diagnostics
    );
    assert_eq!(parse.syntax().to_string(), src, "CST must stay lossless");
    let document = Pipeline::default()
        .analyze("test.not", src, notist::builtins::registry())
        .unwrap();
    assert!(
        document.diagnostics().is_empty(),
        "{src:?}: {:?}",
        document.diagnostics()
    );
    document.into_parts().0
}

fn text_of(item: &Item) -> String {
    item.descendants()
        .filter(|node| node.ctor == Ctor::Text)
        .filter_map(|node| match node.fields.get("text") {
            Some(Value::Str(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn strike_sugar_and_function_support_nested_inline_content_and_attrs() {
    for src in [
        "@(id: \"removed\")~*粗体* _强调_ [链接](target)~",
        "@(id: \"removed\")#strike[*粗体* _强调_ [链接](target)]",
    ] {
        let root = analyze(src);
        let strike = root.find(|node| node.ctor == Ctor::Strike).unwrap();
        assert_eq!(strike.attrs.get("id"), Some(&Value::Str("removed".into())));
        assert_eq!(strike.ctor.level(), Some(Level::Inline));
        assert_eq!(strike.ctor.accepts(), Some(Accepts::Inline));
        assert_eq!(text_of(strike), "粗体 强调 链接");
        assert!(strike.find(|node| node.ctor == Ctor::Strong).is_some());
        assert!(strike.find(|node| node.ctor == Ctor::Emph).is_some());
        assert!(strike.find(|node| node.ctor == Ctor::Link).is_some());
    }
    assert_eq!(text_of(&analyze("~甲\n乙~")), "甲乙");
}

#[test]
fn strike_delimiters_remain_literal_when_escaped_unpaired_or_in_payloads() {
    for src in [
        "\\~literal\\~",
        "~ spaced~",
        "~spaced ~",
        "~~",
        "~unclosed",
        "~first\n\nsecond~",
        "~first\n---\nsecond~",
        "`~code~`",
        "$~math~$",
    ] {
        let root = analyze(src);
        assert!(
            root.find(|node| node.ctor == Ctor::Strike).is_none(),
            "{src:?}"
        );
    }
    assert_eq!(text_of(&analyze("\\~literal\\~")), "~literal~");
}

#[test]
fn divider_splits_blocks_without_blank_lines_and_carries_attrs() {
    let src = "前文\n@(id: \"break\")\n--- \t\r\n后文\n#divider()\n";
    let root = analyze(src);
    let ctors: Vec<_> = root.children.iter().map(|node| &node.ctor).collect();
    assert_eq!(
        ctors,
        [
            &Ctor::Paragraph,
            &Ctor::Divider,
            &Ctor::Paragraph,
            &Ctor::Divider
        ]
    );
    let divider = &root.children[1];
    assert_eq!(divider.attrs.get("id"), Some(&Value::Str("break".into())));
    assert_eq!(
        &src[usize::from(divider.span.start())..usize::from(divider.span.end())],
        "---"
    );
    assert_eq!(divider.ctor.level(), Some(Level::Block));
    assert_eq!(divider.ctor.accepts(), Some(Accepts::Nothing));
    assert!(divider.children.is_empty());

    let root = analyze("前\n---\n后");
    assert_eq!(root.children.len(), 3);
    assert_eq!(root.children[1].ctor, Ctor::Divider);
    let root = analyze("- 第一项\n---\n- 第二项");
    assert_eq!(
        root.children
            .iter()
            .map(|node| &node.ctor)
            .collect::<Vec<_>>(),
        [&Ctor::List, &Ctor::Divider, &Ctor::List]
    );
    assert_eq!(analyze("---").children[0].ctor, Ctor::Divider);
}

#[test]
fn divider_requires_exactly_three_dashes_on_its_own_line() {
    for src in [
        "--",
        "----",
        " ---",
        "--- text",
        "a---b",
        "- - -",
        "/*comment*/---",
        "\\---",
    ] {
        let root = analyze(src);
        assert!(
            root.find(|node| node.ctor == Ctor::Divider).is_none(),
            "{src:?}"
        );
    }
    let json = notist::cst_json::analyze_json("before\n---\nafter");
    assert!(json.contains("\"kind\":\"Divider\",\"start\":7,\"end\":10"));
}

#[test]
fn callout_mounts_are_shaped_recursively() {
    let root = analyze("#callout(kind: \"note\")[\n= 小节\n前文\n---\n~后文~\n]");
    let callout = &root.children[0];
    assert_eq!(callout.ctor, Ctor::Callout);
    assert_eq!(callout.fields.get("kind"), Some(&Value::Str("note".into())));
    assert_eq!(callout.ctor.level(), Some(Level::Block));
    assert_eq!(callout.ctor.accepts(), Some(Accepts::Content));
    let section = &callout.children[0];
    assert_eq!(section.ctor, Ctor::Section);
    assert_eq!(
        section
            .children
            .iter()
            .map(|node| &node.ctor)
            .collect::<Vec<_>>(),
        [
            &Ctor::Heading,
            &Ctor::Paragraph,
            &Ctor::Divider,
            &Ctor::Paragraph
        ]
    );
    assert!(section.find(|node| node.ctor == Ctor::Strike).is_some());

    let root = analyze("前 #callout[提示] 后");
    assert_eq!(
        root.children
            .iter()
            .map(|node| &node.ctor)
            .collect::<Vec<_>>(),
        [&Ctor::Paragraph, &Ctor::Callout, &Ctor::Paragraph]
    );
    assert_eq!(root.children[1].children[0].ctor, Ctor::Paragraph);
}

#[test]
fn new_functions_report_invalid_children_mounts() {
    for (src, message) in [
        ("#divider[x]", "`divider` takes no children"),
        ("#strike[ padded ]", "`strike` takes inline content"),
    ] {
        let document = Pipeline::default()
            .analyze("test.not", src, notist::builtins::registry())
            .unwrap();
        assert!(
            document
                .diagnostics()
                .iter()
                .any(|d| d.message.contains(message)),
            "{src}"
        );
    }
}

fn without_spans(mut item: Item) -> Item {
    item.span = TextRange::empty(0.into());
    item.children = item.children.into_iter().map(without_spans).collect();
    item
}

#[test]
fn markdown_and_notist_produce_the_same_callout_strike_and_divider_ir() {
    let engine = Pipeline::default();
    let not = engine
        .analyze(
            "test.not",
            "#callout(kind: \"quote\")[~删除~]\n\n---\n",
            notist::builtins::registry(),
        )
        .unwrap();
    let md = engine
        .analyze(
            "test.md",
            "> ~~删除~~\n\n---\n",
            notist::builtins::registry(),
        )
        .unwrap();
    assert!(not.diagnostics().is_empty());
    assert!(md.diagnostics().is_empty(), "{:?}", md.diagnostics());
    assert_eq!(
        without_spans(not.into_parts().0),
        without_spans(md.into_parts().0)
    );
}

#[test]
fn markdown_html_is_ignored_while_text_and_code_are_preserved() {
    let src = "alpha <em>beta</em> gamma\n\n<div>\nskipped\n</div>\n\nomega\n\n`<span>`\n";
    let doc = Pipeline::default()
        .analyze("test.md", src, notist::builtins::registry())
        .unwrap();
    assert!(doc.diagnostics().is_empty(), "{:?}", doc.diagnostics());
    assert_eq!(text_of(doc.root()), "alpha beta gammaomega");
    assert!(
        doc.root()
            .find(|node| matches!(node.ctor, Ctor::Custom(_)))
            .is_none()
    );
    let raw = doc
        .root()
        .find(|node| node.ctor == Ctor::RawInline)
        .unwrap();
    assert_eq!(raw.fields.get("text"), Some(&Value::Str("<span>".into())));

    let doc = Pipeline::default()
        .analyze(
            "test.md",
            "<div>\nskipped\n</div>\n",
            notist::builtins::registry(),
        )
        .unwrap();
    assert!(doc.diagnostics().is_empty());
    assert!(doc.root().children.is_empty());
}

#[test]
fn markdown_math_matches_notist_and_preserves_opaque_payload_and_span() {
    let engine = Pipeline::default();
    for src in [
        "前 $x^2 + y_1$ 后",
        "$*bold* [link](x) ~strike~ <tag> \\alpha$",
        "$甲\n乙$",
        "$x\\$y$",
    ] {
        let not = engine
            .analyze("test.not", src, notist::builtins::registry())
            .unwrap();
        let md = engine
            .analyze("test.md", src, notist::builtins::registry())
            .unwrap();
        assert!(not.diagnostics().is_empty(), "{src:?}");
        assert!(md.diagnostics().is_empty(), "{src:?}");
        assert_eq!(
            without_spans(not.into_parts().0),
            without_spans(md.into_parts().0),
            "{src:?}"
        );
    }

    let src = "前 $x$ 后";
    let md = engine
        .analyze("test.md", src, notist::builtins::registry())
        .unwrap();
    let math = md.root().find(|node| node.ctor == Ctor::Math).unwrap();
    assert_eq!(math.fields.get("text"), Some(&Value::Str("x".into())));
    assert_eq!(
        &src[usize::from(math.span.start())..usize::from(math.span.end())],
        "$x$"
    );
    assert!(math.children.is_empty());

    let md = engine
        .analyze(
            "test.md",
            "**粗 $x$** [链接 $y$](target)\n\n> $z$\n\n- $w$",
            notist::builtins::registry(),
        )
        .unwrap();
    assert!(md.diagnostics().is_empty());
    assert_eq!(
        md.root()
            .descendants()
            .filter(|n| n.ctor == Ctor::Math)
            .count(),
        4
    );
}

#[test]
fn padded_math_is_block_content_and_matches_explicit_calls() {
    let engine = Pipeline::default();
    for (sugar, function) in [
        ("$ x $", "#math(\"x\", block: true)"),
        ("$ \t x + y \t $", "#math(block: true, text: \"x + y\")"),
        (
            "前 $x$ 中 $ x^2 $ 后",
            "前 #math(\"x\") 中 #math(\"x^2\", block: true) 后",
        ),
        (
            r#"$ *bold* [link](x) #raw("x") \alpha $"#,
            r##"#math(r#"*bold* [link](x) #raw("x") \alpha"#, block: true)"##,
        ),
    ] {
        let explicit = without_spans(analyze(function));
        for path in ["test.not", "test.md"] {
            let doc = engine
                .analyze(path, sugar, notist::builtins::registry())
                .unwrap();
            assert!(doc.diagnostics().is_empty(), "{path}: {sugar:?}");
            let equation = doc
                .root()
                .find(|n| n.ctor == Ctor::Math && n.level == Level::Block)
                .unwrap();
            assert_eq!(equation.level, Level::Block);
            assert_eq!(equation.ctor.accepts(), Some(Accepts::Nothing));
            assert!(equation.children.is_empty());
            assert_eq!(
                without_spans(doc.into_parts().0),
                explicit,
                "{path}: {sugar:?}"
            );
        }
    }
    let root = analyze("前 $x$ 中 $ x^2 $ 后");
    assert_eq!(
        root.children.iter().map(|n| &n.ctor).collect::<Vec<_>>(),
        [&Ctor::Paragraph, &Ctor::Math, &Ctor::Paragraph]
    );
    assert_eq!(root.children[0].children[1].ctor, Ctor::Math);
}

#[test]
fn multiline_block_math_keeps_payload_container_indentation_and_source_spans() {
    let engine = Pipeline::default();
    for newline in ["\n", "\r\n", "\r"] {
        let source = format!("- before{newline}  $ {newline}  x +{newline}    y{newline}  $ after");
        let root = analyze(&source);
        let item = root.find(|n| n.ctor == Ctor::ListItem).unwrap();
        let equation = &item.children[1];
        assert_eq!(equation.ctor, Ctor::Math);
        assert_eq!(
            equation.fields.get("text"),
            Some(&Value::Str(format!("x +{newline}  y")))
        );
        assert_eq!(item.children[0].ctor, Ctor::Paragraph);
        assert_eq!(item.children[2].ctor, Ctor::Paragraph);
        assert_eq!(
            &source[usize::from(equation.span.start())..usize::from(equation.span.end())],
            format!("$ {newline}  x +{newline}    y{newline}  $")
        );
        if newline == "\n" {
            let md = engine
                .analyze("test.md", &source, notist::builtins::registry())
                .unwrap();
            assert!(md.diagnostics().is_empty(), "{:?}", md.diagnostics());
            let item = md.root().find(|n| n.ctor == Ctor::ListItem).unwrap();
            assert_eq!(
                item.children.iter().map(|n| &n.ctor).collect::<Vec<_>>(),
                [&Ctor::Paragraph, &Ctor::Math, &Ctor::Paragraph]
            );
            // Markdown paragraphs remove continuation-line leading spaces.
            assert_eq!(
                item.children[1].fields.get("text"),
                Some(&Value::Str("x +\ny".into()))
            );
            assert_eq!(item.children[1].span, equation.span);
        }
    }
}

#[test]
fn empty_mismatched_escaped_or_unclosed_math_remains_literal() {
    let engine = Pipeline::default();
    for source in [
        "$$",
        "$  $",
        "$\t$",
        "$ x$",
        "$x $",
        "$ x",
        "$ \n\nx $",
        "$ x\n---\ny $",
        "\\$ x \\$",
        "`$ x $`",
    ] {
        for path in ["test.not", "test.md"] {
            let doc = engine
                .analyze(path, source, notist::builtins::registry())
                .unwrap();
            assert!(doc.diagnostics().is_empty(), "{path}: {source:?}");
            assert!(
                doc.root().find(|n| n.ctor == Ctor::Math).is_none(),
                "{path}: {source:?}"
            );
        }
    }
}

#[test]
fn markdown_math_does_not_parse_escapes_code_or_unpaired_delimiters() {
    let engine = Pipeline::default();
    for src in [
        "\\$literal\\$",
        "`$code$`",
        "```\n$code$\n```",
        "$ spaced$",
        "$spaced $",
        "$$",
        "$unclosed",
        "$first\n\nsecond$",
        "$first\n---\nsecond$",
        "[label](https://example.com/$target$)",
    ] {
        let md = engine
            .analyze("test.md", src, notist::builtins::registry())
            .unwrap();
        assert!(md.diagnostics().is_empty(), "{src:?}");
        assert!(
            md.root().find(|n| n.ctor == Ctor::Math).is_none(),
            "{src:?}"
        );
    }
}

#[test]
fn markdown_line_breaks_match_notist_and_preserve_inline_spaces() {
    let engine = Pipeline::default();
    for (not_src, md_src) in [
        ("甲\n乙", "甲\n乙"),
        ("alpha \n beta", "alpha \n beta"),
        ("甲\r\n乙", "甲\r\n乙"),
        ("甲\\\n乙", "甲\\\n乙"),
        ("甲\\\r\n乙", "甲\\\r\n乙"),
        ("甲\\\n乙\\\n丙", "甲\\\n乙\\\n丙"),
        ("*甲*\\\n*乙*", "**甲**\\\n**乙**"),
        ("甲\n\n乙", "甲\n\n乙"),
        ("甲\n \t\n\n乙", "甲\n \t\n\n乙"),
        ("甲  \n乙", "甲  \n乙"),
        ("*甲*\n*乙*", "**甲**\n**乙**"),
        ("*甲* *乙*", "**甲** **乙**"),
        ("$甲$ $乙$", "$甲$ $乙$"),
        ("*甲\n乙*", "**甲\n乙**"),
    ] {
        let not = engine
            .analyze("test.not", not_src, notist::builtins::registry())
            .unwrap();
        let md = engine
            .analyze("test.md", md_src, notist::builtins::registry())
            .unwrap();
        assert!(not.diagnostics().is_empty());
        assert!(md.diagnostics().is_empty());
        assert_eq!(
            without_spans(not.into_parts().0),
            without_spans(md.into_parts().0),
            "{not_src:?} / {md_src:?}"
        );
    }
    let md = engine
        .analyze("test.md", "甲\\\n乙\n\n丙", notist::builtins::registry())
        .unwrap();
    assert_eq!(text_of(md.root()), "甲乙丙");
    assert_eq!(md.root().children.len(), 3);

    let md = engine
        .analyze(
            "test.md",
            "```\n甲\n乙\n```\n",
            notist::builtins::registry(),
        )
        .unwrap();
    let raw = md.root().find(|n| n.ctor == Ctor::RawInline).unwrap();
    assert_eq!(raw.fields.get("text"), Some(&Value::Str("甲\n乙\n".into())));
}

#[test]
fn notist_embed_sugar_matches_function_and_markdown() {
    let engine = Pipeline::default();
    for target in [
        "picture.png",
        "clip.mp4",
        "paper.pdf",
        "other.not",
        "no-extension",
    ] {
        let sugar = analyze(&format!("![资源]({target} \"说明\")"));
        let function = analyze(&format!(
            "#embed(target: \"{target}\", description: \"资源\", title: \"说明\")"
        ));
        let md = engine
            .analyze(
                "test.md",
                &format!("![资源]({target} \"说明\")"),
                notist::builtins::registry(),
            )
            .unwrap();
        assert_eq!(without_spans(sugar.clone()), without_spans(function));
        assert_eq!(without_spans(sugar), without_spans(md.into_parts().0));
    }
    for src in [
        "![](asset)",
        "![](asset \"\")",
        "![说明](<asset(1).pdf> '标题')",
    ] {
        let root = analyze(src);
        let embed = root.find(|node| node.ctor == Ctor::Embed).unwrap();
        assert!(embed.children.is_empty());
    }
}

#[test]
fn notist_embed_sugar_preserves_nested_description_attrs_and_spans() {
    let src = r#"前 @(id: "resource")![前 *粗* _斜_ ~删~ `code` [链接](somewhere) $x$ \] 后](asset\(1\).pdf "说明\"文字") 后"#;
    let root = analyze(src);
    let embed = root.find(|node| node.ctor == Ctor::Embed).unwrap();
    assert_eq!(
        embed.fields.get("target"),
        Some(&Value::Str("asset(1).pdf".into()))
    );
    assert_eq!(
        embed.fields.get("description"),
        Some(&Value::Str("前 粗 斜 删 code 链接 x ] 后".into()))
    );
    assert_eq!(
        embed.fields.get("title"),
        Some(&Value::Str("说明\"文字".into()))
    );
    assert_eq!(embed.attrs.get("id"), Some(&Value::Str("resource".into())));
    assert!(src[usize::from(embed.span.start())..usize::from(embed.span.end())].starts_with("!["));
    assert_eq!(text_of(&root), "前  后");

    let table = analyze("| 资源 |\n| --- |\n| ![*描述*](asset) |\n");
    assert_eq!(
        table
            .find(|node| node.ctor == Ctor::Embed)
            .unwrap()
            .fields
            .get("description"),
        Some(&Value::Str("描述".into()))
    );
    let link = analyze("[![说明](asset)](destination)");
    let link = link.find(|node| node.ctor == Ctor::Link).unwrap();
    assert_eq!(link.children[0].ctor, Ctor::Embed);
    assert_eq!(
        link.fields.get("target"),
        Some(&Value::Str("destination".into()))
    );
    let root = analyze("![计算 (A) `]` $[$](Bob's(1).pdf \"含 ) 的标题\")");
    let embed = root.find(|node| node.ctor == Ctor::Embed).unwrap();
    assert_eq!(
        embed.fields.get("target"),
        Some(&Value::Str("Bob's(1).pdf".into()))
    );
    assert_eq!(
        embed.fields.get("description"),
        Some(&Value::Str("计算 (A) ] [".into()))
    );
    assert_eq!(
        embed.fields.get("title"),
        Some(&Value::Str("含 ) 的标题".into()))
    );
}

#[test]
fn malformed_or_escaped_embeds_remain_literal_and_lossless() {
    for src in [
        "![unclosed",
        "![text]",
        "![text](unclosed",
        "![text](first\nsecond)",
        "![first\n\nsecond](asset)",
        r"\![text](asset)",
        "`![text](asset)`",
        "$![text](asset)$",
    ] {
        assert!(
            analyze(src).find(|node| node.ctor == Ctor::Embed).is_none(),
            "{src:?}"
        );
    }
}

#[test]
fn explicit_breaks_preserve_containers_and_opaque_payloads() {
    let engine = Pipeline::default();
    let md = engine
        .analyze("test.md", "> 甲\\\n> 乙\n", notist::builtins::registry())
        .unwrap();
    let quote = md.root().find(|node| node.ctor == Ctor::Callout).unwrap();
    assert_eq!(quote.children.len(), 2);
    assert_eq!(text_of(&quote.children[0]), "甲");
    assert_eq!(text_of(&quote.children[1]), "乙");
    let md = engine
        .analyze("test.md", "- 甲\\\n  乙\n", notist::builtins::registry())
        .unwrap();
    assert_eq!(
        md.root()
            .find(|node| node.ctor == Ctor::ListItem)
            .unwrap()
            .children
            .len(),
        2
    );
    for src in [
        "$甲\\\n乙$",
        "```\n甲\\\n乙\n```",
        "`甲\\\n乙`",
        "甲\\\\\n乙",
    ] {
        let md = engine
            .analyze("test.md", src, notist::builtins::registry())
            .unwrap();
        assert_eq!(md.root().children.len(), 1, "{src:?}");
    }
    for (extension, src) in [("not", "*甲\\\n乙*"), ("md", "**甲\\\n乙**")] {
        let root = engine
            .analyze(
                &format!("test.{extension}"),
                src,
                notist::builtins::registry(),
            )
            .unwrap();
        assert_eq!(root.root().children.len(), 2);
        assert!(root.root().find(|node| node.ctor == Ctor::Strong).is_none());
    }
}

#[test]
fn embed_function_and_markdown_share_fields_for_every_resource_type() {
    let engine = Pipeline::default();
    for target in [
        "assets/image.png",
        "assets/movie.mp4",
        "assets/audio.ogg",
        "assets/paper.pdf#page=3",
        "notes/example.not#section",
        "https://example.com/resource?id=42",
        "missing-file",
    ] {
        let not_src =
            format!("#embed(target: \"{target}\", description: \"资源\", title: \"说明\")");
        let md_src = format!("![资源]({target} \"说明\")");
        let not = analyze(&not_src);
        let md = engine
            .analyze("test.md", &md_src, notist::builtins::registry())
            .unwrap();
        assert!(
            md.diagnostics().is_empty(),
            "{target}: {:?}",
            md.diagnostics()
        );
        assert_eq!(
            without_spans(not),
            without_spans(md.root().clone()),
            "{target}"
        );

        let embed = md.root().find(|node| node.ctor == Ctor::Embed).unwrap();
        assert_eq!(embed.fields.get("target"), Some(&Value::Str(target.into())));
        assert_eq!(
            embed.fields.get("description"),
            Some(&Value::Str("资源".into()))
        );
        assert_eq!(embed.fields.get("title"), Some(&Value::Str("说明".into())));
        assert_eq!(embed.fields.iter().count(), 3);
        assert!(embed.children.is_empty());
        assert_eq!(embed.ctor.level(), Some(Level::Inline));
        assert_eq!(embed.ctor.accepts(), Some(Accepts::Nothing));

        let positional = analyze(&format!("#embed(\"{target}\")"));
        let embed = positional.find(|node| node.ctor == Ctor::Embed).unwrap();
        assert_eq!(embed.fields.get("target"), Some(&Value::Str(target.into())));
    }
}

#[test]
fn markdown_embed_keeps_complete_plain_description_and_reference_title() {
    let engine = Pipeline::default();
    let md = engine
        .analyze(
            "test.md",
            "![前 **粗** _斜_ `code` [链接](somewhere) $x$ 后](asset)\n\n![引用][resource]\n\n[resource]: clip.mp4 \"说明\"\n\n![](empty)\n", notist::builtins::registry())
        .unwrap();
    assert!(md.diagnostics().is_empty(), "{:?}", md.diagnostics());
    let embeds: Vec<_> = md
        .root()
        .descendants()
        .filter(|n| n.ctor == Ctor::Embed)
        .collect();
    assert_eq!(embeds.len(), 3);
    assert_eq!(
        embeds[0].fields.get("description"),
        Some(&Value::Str("前 粗 斜 code 链接 x 后".into()))
    );
    assert_eq!(
        embeds[0].fields.get("target"),
        Some(&Value::Str("asset".into()))
    );
    assert!(embeds[0].fields.get("title").is_none());
    assert_eq!(
        embeds[1].fields.get("target"),
        Some(&Value::Str("clip.mp4".into()))
    );
    assert_eq!(
        embeds[1].fields.get("title"),
        Some(&Value::Str("说明".into()))
    );
    assert_eq!(
        embeds[2].fields.get("description"),
        Some(&Value::Str(String::new()))
    );
    assert!(embeds.iter().all(|e| e.children.is_empty()));

    let md = engine
        .analyze(
            "test.md",
            "[![描述](asset)](destination)",
            notist::builtins::registry(),
        )
        .unwrap();
    assert!(md.diagnostics().is_empty());
    let link = md.root().find(|n| n.ctor == Ctor::Link).unwrap();
    assert_eq!(
        link.fields.get("target"),
        Some(&Value::Str("destination".into()))
    );
    assert_eq!(link.children[0].ctor, Ctor::Embed);
}

#[test]
fn embed_function_supports_named_target_attrs_and_reports_invalid_calls() {
    let root =
        analyze("前 @(id: \"resource\")#embed(target: \"paper.pdf\", description: \"说明\") 后");
    let embed = root.find(|n| n.ctor == Ctor::Embed).unwrap();
    assert_eq!(embed.attrs.get("id"), Some(&Value::Str("resource".into())));
    assert_eq!(notist::query::select(&root, "ctor:embed").len(), 1);
    assert_eq!(text_of(&root), "前  后");

    let engine = Pipeline::default();
    for (src, message) in [
        ("#embed(\"asset\")[内容]", "`embed` takes no children"),
        (
            "#embed(\"asset\", \"extra\")",
            "too many positional arguments for `embed`",
        ),
    ] {
        let doc = engine
            .analyze("test.not", src, notist::builtins::registry())
            .unwrap();
        assert!(
            doc.diagnostics()
                .iter()
                .any(|d| d.message.contains(message)),
            "{src}"
        );
    }
}

#[test]
fn markdown_embed_decodes_fields_once_and_keeps_code_payload_literal() {
    let doc = Pipeline::default()
        .analyze("test.md", r#"![A &amp; B &#20013; \*literal\* \&amp; &#38;amp; `&amp;` $\alpha$](asset\(1\)?a=1&amp;b=2 "A &quot;title&quot;")"#, notist::builtins::registry())
        .unwrap();
    assert!(doc.diagnostics().is_empty());
    let embed = doc.root().find(|n| n.ctor == Ctor::Embed).unwrap();
    assert_eq!(
        embed.fields.get("description"),
        Some(&Value::Str(
            "A & B 中 *literal* &amp; &amp; &amp; \\alpha".into()
        ))
    );
    assert_eq!(
        embed.fields.get("target"),
        Some(&Value::Str("asset(1)?a=1&b=2".into()))
    );
    assert_eq!(
        embed.fields.get("title"),
        Some(&Value::Str("A \"title\"".into()))
    );
}
