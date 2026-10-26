use notist::{Ctor, Item, Pipeline, Value};
use notist_html::{Renderer, escape_attribute, escape_text, is_safe_url, render};

#[test]
fn code_renderer_preserves_the_shell_and_escapes_fallbacks() {
    let root = analyze(
        "md",
        "`<inline>`\n\n```rust\nlet x = 1 < 2;\n```\n\n```unknown\n<script>\n```",
    );
    let before = root.clone();
    let rendered = Renderer::new()
        .with_code_renderer(|item| {
            (item.fields.get("lang") == Some(&Value::Str("rust".into())))
                .then(|| "<span class=\"keyword\">let</span> x = 1 &lt; 2;".into())
        })
        .render(&root);
    assert!(
        rendered.contains("<pre><code class=\"language-rust\"><span class=\"keyword\">let</span>")
    );
    assert!(rendered.contains("<code>&lt;inline&gt;</code>"));
    assert!(rendered.contains("&lt;script&gt;"));
    assert!(!rendered.contains("<script>"));
    assert_eq!(root, before);
}

#[test]
fn publication_url_resolver_preserves_url_safety_and_source_tree() {
    let root = analyze("not", "[Guide](guide.not) ![Picture](picture.svg)");
    let before = root.clone();
    let output = Renderer::new()
        .with_url_resolver(|_, url| match url {
            "guide.not" => Some("../guide/".into()),
            "picture.svg" => Some("../media/picture.svg".into()),
            _ => None,
        })
        .render_with_diagnostics(&root);
    assert!(output.diagnostics.is_empty());
    assert!(output.html.contains("href=\"../guide/\""));
    assert!(output.html.contains("href=\"../media/picture.svg\""));
    assert_eq!(root, before);
    let output = Renderer::new()
        .with_url_resolver(|_, _| Some("javascript:bad".into()))
        .render_with_diagnostics(&root);
    assert_eq!(output.diagnostics.len(), 2);
    assert!(!output.html.contains("href="));
    let root = analyze("not", "[Bad](javascript:bad)");
    let output = Renderer::new()
        .with_url_resolver(|_, _| panic!("unsafe input must not reach resolver"))
        .render_with_diagnostics(&root);
    assert_eq!(output.diagnostics.len(), 1);
}

fn analyze(extension: &str, src: &str) -> Item {
    let document = Pipeline::default()
        .analyze(
            format!("test.{extension}"),
            src,
            &notist::Context::default(),
        )
        .unwrap();
    assert!(
        document.diagnostics().is_empty(),
        "{src}: {:?}",
        document.diagnostics()
    );
    document.into_parts().0
}

#[test]
fn both_frontends_render_the_same_complete_document() {
    let not = "= Title\n\n*bold* _italic_ ~removed~ `a < b` $x$ [label](https://example.com/?a=1&b=2)\n\n- first\n\n  second\n  + nested\n\n---\n\n#callout(kind: \"quote\")[ quoted ]\n\n![asset](file.pdf \"title\")\n\n```rust\na < b\n```\n";
    let md = "# Title\n\n**bold** *italic* ~~removed~~ `a < b` $x$ [label](https://example.com/?a=1&b=2)\n\n- first\n\n  second\n  1. nested\n\n---\n\n> quoted\n\n![asset](file.pdf \"title\")\n\n```rust\na < b\n```\n";
    let result = Renderer::new().render_with_diagnostics(&analyze("not", not));
    assert!(result.diagnostics.is_empty());
    assert_eq!(result.html, render(&analyze("md", md)));
    assert_eq!(
        result.html,
        concat!(
            "<section><h1>Title</h1>",
            "<p><strong>bold</strong> <em>italic</em> <del>removed</del> ",
            "<code>a &lt; b</code> <span class=\"notist-math\">x</span> ",
            "<a href=\"https://example.com/?a=1&amp;b=2\">label</a></p>",
            "<ul><li><p>first</p><p>second</p><ol><li><p>nested</p></li></ol></li></ul>",
            "<hr><blockquote data-notist-kind=\"quote\"><p>quoted</p></blockquote>",
            "<p><span class=\"notist-embed\" title=\"title\"><a href=\"file.pdf\">asset</a></span></p>",
            "<pre><code class=\"language-rust\">a &lt; b\n</code></pre></section>"
        )
    );
}

