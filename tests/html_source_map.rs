use notist_core::item::{Ctor, Item, Value};
use notist_html::{Renderer, SourceMappingKind};
use rowan::TextRange;

fn span(from: u32, to: u32) -> notist_core::item::Item {
    let mut item = Item::new(Ctor::Text, Default::default());
    item.span = TextRange::new(from.into(), to.into());
    item
}

#[test]
fn mapping_is_optional_reserved_and_attached_to_real_output() {
    let mut text = span(2, 5);
    text.fields.insert("text", Value::Str("abc".into()));
    text.attrs
        .insert("data-notist-node", Value::Str("fake".into()));
    let mut paragraph = Item::new(Ctor::Paragraph, text.span).with_children(vec![text]);
    paragraph.attrs.insert("id", Value::Str("user-id".into()));
    let root = Item::new(Ctor::Group, paragraph.span).with_children(vec![paragraph]);
    let default = Renderer::new().render_with_diagnostics(&root);
    assert_eq!(default.html, "<p id=\"user-id\">abc</p>");
    assert!(default.source_map.is_empty());
    let mapped = Renderer::new()
        .with_source_map()
        .render_with_diagnostics(&root);
    assert_eq!(
        mapped.html,
        "<p id=\"user-id\" data-notist-node=\"0\"><span data-notist-node=\"1\">abc</span></p>"
    );
    assert_eq!(mapped.source_map.len(), 2);
    assert_eq!(mapped.source_map[0].range, 2..5);
    assert_eq!(mapped.source_map[0].kind, SourceMappingKind::Block);
    assert_eq!(mapped.source_map[1].kind, SourceMappingKind::Inline);
    // Identifiers start fresh on every render and are not document identities.
    assert_eq!(Renderer::new().with_source_map().render(&root), mapped.html);
}

#[test]
fn structural_groups_are_flattened_and_empty_generated_cells_are_unmapped() {
    let mut row = span(1, 5);
    row.ctor = Ctor::TableRow;
    row.children = vec![Item::new(Ctor::TableCell, Default::default())];
    let group = Item::new(Ctor::Group, row.span).with_children(vec![row]);
    let table = Item::new(Ctor::Table, group.span).with_children(vec![group]);
    let result = Renderer::new()
        .with_source_map()
        .render_with_diagnostics(&table);
    assert_eq!(
        result.html,
        "<table data-notist-node=\"0\"><tbody><tr data-notist-node=\"1\"><td></td></tr></tbody></table>"
    );
    assert_eq!(result.source_map.len(), 2);
    assert!(result.diagnostics.is_empty());
}
