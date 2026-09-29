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
            '`' => {
                let len = rest.len() - rest.trim_start_matches('`').len();
                (SyntaxKind::Backtick, len)
            }
            '/' => {
                if rest[1..].starts_with('/') {
                    let mut len = rest.find('\n').unwrap_or(rest.len());
                    if rest[..len].ends_with('\r') {
                        len -= 1;
                    }
                    (SyntaxKind::LineComment, len)
                } else if rest[1..].starts_with('*') {
                    let bytes = rest.as_bytes();
                    let mut depth = 0;
                    let mut i = 0;
                    while i < bytes.len() {
                        if bytes[i..].starts_with(b"/*") {
                            depth += 1;
                            i += 2;
                        } else if bytes[i..].starts_with(b"*/") {
                            depth -= 1;
                            i += 2;
                            if depth == 0 {
                                break;
                            }
                        } else {
                            i += 1;
                        }
                    }
                    (SyntaxKind::BlockComment, i)
                } else {
                    (SyntaxKind::Text, 1)
                }
            }
            ' ' | '\t' => {
                let len = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                (SyntaxKind::Whitespace, len)
            }
            _ => {
                let len = rest.find(['=', '`', '/', ' ', '\t', '\n', '\r']).unwrap_or(rest.len());
                (SyntaxKind::Text, len)
            }
        };
        tokens.push((kind, &rest[..len]));
        rest = &rest[len..];
    }
    tokens
}
