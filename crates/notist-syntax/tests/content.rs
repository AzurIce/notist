use std::sync::atomic::{AtomicUsize, Ordering};

use notist_syntax::{
    code::ast::CodeCall,
    code::{parse_call_prefix, parse_content_literal},
    content::{ContentFlavor, ContentParser, ParsedContent},
    tree::SyntaxKind,
};
use rowan::{TextRange, TextSize};

/// An external dialect with its own terminator and a non-Rowan tree.
struct Pipes(AtomicUsize);
impl ContentParser for Pipes {
    fn parse_content(&self, source: &str, open: TextSize) -> ParsedContent {
        self.0.fetch_add(1, Ordering::SeqCst);
        let start = usize::from(open) + 1;
        let close = source[start..].find("|]").map(|i| start + i + 1);
        let body_end = close.unwrap_or(source.len());
        let end = body_end + usize::from(close.is_some());
        ParsedContent {
            range: TextRange::new(open, (end as u32).into()),
            body: TextRange::new((start as u32).into(), (body_end as u32).into()),
            flavor: ContentFlavor::Inline,
            closed: close.is_some(),
            syntax: Box::new(source[start..body_end].to_owned()),
            diagnostics: if close.is_some() {
                Vec::new()
            } else {
                vec![notist_syntax::Diagnostic {
                    span: TextRange::empty((end as u32).into()),
                    message: "unclosed pipe content".into(),
                }]
            },
        }
    }
}

#[test]
fn content_dialect_owns_termination_and_code_resumes_after_its_range() {
    let source = "#f([中文 \"unclosed code string `]` |], 7)[trailing body]";
    let frontend = Pipes(AtomicUsize::new(0));
    let parsed = parse_call_prefix(source, &frontend);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(frontend.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        parsed.syntax().text().to_string(),
        source[..source.find(")[trailing").unwrap() + 1]
    );
    let call = CodeCall::cast(parsed.syntax()).unwrap();
    assert_eq!(call.args().len(), 2);
    assert_eq!(call.args()[0].kind(), SyntaxKind::ContentLiteral);
    assert_eq!(call.args()[1].to_string(), "7");
    let content = &parsed.contents[0];
    let tree = content
        .syntax
        .as_ref()
        .as_any()
        .downcast_ref::<String>()
        .unwrap();
    assert_eq!(tree, "中文 \"unclosed code string `]` |");
    assert_eq!(
        &source[usize::from(content.body.start())..usize::from(content.body.end())],
        tree
    );
    assert!(
        call.args()[0]
            .clone()
            .into_node()
            .unwrap()
            .children()
            .next()
            .is_none()
    );
}

#[test]
fn every_incomplete_content_prefix_is_lossless_and_keeps_diagnostics_in_range() {
    let frontend = Pipes(AtomicUsize::new(0));
    let source = "[中文 \"quoted ] but opaque |] next";
    for end in (1..=source.len()).filter(|end| source.is_char_boundary(*end)) {
        let prefix = &source[..end];
        let parsed = parse_content_literal(prefix, &frontend);
        let consumed = usize::from(parsed.consumed());
        assert_eq!(parsed.syntax().text().to_string(), prefix[..consumed]);
        assert_eq!(parsed.contents.len(), 1);
        assert!(
            parsed
                .diagnostics
                .iter()
                .all(|d| usize::from(d.span.end()) <= end)
        );
    }
}
