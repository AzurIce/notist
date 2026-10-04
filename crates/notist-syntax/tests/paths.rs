use notist_syntax::syntax::SyntaxKind;
use notist_syntax::{ast::CodeCall, parse_document};

#[test]
fn call_paths_keep_all_segments_and_existing_body_arguments() {
    let src = "#mermaid::diagram(\"graph TD\", theme: \"default\")[#other::badge[x]]\n#pkg::future::foo()";
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
        ["mermaid::diagram", "other::badge", "pkg::future::foo"]
    );
    let path = calls[0].path().unwrap();
    assert_eq!(
        path.segments()
            .map(|t| t.text().to_string())
            .collect::<Vec<_>>(),
        ["mermaid", "diagram"]
    );
    assert_eq!(path.range(), rowan::TextRange::new(1.into(), 17.into()));
    assert_eq!(calls[0].args().len(), 2);
    assert!(calls[0].has_body());
    assert!(!calls[2].has_body());
}

#[test]
fn plain_and_anonymous_calls_remain_available() {
    let parsed = parse_document("#callout[x] #[group] #foo() #名字::函数()");
    let calls: Vec<_> = parsed
        .syntax()
        .descendants()
        .filter_map(CodeCall::cast)
        .collect();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0].name().as_deref(), Some("callout"));
    assert!(calls[1].path().is_none());
    assert!(calls[1].has_body());
    assert_eq!(calls[2].name().as_deref(), Some("foo"));
    assert_eq!(calls[3].name().as_deref(), Some("名字::函数"));
}

#[test]
fn malformed_or_nonadjacent_paths_remain_text() {
    for src in [
        "#pkg::",
        "#pkg::()",
        "#pkg::::foo()",
        "#pkg ::foo()",
        "#pkg:: foo()",
        "#pkg::foo ()",
        "#pkg::foo",
        "#pkg/*c*/::foo()",
    ] {
        let parsed = parse_document(src);
        assert_eq!(parsed.syntax().text().to_string(), src);
        assert_eq!(
            parsed
                .syntax()
                .descendants()
                .filter(|n| n.kind() == SyntaxKind::CodeCall)
                .count(),
            0,
            "{src}"
        );
    }
}
