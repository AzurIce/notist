use notist_core::expr::Expr;

#[test]
fn lowering_preserves_qualified_targets_fields_and_children() {
    let parsed = notist_syntax::markup::parse_document(
        "#mermaid.diagram(\"graph\", theme: \"dark\")[#other.badge[x]]",
    );
    assert!(parsed.diagnostics.is_empty());
    let mut diagnostics = Vec::new();
    let (forest, _, _) =
        notist_pipeline::frontend::markup::lower_document(&parsed, &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let Expr::Fragment { children, .. } = &forest[0] else {
        panic!("expected content fragment");
    };
    let diagram = &children[0];
    let Expr::Call {
        callee,
        args,
        children,
        ..
    } = diagram
    else {
        panic!("expected diagram");
    };
    assert!(matches!(callee.as_ref(), Expr::Member { name, .. } if name == "diagram"));
    assert!(
        matches!(args[0].value(), Expr::Literal(notist_core::item::Value::Str(value), _) if value == "graph")
    );
    assert_eq!(
        diagram.named_arg("theme").and_then(Expr::literal_value),
        Some(&notist_core::item::Value::Str("dark".into()))
    );
    let Expr::Call {
        callee, children, ..
    } = &children[0]
    else {
        panic!("expected badge");
    };
    assert!(matches!(callee.as_ref(), Expr::Member { name, .. } if name == "badge"));
    assert_eq!(children.len(), 1);
}