#[test]
fn block_math_renders_outside_paragraphs_and_shares_math_hooks() {
    let source = "before $x$\n\n@(id: \"equation\")$ x < y $\n\nafter";
    for extension in ["not", "md"] {
        let root = analyze(extension, source);
        let result = Renderer::new().render_with_diagnostics(&root);
        assert!(result.diagnostics.is_empty());
        assert_eq!(
            result.html,
            concat!(
                "<p>before <span class=\"notist-math\">x</span></p>",
                "<div class=\"notist-math\" id=\"equation\">x &lt; y</div>",
                "<p>after</p>"
            )
        );
        let equation = root
            .find(|n| n.ctor == Ctor::Math && n.level == notist::builtins::Level::Block)
            .unwrap();
        let mapped = Renderer::new()
            .with_source_map()
            .render_with_diagnostics(&root);
        assert!(mapped.source_map.iter().any(|mapping| mapping.range.start
            == usize::from(equation.span.start())
            && mapping.range.end == usize::from(equation.span.end())));
        let result = Renderer::new()
            .with_math_renderer(|item| {
                (item.level == notist::builtins::Level::Block)
                    .then(|| "<math display=\"block\"><mi>x</mi></math>".into())
            })
            .render(&root);
        assert!(result.contains("<div class=\"notist-math\" id=\"equation\"><math display=\"block\"><mi>x</mi></math></div>"));
        assert!(result.contains("<span class=\"notist-math\">x</span>"));
    }
}

#[test]
fn table_headers_alignment_and_block_cells_are_preserved() {
    let sugar = "| *Name* | Count |\n| :-- | --: |\n| A | 3 |\n";
    let root = analyze("not", sugar);
    assert_eq!(
        render(&root),
        render(&analyze("md", &sugar.replace("*Name*", "**Name**")))
    );
    assert_eq!(
        render(&root),
        concat!(
            "<table><thead><tr><th scope=\"col\" style=\"text-align: left\"><p><strong>Name</strong></p></th>",
            "<th scope=\"col\" style=\"text-align: right\"><p>Count</p></th></tr></thead>",
            "<tbody><tr><td style=\"text-align: left\"><p>A</p></td>",
            "<td style=\"text-align: right\"><p>3</p></td></tr></tbody></table>"
        )
    );
    let root = analyze(
        "not",
        "#table[\n#row(header: true)[#cell[]]\n#row[\n#cell[ \nfirst\n\n- nested\n ]\n]\n]",
    );
    assert_eq!(
        render(&root),
        "<table><thead><tr><th scope=\"col\"></th></tr></thead><tbody><tr><td><p>first</p><ul><li><p>nested</p></li></ul></td></tr></tbody></table>"
    );
}

#[test]
fn later_header_rows_keep_their_position() {
    let root = analyze(
        "not",
        "#table[#row[#cell[ a ]] #row(header: true)[#cell[ b ]] #row[#cell[ c ]]]",
    );
    assert_eq!(
        render(&root),
        "<table><tbody><tr><td><p>a</p></td></tr><tr><th scope=\"col\"><p>b</p></th></tr><tr><td><p>c</p></td></tr></tbody></table>"
    );
    assert_eq!(render(&analyze("not", "#table[]")), "<table></table>");
}

#[test]
fn identities_classes_and_metadata_are_attached_once() {
    let root = analyze(
        "not",
        "@!(id: \"document\")\n\n@(id: \"intro\", tags: (\"a\", \"b\"))\n= Intro\n\n@(id: \"range\")\nbefore\n\n#callout(kind: \"note\")[ body ]\n\nafter\n",
    );
    let html = render(&root);
    assert_eq!(
        html,
        concat!(
            "<div id=\"document\"><section id=\"intro\" data-notist-tags=\"a b\"><h1>Intro</h1>",
            "<p id=\"range\">before</p><div class=\"notist-callout\" data-notist-kind=\"note\"><p>body</p></div>",
            "<p>after</p></section></div>"
        )
    );
    assert_eq!(html.matches("id=\"range\"").count(), 1);
    let root = analyze(
        "not",
        "@(id: \"inline\", class: (\"a\", \"b\"), aria-label: \"説明\", data-test: true)#node[*text*]",
    );
    assert_eq!(
        render(&root),
        "<p><span class=\"a b\" id=\"inline\" aria-label=\"説明\" data-test=\"true\"><strong>text</strong></span></p>"
    );
    assert_eq!(
        render(&analyze("not", "#node[*text*]")),
        "<p><strong>text</strong></p>"
    );
}

