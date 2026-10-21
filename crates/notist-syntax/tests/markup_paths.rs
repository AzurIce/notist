use notist_syntax::code::ast::CodeCall;
use notist_syntax::markup::parse_document;
use notist_syntax::tree::SyntaxKind;

#[test]
fn call_paths_keep_all_segments_and_existing_body_arguments() {
    let src =
        "#mermaid.diagram(\"graph TD\", theme: \"default\")[#other.badge[x]]\n#pkg.future.foo()";
    let parsed = parse_document(src);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.syntax().text().to_string(), src);
    let calls: Vec<_> = parsed
        .syntax()
        .descendants()
        .filter_map(CodeCall::cast)
        .collect();
    assert_eq!(
        calls
            .iter()
            .map(|call| call.name().unwrap())
            .collect::<Vec<_>>(),
        ["mermaid.diagram", "other.badge", "pkg.future.foo"]
    );
    let path = calls[0].callee().unwrap();
    assert_eq!(
        path.descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::Ident)
            .map(|t| t.text().to_string())
            .collect::<Vec<_>>(),
        ["mermaid", "diagram"]
    );
    assert_eq!(
        path.text_range(),
        rowan::TextRange::new(1.into(), 16.into())
    );
    assert_eq!(calls[0].args().len(), 2);
    assert!(calls[0].has_body());
    assert!(!calls[2].has_body());
}

#[test]
fn named_calls_and_node_calls_parse() {
    let parsed = parse_document("#callout[x] #node[group] #foo() #名字.函数()");
    let calls: Vec<_> = parsed
        .syntax()
        .descendants()
        .filter_map(CodeCall::cast)
        .collect();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0].name().as_deref(), Some("callout"));
    assert_eq!(calls[1].name().as_deref(), Some("node"));
    assert!(calls[1].has_body());
    assert_eq!(calls[2].name().as_deref(), Some("foo"));
    assert_eq!(calls[3].name().as_deref(), Some("名字.函数"));
}

#[test]
fn malformed_or_nonadjacent_paths_remain_text() {
    for src in [
        "#pkg.",
        "#pkg.()",
        "#pkg..foo()",
        "#pkg .foo()",
        "#pkg. foo()",
        "#pkg.foo ()",
        "#pkg.foo",
        "#pkg/*c*/.foo()",
    ] {
        let parsed = parse_document(src);
        assert_eq!(parsed.syntax().text().to_string(), src);
        assert_eq!(
            parsed
                .syntax()
                .descendants()
                .filter(|n| n.kind() == SyntaxKind::CodeCall)
                .count(),
            if matches!(
                src,
                "#pkg .foo()" | "#pkg.foo ()" | "#pkg.foo" | "#pkg/*c*/.foo()"
            ) {
                1
            } else {
                0
            },
            "{src}"
        );
    }
}
