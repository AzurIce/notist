use crate::syntax::SyntaxKind;

pub fn lex(src: &str) -> Vec<(SyntaxKind, &str)> {
    let mut tokens: Vec<(SyntaxKind, &str)> = Vec::new();
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
                let prev = tokens.last().and_then(|t| t.1.chars().last());
                if rest[1..].starts_with('/') && prev != Some(':') {
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
            '*' => (SyntaxKind::Star, 1),
            '_' => (SyntaxKind::Underscore, 1),
            '\\' => (SyntaxKind::Backslash, 1),
            '[' => (SyntaxKind::LBracket, 1),
            ']' => (SyntaxKind::RBracket, 1),
            '(' => (SyntaxKind::LParen, 1),
            ')' => (SyntaxKind::RParen, 1),
            '|' => (SyntaxKind::Pipe, 1),
            '#' => (SyntaxKind::Hash, 1),
            '$' => (SyntaxKind::Dollar, 1),
            _ => {
                let len = rest
                    .find(['=', '`', '/', '*', '_', '\\', '[', ']', '(', ')', '|', '#', '$', ' ', '\t', '\n', '\r'])
                    .unwrap_or(rest.len());
                (SyntaxKind::Text, len)
            }
        };
        tokens.push((kind, &rest[..len]));
        rest = &rest[len..];
    }
    tokens
}