#[test]
fn custom_annotations_map_to_data_attributes_with_values_and_reserved_names_preserved() {
    let mut root = analyze("not", "Metadata");
    root.attrs.insert("status", Value::Str("draft".into()));
    root.attrs.insert("reviewed", Value::Bool(false));
    root.attrs.insert("revision", Value::Int(i64::MAX));
    root.attrs.insert("note", Value::Str("<\"&".into()));
    root.attrs.insert("optional", Value::Unit);
    root.attrs.insert(
        "context",
        Value::Array(vec![Value::Int(i64::MAX), Value::Float(-0.0)]),
    );
    root.attrs.insert("data-notist-node", Value::Int(999));
    root.attrs.insert("notist-node", Value::Int(998));
    root.attrs.insert("aria-level", Value::Int(99));
    let output = render(&root);
    assert!(output.contains("data-status=\"draft\""));
    assert!(output.contains("data-reviewed=\"false\""));
    assert!(output.contains("data-revision=\"9223372036854775807\""));
    assert!(output.contains("data-note=\"&lt;&quot;&amp;\""));
    assert!(output.contains("data-optional=\"null\""));
    assert!(output.contains("data-context=\"[&quot;array&quot;,[[&quot;int&quot;,&quot;9223372036854775807&quot;],[&quot;float&quot;,&quot;8000000000000000&quot;]]]\""));
    assert!(!output.contains("data-notist-node"));
    assert!(!output.contains("aria-level"));
    root.attrs
        .insert("data-status", Value::Str("staged".into()));
    let output = render(&root);
    assert_eq!(output.matches("data-status=").count(), 1);
    assert!(output.contains("data-status=\"staged\""));
}

#[test]
fn status_annotations_preserve_sections_inline_groups_and_cells_in_both_frontends() {
    for (extension, heading) in [("not", "= Status"), ("md", "# Status")] {
        let source = format!(
            "@(status: \"draft\")\n{heading}\n\n@(status: \"staged\")\nPending implementation.\n\nInline @(status: \"outdate\")#node[old term] remains inline.\n\n#table[ #row[ @(status: \"outdate\")#cell[ Old definition ] #cell[ Current definition ] ] ]"
        );
        let root = analyze(extension, &source);
        let before = root.clone();
        let output = Renderer::new().render_with_diagnostics(&root);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        assert!(output.html.contains("<section data-status=\"draft\">"));
        assert!(
            output
                .html
                .contains("<p data-status=\"staged\">Pending implementation.</p>")
        );
        assert!(
            output
                .html
                .contains("<span data-status=\"outdate\">old term</span>")
        );
        assert!(
            output
                .html
                .contains("<td data-status=\"outdate\"><p>Old definition</p></td>")
        );
        assert_eq!(root, before);
        assert!(output.used_components.is_empty());
    }
}

#[test]
fn escaping_occurs_after_frontend_decoding_and_in_all_attributes() {
    assert_eq!(escape_text("<>&\"'\0"), "&lt;&gt;&amp;\"'�");
    assert_eq!(escape_attribute("<>&\"'\0"), "&lt;&gt;&amp;&quot;&#39;�");
    assert_eq!(
        render(&analyze("md", "&lt;script&gt; &amp;amp; &#38;")),
        "<p>&lt;script&gt; &amp;amp; &amp;</p>"
    );
    let mut root = analyze("not", "#text(text: \"<script>&\")");
    let text = &mut root.children[0].children[0];
    text.attrs
        .insert("id", Value::Str("\" onmouseover=\"bad".into()));
    text.attrs.insert("onclick", Value::Str("bad()".into()));
    text.attrs.insert("style", Value::Str("bad".into()));
    text.attrs
        .insert("data-x\" onclick", Value::Str("bad".into()));
    assert_eq!(
        render(&root),
        "<p><span id=\"&quot; onmouseover=&quot;bad\" data-onclick=\"bad()\" data-style=\"bad\">&lt;script&gt;&amp;</span></p>"
    );
    let root = analyze(
        "not",
        "#embed(\"file\", description: \"<asset>\", title: \"\\\"<&\")",
    );
    assert_eq!(
        render(&root),
        "<p><span class=\"notist-embed\" title=\"&quot;&lt;&amp;\"><a href=\"file\">&lt;asset&gt;</a></span></p>"
    );
}

#[test]
fn unsafe_urls_are_omitted_with_source_diagnostics() {
    for target in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "\0 javascript:alert(1)",
        "java\nscript:alert(1)",
        "java\tscript:alert(1)",
        "data:text/html,bad",
        "vbscript:bad",
    ] {
        assert!(!is_safe_url(target), "{target:?}");
        let mut root = analyze("not", "[label](safe)");
        let link = &mut root.children[0].children[0];
        link.fields.insert("target", Value::Str(target.into()));
        let span = link.span;
        let result = Renderer::new().render_with_diagnostics(&root);
        assert_eq!(result.html, "<p><a>label</a></p>");
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].span, span);
    }
    for target in [
        "#item",
        "../other.not#id",
        "https://a/b",
        "HTTP://a",
        "mailto:a@example.com",
        "tel:123",
        "ftp://a",
        "//a/path",
        "folder/a:b",
        "?q=javascript:bad",
    ] {
        assert!(is_safe_url(target), "{target}");
    }
    let result =
        Renderer::new().render_with_diagnostics(&analyze("not", "![asset](javascript:bad)"));
    assert_eq!(
        result.html,
        "<p><span class=\"notist-embed\"><a>asset</a></span></p>"
    );
    assert_eq!(result.diagnostics.len(), 1);
}

