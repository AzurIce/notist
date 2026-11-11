use notist_syntax::{code::lexer::literal_token, tree::SyntaxKind};

#[test]
fn string_delimiters_preserve_utf8_and_token_boundaries() {
    for literal in [
        "\"\"",
        "\"中文🙂\"",
        "\"escaped \\\" quote\"",
        "\"escaped \\🙂 character\"",
        "\"\\\\\"",
        "\"\"\"\"\"\"",
        "\"\"\"中文\r\n🙂 \" and \"\" quotes\"\"\"",
        "r#\"中文\\🙂\"#",
        "r###\"a\"#b\"##c\"###",
        "r##\"\"\"中文\n\"\"\"#🙂\"\"\"##",
    ] {
        let source = format!("前缀{literal} trailing");
        assert_eq!(
            literal_token(&source, "前缀".len()),
            (SyntaxKind::Str, literal.len()),
            "{literal:?}"
        );
    }
}

#[test]
fn unterminated_strings_stop_at_the_same_line_or_utf8_boundary() {
    for (source, expected) in [
        ("\"中文🙂", "\"中文🙂"),
        ("\"中文\nnext", "\"中文"),
        ("\"中文\r\nnext", "\"中文"),
        ("\"中文\\\nnext", "\"中文\\"),
        ("\"中文\\\r\nnext", "\"中文\\"),
        ("\"中文\\", "\"中文\\"),
        ("r###\"中文\"##\nnext", "r###\"中文\"##"),
        ("r#\"中文\\\"# next", "r#\"中文\\\"#"),
        ("\"\"\"中文\n🙂\"\"", "\"\"\"中文\n🙂\"\""),
        ("r##\"\"\"中文\n🙂\"\"\"#", "r##\"\"\"中文\n🙂\"\"\"#"),
    ] {
        assert_eq!(
            literal_token(source, 0),
            (SyntaxKind::Str, expected.len()),
            "{source:?}"
        );
        assert!(source.is_char_boundary(expected.len()));
    }
}

#[test]
fn path_strings_remain_single_line() {
    for (source, expected) in [
        ("p\"中文🙂\" next", "p\"中文🙂\""),
        ("p\"\"\"中文\nnext", "p\"\"\"中文"),
        ("p\"中文\\\r\nnext", "p\"中文\\"),
    ] {
        assert_eq!(
            literal_token(source, 0),
            (SyntaxKind::PathLiteral, expected.len()),
            "{source:?}"
        );
    }
}
