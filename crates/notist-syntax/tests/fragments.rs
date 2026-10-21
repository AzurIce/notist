use notist_syntax::code::ast::{Annotation, CodeCall, Entry};
use notist_syntax::code::{parse_annotation, parse_call_header};

#[test]
fn call_headers_parse_without_a_document_or_a_synthetic_body() {
    let source = "#widgets.badge(r#\"[]()#@\"#, options: (enabled: true, items: (1, -2)))";
    let parsed = parse_call_header(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.syntax().text().to_string(), source);
    let call = CodeCall::cast(parsed.syntax()).unwrap();
    assert_eq!(call.name().as_deref(), Some("widgets.badge"));
    assert_eq!(call.args().len(), 2);
    assert!(Entry::cast(call.args()[1].clone().into_node().unwrap()).is_some());
    assert!(!call.has_body());

    let anonymous = parse_call_header("#");
    assert!(anonymous.diagnostics.is_empty());
    assert!(
        CodeCall::cast(anonymous.syntax())
            .unwrap()
            .callee()
            .is_none()
    );
}

#[test]
fn annotation_fragments_preserve_module_marker_and_nested_payload() {
    let source = "@!(id: \"root\", config: (values: (true, false), text: r#\"[)]\"#))";
    let parsed = parse_annotation(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.syntax().text().to_string(), source);
    let annotation = Annotation::cast(parsed.syntax()).unwrap();
    assert!(annotation.is_module());
    assert!(annotation.payload_dict().is_some());
}

#[test]
fn call_header_parser_leaves_content_syntax_to_the_frontend() {
    let source = "#badge()[*content*]";
    let parsed = parse_call_header(source);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|error| error.message == "unexpected trailing syntax")
    );
    assert!(!CodeCall::cast(parsed.syntax()).unwrap().has_body());
}

#[test]
fn common_headers_do_not_apply_a_document_frontends_block_boundaries() {
    let source = "#raw(\n\n\"value\"\n)";
    let parsed = parse_call_header(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert_eq!(CodeCall::cast(parsed.syntax()).unwrap().args().len(), 1);
}

#[test]
fn incomplete_fragments_preserve_every_source_prefix() {
    for (source, annotation) in [
        (
            "#名字.badge(\"text\", options: (nested: (1, 2), raw: r#\"[]\"#))",
            false,
        ),
        (
            "@!(id: \"root\", options: (nested: (1, 2), raw: r#\"[]\"#))",
            true,
        ),
    ] {
        for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
            let prefix = &source[..end];
            let parsed = if annotation {
                parse_annotation(prefix)
            } else {
                parse_call_header(prefix)
            };
            assert_eq!(parsed.syntax().text().to_string(), prefix);
        }
    }
}