#[test]
fn callbacks_override_selected_resources_and_preserve_outer_attributes() {
    let prefix = "resources/";
    let renderer = Renderer::new()
        .with_embed_renderer(|item| {
            let Value::Str(target) = item.fields.get("target")? else {
                return None;
            };
            if !target.ends_with(".png") || !is_safe_url(target) {
                return None;
            }
            Some(format!(
                "<img src=\"{}{}\" alt=\"asset\">",
                prefix,
                escape_attribute(target)
            ))
        })
        .with_math_renderer(|item| {
            let Value::Str(text) = item.fields.get("text")? else {
                return None;
            };
            if text != "x" {
                return None;
            }
            Some("<math><mi>x</mi></math>".into())
        });
    let root = analyze(
        "not",
        "@(id: \"image\")![asset](a.png) ![doc](b.pdf) @(id: \"formula\")$x$ $y$",
    );
    assert_eq!(
        renderer.render(&root),
        concat!(
            "<p><span class=\"notist-embed\" id=\"image\"><img src=\"resources/a.png\" alt=\"asset\"></span> ",
            "<span class=\"notist-embed\"><a href=\"b.pdf\">doc</a></span> ",
            "<span class=\"notist-math\" id=\"formula\"><math><mi>x</mi></math></span> ",
            "<span class=\"notist-math\">y</span></p>"
        )
    );
}

#[test]
fn headings_beyond_html_levels_and_list_starts_are_preserved() {
    let root = analyze(
        "not",
        "======= Deep\n\n#list(ordered: true, start: -3)[#item[ a ] #item[]]\n\n#list[#item[ b ]]\n\n[[target]]",
    );
    assert_eq!(
        render(&root),
        concat!(
            "<section><div class=\"notist-heading\" role=\"heading\" aria-level=\"7\">Deep</div>",
            "<ol start=\"-3\"><li><p>a</p></li><li></li></ol><ul><li><p>b</p></li></ul>",
            "<p><a href=\"target\">target</a></p></section>"
        )
    );
}

#[test]
fn raw_payloads_and_language_attrs_are_escaped_without_trimming() {
    let root = analyze(
        "not",
        "@(id: \"code\")\n```rust\n<x> & \"\n\n```\n\n#raw(\"&amp;\", lang: \"html\")",
    );
    assert_eq!(
        render(&root),
        "<pre id=\"code\"><code class=\"language-rust\">&lt;x&gt; &amp; \"\n\n</code></pre><p><code class=\"language-html\">&amp;amp;</code></p>"
    );
}

#[test]
fn structural_groups_are_flattened_and_unrepresentable_attrs_are_reported() {
    let root = analyze(
        "not",
        "#list[#node[ #item[ a ]\n\n#item[ b ] ]]\n\n#table[#node[ #row[#node[ #cell[ x ]\n\n#cell[ y ] ]] ]]",
    );
    let result = Renderer::new().render_with_diagnostics(&root);
    assert!(result.diagnostics.is_empty());
    assert_eq!(
        result.html,
        "<ul><li><p>a</p></li><li><p>b</p></li></ul><table><tbody><tr><td><p>x</p></td><td><p>y</p></td></tr></tbody></table>"
    );
    let root = analyze(
        "not",
        "#list[@(id: \"items\")#node[ #item[ a ]\n\n#item[ b ] ]]",
    );
    let result = Renderer::new().render_with_diagnostics(&root);
    assert_eq!(result.html, "<ul><li><p>a</p></li><li><p>b</p></li></ul>");
    assert_eq!(result.diagnostics.len(), 1);
    assert!(
        result.diagnostics[0]
            .message
            .contains("node attributes omitted")
    );
}

#[test]
fn recovery_nodes_render_without_panicking_and_report_rendering_issues() {
    let document = Pipeline::default()
        .analyze(
            "test.not",
            "#unknown[body]\n\n#list[text]\n\n#table[wrong]",
            &notist::Context::default(),
        )
        .unwrap();
    assert!(!document.diagnostics().is_empty());
    let result = Renderer::new().render_with_diagnostics(document.root());
    assert_eq!(
        result.html,
        "<p><span class=\"notist-custom\" data-notist-constructor=\"unknown\">body</span></p><ul><li>text</li></ul><table><tbody><tr><td>wrong</td></tr></tbody></table>"
    );
    assert_eq!(result.diagnostics.len(), 3);
    let mut item = analyze("not", "#embed(\"target\")")
        .children
        .remove(0)
        .children
        .remove(0);
    item.fields.insert("target", Value::Int(3));
    assert_eq!(render(&item), "<span class=\"notist-embed\"><a></a></span>");
    assert_eq!(item.ctor, Ctor::Embed);
}
