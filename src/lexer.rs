use crate::syntax::SyntaxKind;

pub fn lex(src: &str) -> Vec<(SyntaxKind, &str)> {
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
