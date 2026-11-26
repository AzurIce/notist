use notist_core::{AttrValue, BlockKind, CustomValue, InlineKind, check_project};

#[test]
fn parses_checks_and_evaluates_md_document_with_imports_and_doc_attrs() {
    let ui = r#"
        fn badge(label: Inline) -> Inline {
            strong[⚡ #label]
        }

        fn note(body: Block) -> Block {
            callout("tip", body)
        }
    "#;
    let doc = r#"
@!(title: "Welcome Guide", draft: false)
#import "./ui.not" as ui

@(id: "top")
# Hello #ui.badge[Notist]

Visit [the guide](./other.md#intro) with ![logo](./logo.png).

- First item
- Second item

> Quoted **wisdom**.

---

#ui.note[
    Everything is statically checked!
]
"#;

    let project = check_project(&[("ui.not", ui), ("index.md", doc)])
        .expect("document project should check");
    let block = project
        .eval_document("index.md")
        .expect("document should evaluate");

    assert_eq!(
        block.node().attrs.get("title"),
        Some(&AttrValue::String("Welcome Guide".into()))
    );
    assert_eq!(block.node().attrs.get("draft"), Some(&AttrValue::Bool(false)));
    assert_eq!(block.headings(), vec![(1, "Hello ⚡ Notist".into())]);

    let links = block.links();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].0.as_ref(), "./other.md#intro");

    let images = block.images();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].0.as_ref(), "./logo.png");
    assert_eq!(images[0].1.as_ref(), "logo");

    let ids = block.ids();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].0.as_ref(), "top");

    let BlockKind::Fragment(children) = &block.node().kind else {
        panic!("expected root fragment, got {block}");
    };
    assert!(matches!(
        children[0].node().kind,
        BlockKind::Heading { level: 1, .. }
    ));
    assert!(matches!(children[1].node().kind, BlockKind::Paragraph(_)));
    assert!(matches!(
        children[2].node().kind,
        BlockKind::List { ordered: false, ref items, .. } if items.len() == 2
    ));
    assert!(matches!(children[3].node().kind, BlockKind::Quote(_)));
    assert!(matches!(children[4].node().kind, BlockKind::Divider));
    assert!(matches!(
        children[5].node().kind,
        BlockKind::Callout { ref kind, .. } if kind.as_ref() == "tip"
    ));

    // Verify paragraph contains both Link and Image.
    if let BlockKind::Paragraph(ref inl) = children[1].node().kind {
        let InlineKind::Fragment(parts) = &inl.node().kind else {
            panic!("expected inline fragment");
        };
        assert!(
            parts
                .iter()
                .any(|p| matches!(p.node().kind, InlineKind::Link { .. }))
        );
        assert!(
            parts
                .iter()
                .any(|p| matches!(p.node().kind, InlineKind::Image { .. }))
        );
    }
}

#[test]
fn rejects_type_errors_inside_document_calls() {
    let ui = "fn need_inline(x: Inline) -> Inline { strong(x) }";
    let bad_doc = r#"
#import "./ui.not" as ui

#ui.need_inline[
    A block passed to an inline parameter!
]
"#;

    let errors = check_project(&[("ui.not", ui), ("bad.md", bad_doc)]).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].module, "bad.md");
}

#[test]
fn supports_nested_lists_tables_math_and_bodyless_external_content_declarations() {
    let components = r#"
        fn badge(label: String, count: Int) -> Inline;
        fn panel(title: String, body: Block) -> Block;
    "#;
    let doc = r#"
#import "./components.not" as ui

# Spec

Inline math $E = mc^2$ and #ui.badge("beta", 2).

$ \sum_{i=1}^n i = \frac{n(n+1)}{2} $

| Name | Type |
| --- | --- |
| Inline | `Inline` |
| Block | `Block` |

- Parent
  - Child 1
  - Child 2
- Sibling

#ui.panel("Details")[
    See [ref](./other.md).
]
"#;

    let project = check_project(&[("components.not", components), ("doc.md", doc)])
        .expect("document with rich blocks and extern decls should check");
    let block = project.eval_document("doc.md").expect("should evaluate");

    // Links inside custom component block props are still discovered!
    assert_eq!(block.links().len(), 1);
    assert_eq!(block.links()[0].0.as_ref(), "./other.md");

    let BlockKind::Fragment(children) = &block.node().kind else {
        panic!("expected fragment, got {block}");
    };
    assert!(matches!(
        children[0].node().kind,
        BlockKind::Heading { .. }
    ));

    // Paragraph contains Math + Custom Inline
    let BlockKind::Paragraph(ref inl) = children[1].node().kind else {
        panic!("expected paragraph");
    };
    let InlineKind::Fragment(ref parts) = inl.node().kind else {
        panic!("expected inline fragment");
    };
    assert!(
        parts
            .iter()
            .any(|p| matches!(p.node().kind, InlineKind::Math(ref m) if m.as_ref() == "E = mc^2"))
    );
    assert!(parts.iter().any(|p| matches!(
        p.node().kind,
        InlineKind::Custom(ref c)
            if c.module.as_ref() == "components.not"
            && c.name.as_ref() == "badge"
            && c.props.len() == 2
    )));

    // Block Math
    assert!(matches!(
        children[2].node().kind,
        BlockKind::Math(ref m) if m.contains("\\sum_{i=1}^n")
    ));

    // Table
    let BlockKind::Table { headers, rows, .. } = &children[3].node().kind else {
        panic!("expected Table, got {}", children[3]);
    };
    assert_eq!(headers.len(), 2);
    assert_eq!(rows.len(), 2);
    assert_eq!(headers[0].plain_text(), "Name");
    assert_eq!(rows[1][1].plain_text(), "Block");

    // Nested List
    let BlockKind::List {
        ordered: false,
        items,
        ..
    } = &children[4].node().kind
    else {
        panic!("expected List, got {}", children[4]);
    };
    assert_eq!(items.len(), 2);
    let BlockKind::Fragment(parent_parts) = &items[0].node().kind else {
        panic!(
            "expected nested list inside first list item, got {}",
            items[0]
        );
    };
    assert!(matches!(
        parent_parts[0].node().kind,
        BlockKind::Paragraph(_)
    ));
    assert!(matches!(
        parent_parts[1].node().kind,
        BlockKind::List { ordered: false, ref items, .. } if items.len() == 2
    ));

    // Custom Block
    let BlockKind::Custom(ref panel) = children[5].node().kind else {
        panic!("expected Custom block, got {}", children[5]);
    };
    assert_eq!(panel.module.as_ref(), "components.not");
    assert_eq!(panel.name.as_ref(), "panel");
    assert_eq!(panel.props[0].0.as_ref(), "title");
    assert_eq!(panel.props[0].1, CustomValue::String("Details".into()));
    assert!(matches!(panel.props[1].1, CustomValue::Block(_)));
}
