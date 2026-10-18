use notist::{Ctor, Item, Pipeline, Value};

fn analyze(extension: &str, src: &str) -> (Item, Vec<notist::Diagnostic>) {
    Pipeline::default()
        .analyze(
            format!("test.{extension}"),
            src,
            &notist::Context::default(),
        )
        .unwrap()
        .into_parts()
}

fn find(item: &Item, ctor: Ctor) -> Option<&Item> {
    item.find(|node| node.ctor == ctor)
}

#[test]
fn block_annotation_targets_the_first_block_without_a_wrapper() {
    let (root, diagnostics) = analyze("not", "@(id: \"c\")\n#callout[hi]");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let callout = find(&root, Ctor::Callout).unwrap();
    assert_eq!(callout.attrs.get("id"), Some(&Value::Str("c".into())));
    assert!(find(&root, Ctor::Node).is_none());
}

#[test]
fn block_annotation_targets_the_first_of_several_blocks() {
    let (root, diagnostics) = analyze("not", "@(id: \"first\")\n#callout[a]\n\n#callout[b]");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let callouts: Vec<_> = root
        .descendants()
        .filter(|node| node.ctor == Ctor::Callout)
        .collect();
    assert_eq!(callouts.len(), 2);
    assert_eq!(
        callouts[0].attrs.get("id"),
        Some(&Value::Str("first".into()))
    );
    assert!(callouts[1].attrs.is_empty());
}

#[test]
fn block_annotation_targets_the_first_source_fragment() {
    let (root, diagnostics) = analyze("not", "@(id: \"r\")\nbefore\n\n#callout[hi]\n\nafter");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let paragraph = find(&root, Ctor::Paragraph).unwrap();
    assert_eq!(paragraph.attrs.get("id"), Some(&Value::Str("r".into())));
    assert!(find(&root, Ctor::Node).is_none());
}

#[test]
fn inline_annotation_targets_the_adjacent_element() {
    for src in ["asd @(id: \"e\")#emph[x] asd", "@(id: \"e\")#emph[x]"] {
        let (root, diagnostics) = analyze("not", src);
        assert!(diagnostics.is_empty(), "{src:?}: {diagnostics:?}");
        let emph = find(&root, Ctor::Emph).unwrap();
        assert_eq!(emph.attrs.get("id"), Some(&Value::Str("e".into())));
        let paragraph = find(&root, Ctor::Paragraph).unwrap();
        assert!(paragraph.attrs.is_empty());
    }
}

#[test]
fn inline_annotation_requires_adjacency_and_keeps_the_paragraph() {
    let (root, diagnostics) = analyze("not", "asd @(id: \"x\") #emph[y] asd");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("inline annotation")),
        "{diagnostics:?}"
    );
    let paragraphs: Vec<_> = root
        .descendants()
        .filter(|node| node.ctor == Ctor::Paragraph)
        .collect();
    assert_eq!(paragraphs.len(), 1);
    assert!(find(&root, Ctor::Emph).unwrap().attrs.is_empty());
}

#[test]
fn dangling_block_annotation_is_diagnosed() {
    let (root, diagnostics) = analyze("not", "@(id: \"dangling\")\n");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("no following block")),
        "{diagnostics:?}"
    );
    assert!(root.children.is_empty());
}

#[test]
fn inline_module_annotation_is_diagnosed_without_leaking_document_attrs() {
    let (root, diagnostics) = analyze("not", "text @!(id: \"leak\")\nnext");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("document top")),
        "{diagnostics:?}"
    );
    assert!(root.attrs.get("id").is_none());
}

#[test]
fn heading_annotation_lands_on_the_section() {
    let (root, diagnostics) = analyze("not", "@(id: \"h\")\n= Title");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let section = find(&root, Ctor::Section).unwrap();
    assert_eq!(section.attrs.get("id"), Some(&Value::Str("h".into())));
    assert!(find(&root, Ctor::Heading).unwrap().attrs.is_empty());
}

#[test]
fn annotations_inside_call_bodies_attach_in_place() {
    let (root, diagnostics) = analyze("not", "#callout[\n@(id: \"inside\")\n#emph[x]\n]");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let paragraph = find(&root, Ctor::Paragraph).unwrap();
    assert_eq!(
        paragraph.attrs.get("id"),
        Some(&Value::Str("inside".into()))
    );

    let (root, diagnostics) = analyze("not", "#callout[\n@(id: \"inline\")#emph[x]\n]");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let emph = find(&root, Ctor::Emph).unwrap();
    assert_eq!(emph.attrs.get("id"), Some(&Value::Str("inline".into())));
}

#[test]
fn annotations_in_code_content_literals_attach_before_evaluation() {
    let module = notist::analyze_module(
        "demo",
        r#"
fn box() -> BlockContent {
    [
@(id: "x")
#callout[hi] ]
}
fn inline() -> InlineContent {
    [text @(id: "e")#emph[x] text]
}
"#,
    )
    .unwrap();
    let context = notist::Context::default().with_module(module).unwrap();
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("test.not", "#demo::box()\n\n#demo::inline()", &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let evaluation = pipeline.evaluate(&analysis, &context);
    assert!(
        evaluation.diagnostics.is_empty(),
        "{:?}",
        evaluation.diagnostics
    );
    let callout = evaluation
        .root
        .find(|node| node.ctor == Ctor::Callout)
        .unwrap();
    assert_eq!(callout.attrs.get("id"), Some(&Value::Str("x".into())));
    let emph = evaluation
        .root
        .find(|node| node.ctor == Ctor::Emph)
        .unwrap();
    assert_eq!(emph.attrs.get("id"), Some(&Value::Str("e".into())));
}

#[test]
fn late_module_annotation_does_not_leak_document_attrs() {
    for extension in ["not", "md"] {
        let (root, diagnostics) = analyze(extension, "text\n\n@!(id: \"late\")\nnext");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("precede all content")),
            "{extension}: {diagnostics:?}"
        );
        assert!(root.attrs.get("id").is_none(), "{extension}");
    }
}
