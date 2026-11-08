use notist_core::frontend::{Frontend, FrontendOptions};

fn analyze(
    src: &str,
) -> (
    notist_core::item::Item,
    Vec<notist_core::diag::Diagnostic>,
    notist_core::source::Sources,
) {
    notist_pipeline::Pipeline::new()
        .with_frontend(notist_pipeline::frontend::markdown::MarkdownFrontend)
        .analyze("test.md", src, &notist_pipeline::Context::default())
        .unwrap()
        .into_parts()
}

#[test]
fn markdown_wikilinks_lower_to_normal_links_with_raw_targets() {
    use notist_core::item::{Ctor, Value};
    let src =
        "前文 [[ page.not ]]，[[目录/笔记.md#节|显示 **粗体**]] [[a\\!b&copy;.not]] [[page.not|]]";
    let (root, diagnostics, _sources) = analyze(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let links: Vec<_> = root
        .descendants()
        .filter(|n| n.ctor() == Ctor::Link)
        .collect();
    assert_eq!(links.len(), 4);
    for (link, target, source) in [
        (links[0], "page.not", "[[ page.not ]]"),
        (
            links[1],
            "目录/笔记.md#节",
            "[[目录/笔记.md#节|显示 **粗体**]]",
        ),
        (links[2], "a\\!b&copy;.not", "[[a\\!b&copy;.not]]"),
        (links[3], "page.not", "[[page.not|]]"),
    ] {
        assert_eq!(link.field("target"), Some(&Value::Str(target.into())));
        assert_eq!(
            &src[usize::from(link.span.start())..usize::from(link.span.end())],
            source
        );
    }
    assert!(links[0].children().is_empty());
    assert!(links[3].children().is_empty());
    let strong = links[1].find(|n| n.ctor() == Ctor::Strong).unwrap();
    assert_eq!(
        &src[usize::from(strong.span.start())..usize::from(strong.span.end())],
        "**粗体**"
    );
}

#[test]
fn markdown_wikilinks_respect_code_escapes_and_line_boundaries() {
    use notist_core::item::{Ctor, Value};
    let src = "`[[code.not]]` \\[[escaped.not]] [[unclosed\nnext]]\n\n```text\n[[fenced.not]]\n```\n\n[普通](normal.not) [[valid.not]]";
    let (root, diagnostics, _sources) = analyze(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let targets: Vec<_> = root
        .descendants()
        .filter(|n| n.ctor() == Ctor::Link)
        .map(|n| n.field("target").unwrap().clone())
        .collect();
    assert_eq!(
        targets,
        vec![
            Value::Str("normal.not".into()),
            Value::Str("valid.not".into())
        ]
    );
}

#[test]
fn markdown_wikilinks_inside_containers_and_content_keep_offsets() {
    use notist_core::item::{Ctor, Value};
    for src in [
        "> [[page.not|显示]]\n",
        "- [[page.not|显示]]\n",
        "# 标题 [[page.not|显示]]\n",
        "#strong[[[page.not|显示]]]",
        "| 链接 |\n| --- |\n| [[page.not\\|显示]] |\n",
    ] {
        let (root, diagnostics, _sources) = analyze(src);
        assert!(diagnostics.is_empty(), "{src:?}: {diagnostics:?}");
        let link = root.find(|n| n.ctor() == Ctor::Link).unwrap();
        assert_eq!(link.field("target"), Some(&Value::Str("page.not".into())));
        assert_eq!(
            link.children()[0].field("text"),
            Some(&Value::Str("显示".into()))
        );
        let source = &src[usize::from(link.span.start())..usize::from(link.span.end())];
        assert!(source.starts_with("[[page.not"), "{src:?}: {source:?}");
        assert!(source.ends_with("]]"), "{src:?}: {source:?}");
        assert!(link.descendants().all(|n| link.span.contains_range(n.span)));
    }
}

#[test]
fn markdown_wikilink_labels_keep_annotations_calls_and_diagnostic_offsets() {
    use notist_core::item::{Ctor, Value};
    let src = "前文 [[page.not|@(id: \"label\")**粗体** #emph[显示]]]";
    let (root, diagnostics, _sources) = analyze(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let strong = root.find(|n| n.ctor() == Ctor::Strong).unwrap();
    assert_eq!(strong.attrs.get("id"), Some(&Value::Str("label".into())));
    let emph = root.find(|n| n.ctor() == Ctor::Emph).unwrap();
    assert_eq!(
        &src[usize::from(emph.span.start())..usize::from(emph.span.end())],
        "#emph[显示]"
    );
    let src = "前文 [[page.not|#missing[]]]";
    let (_, diagnostics, _sources) = analyze(src);
    assert!(!diagnostics.is_empty());
    for diagnostic in diagnostics {
        assert!(usize::from(diagnostic.span.start()) >= src.find('#').unwrap());
        assert!(usize::from(diagnostic.span.end()) <= src.len());
    }
}

#[test]
fn markdown_lowering() {
    let src = "# 标题 *em*\n\n段落 **粗** _斜_ `代码` [链接](x.not)。\n\n- 项一\n- 项二\n\n```rust\nfn main() {}\n```\n";
    let (item, diags, _sources) = analyze(src);
    assert!(diags.is_empty(), "{diags:?}");
    let dump = notist_core::dump::dump(&item, &_sources).unwrap();
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
    let (item, diags, _sources) = analyze(src);
    assert!(diags.is_empty(), "{diags:?}");
    let dump = notist_core::dump::dump(&item, &_sources).unwrap();
    assert!(dump.contains("callout"), "{dump}");
    assert!(dump.contains("table"), "{dump}");
    assert!(dump.contains("cell"), "{dump}");
}

#[test]
fn atx_heading_spans_include_closing_markers_but_exclude_line_breaks() {
    for (src, expected) in [
        ("## title *em* ##\r\n\r\ntext", "## title *em* ##"),
        ("> # [title](target)\n>\n> text", "# [title](target)"),
        ("#", "#"),
    ] {
        let (root, diags, _sources) = analyze(src);
        assert!(diags.is_empty());
        let heading = root
            .find(|n| n.ctor() == notist_core::item::Ctor::Heading)
            .unwrap();
        assert_eq!(
            &src[usize::from(heading.span.start())..usize::from(heading.span.end())],
            expected
        );
        assert!(
            heading
                .descendants()
                .all(|n| heading.span.contains_range(n.span))
        );
    }
}

#[test]
fn markdown_notist_attributes_attach_to_module_blocks_and_inline_elements() {
    use notist_core::item::{Ctor, Value};
    let src = "@!(title: \"测试\", tags: (\"md\", \"notist\"))\n@(id: \"intro\")\n# 标题\n\n@(role: \"body\")\n正文 @(color: \"red\")**粗体** 和 @(id: \"link\")[链接](target)。\n";
    let (root, diagnostics, _sources) = analyze(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(root.attrs.get("title"), Some(&Value::Str("测试".into())));
    for (ctor, key, value) in [
        (Ctor::Section, "id", "intro"),
        (Ctor::Paragraph, "role", "body"),
        (Ctor::Strong, "color", "red"),
        (Ctor::Link, "id", "link"),
    ] {
        let node = root.find(|n| n.ctor() == ctor).unwrap();
        assert_eq!(node.attrs.get(key), Some(&Value::Str(value.into())));
    }
}

#[test]
fn markdown_calls_lower_markdown_bodies_and_share_literal_arguments() {
    use notist_core::item::{Ctor, Value};
    let src = "前缀 #strong[**粗体** 和 *斜体*]  #raw(r#\"a\\b\"#)\n\n@(id: \"box\")\n#callout(kind: \"note\")[\n## 标题\n\n- 项一\n- #emph[项二]\n\n```text\n#missing[] @(ignored: true) ]\n```\n]\n";
    let (root, diagnostics, _sources) = analyze(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let callout = root.find(|n| n.ctor() == Ctor::Callout).unwrap();
    assert_eq!(callout.attrs.get("id"), Some(&Value::Str("box".into())));
    assert_eq!(callout.field("kind"), Some(&Value::Str("note".into())));
    assert!(callout.find(|n| n.ctor() == Ctor::Heading).is_some());
    assert!(callout.find(|n| n.ctor() == Ctor::List).is_some());
    assert!(callout.find(|n| n.ctor() == Ctor::Emph).is_some());
    assert!(
        root.find(|n| n.field("text") == Some(&Value::Str("a\\b".into())))
            .is_some()
    );
    for node in root.descendants() {
        assert!(
            src.is_char_boundary(usize::from(node.span.start())),
            "{node:?}"
        );
        assert!(
            src.is_char_boundary(usize::from(node.span.end())),
            "{node:?}"
        );
        assert!(usize::from(node.span.end()) <= src.len());
    }
}

#[test]
fn markdown_extensions_preserve_opaque_and_escaped_text() {
    let src = "# Heading\n\n`@(id: \"code\") #strong[x]` $a_{#strong[x]}$ \\#strong[x] \\@(id: \"escaped\") \\#tag mail@example.com\n\n```\n@!(id: \"raw\")\n#missing[]\n```\n";
    let (root, diagnostics, _sources) = analyze(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(root.descendants().all(|n| n.attrs.is_empty()));
    assert!(
        root.find(|n| n.ctor() == notist_core::item::Ctor::Strong)
            .is_none()
    );
}

#[test]
fn markdown_extension_diagnostics_cover_invalid_annotations_and_calls() {
    for (src, message) in [
        ("@(42)\ntext", "annotation payload must be a dict literal"),
        ("@(id: bare)\ntext", "bare names are not literals"),
        (
            "text\n\n@!(id: \"late\")",
            "module annotation must precede all content",
        ),
        (
            "text @(id: \"x\") plain",
            "annotation must be immediately followed by an element",
        ),
        (
            "@(id: \"dangling\")",
            "block annotation has no following block",
        ),
        ("#missing[x]", "unknown"),
        ("#callout[\ntext", "unclosed Notist annotation or call"),
        (
            "> @!(id: \"nested\")\n> text",
            "module annotation is only valid at the document top",
        ),
    ] {
        let (_, diagnostics, _sources) = analyze(src);
        assert!(
            diagnostics.iter().any(|d| d.message.contains(message)),
            "{src:?}: {diagnostics:?}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|d| usize::from(d.span.end()) <= src.len())
        );
    }
}

#[test]
fn markdown_extensions_inside_containers_keep_original_offsets() {
    use notist_core::item::{Ctor, Value};
    let src = "> @(id: \"quote\")\n> 引用 #strong[粗体]\n\n- @(id: \"item\")\n  正文 @(id: \"em\")*强调*\n";
    let (root, diagnostics, _sources) = analyze(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let strong = root.find(|n| n.ctor() == Ctor::Strong).unwrap();
    assert_eq!(
        &src[usize::from(strong.span.start())..usize::from(strong.span.end())],
        "#strong[粗体]"
    );
    let emph = root.find(|n| n.ctor() == Ctor::Emph).unwrap();
    assert_eq!(emph.attrs.get("id"), Some(&Value::Str("em".into())));
    assert!(
        root.find(|n| n.attrs.get("id") == Some(&Value::Str("quote".into())))
            .is_some()
    );
    assert!(
        root.find(|n| n.attrs.get("id") == Some(&Value::Str("item".into())))
            .is_some()
    );
}

#[test]
fn markdown_call_boundaries_keep_inline_markers_and_trailing_text() {
    use notist_core::item::{Ctor, Value};
    let src = "#strong[# heading]\n\n#callout[\n正文\n] 后面的文字\n\n前缀 #strong[未闭合";
    let (root, diagnostics, _sources) = analyze(src);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("unclosed Notist"))
    );
    assert!(root.find(|n| n.ctor() == Ctor::Heading).is_none());
    for text in ["# heading", "后面的文字", "#strong[未闭合"] {
        assert!(
            root.find(|n| n.field("text") == Some(&Value::Str(text.into())))
                .is_some(),
            "{text}: {}",
            notist_core::dump::dump(&root, &Default::default()).unwrap()
        );
    }
}

#[test]
fn markdown_nested_calls_anonymous_groups_and_multiline_payloads() {
    use notist_core::item::{Ctor, Value};
    let src = "@!(\n title: \"标题\",\n meta: (nested: (1, true, -2)),\n)\n@(id: \"group\")\n#node[\n#notist.callout(kind: \"note\")[\n@(id: \"inner\")\n正文 #strong[一 #emph[二] `]` $x_{]}$]\n]\n]\n";
    let (root, diagnostics, _sources) = analyze(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(root.attrs.get("title"), Some(&Value::Str("标题".into())));
    assert!(
        root.find(
            |n| n.ctor() == Ctor::Node && n.attrs.get("id") == Some(&Value::Str("group".into()))
        )
        .is_some()
    );
    assert!(root.find(|n| n.ctor() == Ctor::Callout).is_some());
    assert!(root.find(|n| n.ctor() == Ctor::Emph).is_some());
    let raw = root.find(|n| n.ctor() == Ctor::RawInline).unwrap();
    assert_eq!(
        &src[usize::from(raw.span.start())..usize::from(raw.span.end())],
        "`]`"
    );
    for node in root.descendants() {
        assert!(
            node.descendants()
                .all(|child| node.span.contains_range(child.span)),
            "{node:?}"
        );
    }
}

#[test]
fn markdown_call_fences_hide_brackets_and_extensions() {
    for (open, close) in [("~~~", "~~~~"), ("```", "````")] {
        let src = format!("#callout[\n{open}text\n] #missing[] @(id: \"opaque\")\n{close}\n]\n");
        let (root, diagnostics, _sources) = analyze(&src);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let raw = root
            .find(|n| n.ctor() == notist_core::item::Ctor::RawInline)
            .unwrap();
        assert_eq!(
            raw.field("text"),
            Some(&notist_core::item::Value::Str(
                "] #missing[] @(id: \"opaque\")\n".into()
            ))
        );
        assert!(
            root.find(|n| n.ctor() == notist_core::item::Ctor::Callout)
                .is_some()
        );
    }
}

#[test]
fn malformed_markdown_extensions_recover_for_every_truncated_prefix() {
    for src in [
        "#raw(\n\n\"value\"\n)",
        "@(id: (nested: \"值\"))\n#callout[\n# 标题\n\n正文 #strong[*内容*]\n]",
        "#notist.raw(r#\"raw ) value\"#, lang: \"text\")",
        "前缀 #strong[未闭合",
    ] {
        for end in src
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(src.len()))
        {
            let (_, diagnostics, _sources) = analyze(&src[..end]);
            assert!(
                diagnostics.iter().all(|d| usize::from(d.span.end()) <= end),
                "{src:?} at {end}: {diagnostics:?}"
            );
        }
    }
    let (_, diagnostics, _sources) = analyze("#raw(\n\n\"value\"\n)");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("invalid Notist call header"))
    );
}

#[test]
fn markdown_call_arguments_with_unicode_bodies_keep_valid_source_ranges() {
    let src = "#link(\"target\")[中文]";
    let output = notist_pipeline::frontend::markdown::MarkdownFrontend
        .compile(src, FrontendOptions::default());
    let (forest, diagnostics) = (output.forest, output.diagnostics);
    assert!(diagnostics.is_empty());
    fn check(expr: &notist_core::expr::Expr, src: &str) {
        assert!(src.is_char_boundary(usize::from(expr.span().start())));
        assert!(src.is_char_boundary(usize::from(expr.span().end())));
        if let notist_core::expr::Expr::Call {
            callee,
            args,
            children,
            ..
        } = expr
        {
            check(callee, src);
            for child in args.iter().map(|arg| arg.value()).chain(children) {
                check(child, src);
            }
        }
    }
    for expr in &forest {
        check(expr, src);
    }
    let (_, diagnostics, _sources) = analyze(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn markdown_frontend_captures_its_own_parse_tree_for_inspection() {
    let src = "@(id: \"intro\")\n# 标题\n\n#strong[正文]";
    let frontend = notist_pipeline::frontend::markdown::MarkdownFrontend;
    assert!(
        frontend
            .compile(src, FrontendOptions::default())
            .syntax
            .is_none()
    );
    let pipeline = notist_pipeline::Pipeline::default();
    let (analysis, inspection) = pipeline
        .inspect("document.nmd", src, &notist_pipeline::Context::default())
        .unwrap();
    let syntax = inspection
        .syntax
        .as_deref()
        .unwrap()
        .as_any()
        .downcast_ref::<notist_pipeline::frontend::markdown::MarkdownSyntax>()
        .unwrap();
    assert_eq!(syntax.source, src);
    assert!(syntax.arena.get(syntax.root).is_some());
    let ordinary = pipeline
        .analyze("document.nmd", src, &notist_pipeline::Context::default())
        .unwrap();
    assert_eq!(analysis, ordinary);
}
