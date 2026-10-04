use notist::expr::Expr;

#[test]
fn desugar_preserves_qualified_targets_fields_and_children() {
    let (forest, _, diagnostics) = notist::desugar::lower_not(
        "#mermaid::diagram(\"graph\", theme: \"dark\")[#other::badge[x]]",
    );
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
    assert!(matches!(&args[0], Expr::Literal(notist::Value::Str(value), _) if value == "graph"));
    assert_eq!(
        fields.get("theme"),
        Some(&notist::Value::Str("dark".into()))
    );
    let Expr::Call { name, children, .. } = &children[0] else {
        panic!("expected badge");
    };
    assert_eq!(name, "other::badge");
    assert_eq!(children.len(), 1);
}
