//! Every input is lossless and no input panics. Parsing is a total function:
//! recovery may produce incomplete nodes, but never an unwind.

use notist_syntax::{code, markup};

#[test]
fn annotation_flavor_respects_its_starting_context() {
    use notist_syntax::tree::SyntaxKind;
    for (source, expected) in [
        ("text @!(id: \"x\")\nnext", SyntaxKind::Annotation),
        ("text @(id: \"x\")", SyntaxKind::Annotation),
        ("@(id: \"x\")\nnext", SyntaxKind::BlockAnnotation),
        ("@(id: \"x\") *next*", SyntaxKind::Annotation),
    ] {
        let parsed = markup::parse_document(source);
        assert_eq!(parsed.syntax().text().to_string(), source);
        let annotation = parsed
            .syntax()
            .descendants()
            .find(|node| {
                matches!(
                    node.kind(),
                    SyntaxKind::Annotation | SyntaxKind::BlockAnnotation
                )
            })
            .expect("annotation");
        assert_eq!(annotation.kind(), expected, "{source:?}");
    }
}

/// Deterministic xorshift so a failure reproduces exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

const PIECES: &[&str] = &[
    "#", "f", "for", " x in ", "(", ")", "[", "]", "<", ">", "\"", "a", " ", "\n", "\n\n", "|",
    "---", "*", "_", "$", "`", "```", "@", "!", ",", ":", "->", "-", "=", "//", "/*", "*/", "\\",
    ".", "1", "import", "{", "}", "r#\"", "p\"", "+", "~",
];

fn fragments(iterations: u64) -> impl Iterator<Item = String> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    (0..iterations).map(move |_| {
        let len = 1 + rng.next() % 14;
        (0..len)
            .map(|_| PIECES[(rng.next() % PIECES.len() as u64) as usize])
            .collect()
    })
}

#[test]
fn native_markup_fragments_are_lossless() {
    for source in fragments(50_000) {
        let parsed = markup::parse_document(&source);
        assert_eq!(
            parsed.syntax().text().to_string(),
            source,
            "lost source: {source:?}"
        );
    }
}

#[test]
fn code_fragments_are_lossless() {
    for source in fragments(50_000) {
        let parsed = code::parse_module(&source, &markup::MarkupContentParser);
        assert_eq!(
            parsed.syntax().text().to_string(),
            source,
            "lost source: {source:?}"
        );
    }
}

/// An unclosed fence inside an inline body used to reach `State::eat` past the
/// end of the bounded region and unwind.
#[test]
fn unclosed_fence_inside_a_body_is_a_diagnostic() {
    let source = "#[\n```\n ]";
    let parsed = markup::parse_document(source);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.message == "unclosed raw block"),
        "{:?}",
        parsed.diagnostics
    );
}

/// Every prefix of a source with an open construct stays lossless; recovery
/// decisions at the last line must not require the closing bracket.
#[test]
fn prefixes_of_unclosed_constructs_are_lossless() {
    for source in [
        "#f(a, b)[x *y*",
        "#[a b",
        "@(x: [a)\n\n= Title\n\ntext",
        "*a _b* c_",
        "a | b\n--- | ---\n1 | 2",
    ] {
        for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
            let prefix = &source[..end];
            let parsed = markup::parse_document(prefix);
            assert_eq!(parsed.syntax().text().to_string(), prefix, "{prefix:?}");
        }
    }
}
