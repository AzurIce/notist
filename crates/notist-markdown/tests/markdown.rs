use notc::{AttrValue, BlockKind, DocumentFrontend, NotistFrontend, parse_file_with_frontends};
use notist_markdown::{MarkdownFrontend, parse_markdown_document};

#[test]
fn rushdown_markdown_supports_commonmark_gfm_preamble_imports_and_safe_hash_calls() {
    let ui = r#"
fn note(body: Block) -> Block {
    callout("note", body)
}
fn tag(label: String) -> Inline {
    strong(text(label))
}
"#;
    let md = r#"
@!(title: "MD Title", draft: false)
#import "./ui.notc" as ui

# Heading One

> Quoted link [line ref](https://github.com/org/repo/blob/main/a.rs#L1-L27) and _italic #not_a_call_.

Paragraph with #tag_plain, #ui.tag("ok"), [ref link][r1], [[guide|Guide]], and $a^2 + b^2$.

| Col A | Col B |
| ----- | ----- |
| 1 &amp; 2 | `code` |

$$
y = x^2
$$

#ui.note[
    Inside *Notist* callout.
]

[r1]: https://example.com#L10
"#;
    let parsed_ui = notc::parse(ui);
    assert!(parsed_ui.diagnostics.is_empty(), "{:?}", parsed_ui.diagnostics);
    let parsed_md = parse_markdown_document(md);
    assert!(parsed_md.diagnostics.is_empty(), "{:?}", parsed_md.diagnostics);

    let project = notc::check_parsed_modules(&[
        ("ui.notc", &parsed_ui.program.root),
        ("post.md", &parsed_md.program.root),
    ])
    .expect("markdown + notc project checks cleanly");
    let doc = project.eval_document("post.md").expect("evaluates");

    assert_eq!(
        doc.node().attrs.get("title"),
        Some(&AttrValue::String("MD Title".into()))
    );
    assert_eq!(doc.node().attrs.get("draft"), Some(&AttrValue::Bool(false)));

    let plain = doc.plain_text();
    assert!(plain.contains("Heading One"), "plain: {plain}");
    assert!(plain.contains("line ref"), "plain: {plain}");
    assert!(plain.contains("#not_a_call"), "plain: {plain}");
    assert!(plain.contains("#tag_plain"), "plain: {plain}");
    assert!(plain.contains("ok"), "plain: {plain}");
    assert!(plain.contains("ref link"), "plain: {plain}");
    assert!(plain.contains("Guide"), "plain: {plain}");
    assert!(plain.contains("1 & 2"), "plain: {plain}");
    assert!(plain.contains("y = x^2"), "plain: {plain}");
    assert!(plain.contains("Inside Notist callout."), "plain: {plain}");

    let links: Vec<String> = doc.links().into_iter().map(|(u, _)| u.to_string()).collect();
    assert!(links.iter().any(|u| u == "https://github.com/org/repo/blob/main/a.rs#L1-L27"));
    assert!(links.iter().any(|u| u == "https://example.com#L10"));
    assert!(links.iter().any(|u| u == "guide"));
}

#[test]
fn markdown_supports_inline_and_block_attrs_plus_mid_doc_import_and_ide_analysis() {
    let ui = "fn badge(s: String) -> Inline { strong(text(s)) }";
    let md = r#"
# Summary

- @(collapsed: true)Section Group
  - [Intro](./intro.md)

#import "./ui.notc" as ui

@(id: "sec-1")
## Details #ui.badge("v2")
"#;
    let frontends: [&dyn DocumentFrontend; 2] = [&NotistFrontend, &MarkdownFrontend];
    let parsed_md = parse_file_with_frontends("doc.md", md, &frontends);
    assert!(parsed_md.diagnostics.is_empty(), "{:?}", parsed_md.diagnostics);

    let parsed_ui = notc::parse(ui);
    let project = notc::check_parsed_modules(&[
        ("ui.notc", &parsed_ui.program.root),
        ("doc.md", &parsed_md.program.root),
    ])
    .expect("checks cleanly");
    let doc = project.eval_document("doc.md").expect("evaluates");
    let BlockKind::Fragment(children) = &doc.node().kind else {
        panic!("expected fragment");
    };
    let BlockKind::List { items, .. } = &children[1].node().kind else {
        panic!("expected list");
    };
    let BlockKind::Fragment(item_blocks) = &items[0].node().kind else {
        panic!("expected item fragment");
    };
    let BlockKind::Paragraph(label) = &item_blocks[0].node().kind else {
        panic!("expected label paragraph");
    };
    assert_eq!(
        label.node().attrs.get("collapsed"),
        Some(&AttrValue::Bool(true))
    );
    assert_eq!(label.plain_text().trim(), "Section Group");

    // Verify WorkspaceAnalysis works with pluggable MarkdownFrontend
    let analysis = notc::ide::WorkspaceAnalysis::analyze_with_frontends(
        &[("ui.notc", ui), ("doc.md", md)],
        &frontends,
    );
    assert!(
        analysis.diagnostics().is_empty(),
        "ide diagnostics: {:?}",
        analysis.diagnostics()
    );
    let syms = analysis.document_symbols("doc.md");
    assert!(!syms.is_empty(), "expected markdown headings in outline");
}
