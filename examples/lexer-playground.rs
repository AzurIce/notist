use notist::syntax::{Lang, SyntaxKind};
use rowan::{GreenNodeBuilder, Language};

fn lex(src: &str) -> Vec<(SyntaxKind, &str)> {
    let mut tokens = Vec::new();
    let mut rest = src;
    while !rest.is_empty() {
        let (kind, len) = match rest.chars().next().unwrap() {
            '\n' => (SyntaxKind::Newline, 1),
            '\r' => {
                let len = if rest[1..].starts_with('\n') { 2 } else { 1 };
                (SyntaxKind::Newline, len)
            }
            '=' => {
                let len = rest.len() - rest.trim_start_matches('=').len();
                (SyntaxKind::Eq, len)
            }
            ' ' | '\t' => {
                let len = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                (SyntaxKind::Whitespace, len)
            }
            _ => {
                let len = rest.find(['=', ' ', '\t', '\n', '\r']).unwrap_or(rest.len());
                (SyntaxKind::Text, len)
            }
        };
        tokens.push((kind, &rest[..len]));
        rest = &rest[len..];
    }
    tokens
}

fn main() {
    let src = "= Title\n\nhello world\n";

    let mut builder = GreenNodeBuilder::new();
    builder.start_node(Lang::kind_to_raw(SyntaxKind::Document));
    for (kind, text) in lex(src) {
        builder.token(Lang::kind_to_raw(kind), text);
    }
    builder.finish_node();
    let green = builder.finish();

    let node = rowan::SyntaxNode::<Lang>::new_root(green);
    println!("{node:#?}");

    let rebuilt: String = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token().map(|t| t.text().to_string()))
        .collect();
    assert_eq!(rebuilt, src);
    println!("lossless: OK");
}
