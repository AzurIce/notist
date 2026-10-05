use notist_core::expr::Expr;

#[test]
fn lowering_preserves_qualified_targets_fields_and_children() {
    let parsed = notist_syntax::parse_document(
        "#mermaid::diagram(\"graph\", theme: \"dark\")[#other::badge[x]]",
    );
    assert!(parsed.diagnostics.is_empty());
    let document = notist_syntax::ast::Document::cast(parsed.syntax()).unwrap();
    let mut diagnostics = Vec::new();
    let (forest, _) = notist_lowering::lower_document(&document, &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let Expr::Call { children, .. } = &forest[0] else {
        panic!("expected paragraph");
    };
    let Expr::Call {
        name,
        args,
        fields,
        children,
        ..
    } = &children[0]
    else {
        panic!("expected diagram");
    };
    assert_eq!(name, "mermaid::diagram");
    assert!(
        matches!(&args[0], Expr::Literal(notist_core::item::Value::Str(value), _) if value == "graph")
    );
    assert_eq!(
        fields.get("theme"),
        Some(&notist_core::item::Value::Str("dark".into()))
    );
    let Expr::Call { name, children, .. } = &children[0] else {
        panic!("expected badge");
    };
    assert_eq!(name, "other::badge");
    assert_eq!(children.len(), 1);
}
