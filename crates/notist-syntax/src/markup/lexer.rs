//! Native Markup tokenization.
//!
//! A scanner here is a pure function of the source and a byte offset: it never
//! consults the grammar, never keeps state between tokens, and never invokes a
//! parser. Lexical mode is chosen by the parser, which commits to a mode at a
//! token boundary; see [`crate::lexed`].
use crate::code::lexer::literal_token;
use crate::tree::SyntaxKind;

/// Scan one Markup token at a char boundary.
pub(crate) fn token(src: &str, i: usize) -> (SyntaxKind, usize) {
    let rest = &src[i..];
    let ch = rest.chars().next().unwrap_or('\0');
    match ch {
        '@' => (SyntaxKind::At, 1),
        '#' => (SyntaxKind::Hash, 1),
        '=' => {
            let len = rest.len() - rest.trim_start_matches('=').len();
            (SyntaxKind::Eq, len)
        }
        '`' => {
            let len = rest.len() - rest.trim_start_matches('`').len();
            (SyntaxKind::Backtick, len)
        }
        '/' => {
            if rest[1..].starts_with('/') && src[..i].chars().next_back() != Some(':') {
                let mut len = rest.find('\n').unwrap_or(rest.len());
                if rest[..len].ends_with('\r') {
                    len -= 1;
                }
                (SyntaxKind::LineComment, len)
            } else if rest[1..].starts_with('*') {
                (SyntaxKind::BlockComment, literal_token(src, i).1)
            } else {
                (SyntaxKind::Text, 1)
            }
        }
        '*' => (SyntaxKind::Star, 1),
        '_' => (SyntaxKind::Underscore, 1),
        '~' => (SyntaxKind::Tilde, 1),
        '\\' => {
            // `\`+escapable is one Escape token, so no scan ever sees an
            // escaped delimiter as a delimiter.
            match rest[1..].chars().next() {
                Some(c) if is_escapable(c) => (SyntaxKind::Escape, 1 + c.len_utf8()),
                _ => (SyntaxKind::Backslash, 1),
            }
        }
        '[' => (SyntaxKind::LBracket, 1),
        ']' => (SyntaxKind::RBracket, 1),
        '(' => (SyntaxKind::LParen, 1),
        ')' => (SyntaxKind::RParen, 1),
        '|' => (SyntaxKind::Pipe, 1),
        '$' => (SyntaxKind::Dollar, 1),
        '-' => (SyntaxKind::Minus, 1),
        '+' => (SyntaxKind::Plus, 1),
        '!' => (SyntaxKind::Bang, 1),
        '\n' => (SyntaxKind::Newline, 1),
        '\r' => {
            let len = if rest[1..].starts_with('\n') { 2 } else { 1 };
            (SyntaxKind::Newline, len)
        }
        ' ' | '\t' => {
            let len = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            (SyntaxKind::Whitespace, len)
        }
        _ => {
            // Prose runs across interior spaces and tabs so a sentence is one
            // token, not one per word. Whitespace that carries structure stays
            // separate: at a token start, after a marker, and before a newline
            // or another delimiter.
            let stop = rest
                .find([
                    '=', '`', '/', '*', '_', '~', '\\', '[', ']', '(', ')', '|', '#', '$', '-', '+',
                    '@', '!', '\n', '\r',
                ])
                .unwrap_or(rest.len());
            let len = rest[..stop].trim_end_matches([' ', '\t']).len();
            if len == 0 {
                (SyntaxKind::Whitespace, stop.max(1))
            } else {
                (SyntaxKind::Text, len)
            }
        }
    }
}

/// Chars whose special meaning `\` cancels. `\`+newline is not among them:
/// it is the explicit paragraph break, which the parser decides.
pub(crate) fn is_escapable(c: char) -> bool {
    matches!(
        c,
        '=' | '*' | '_' | '~' | '\\' | '[' | ']' | '(' | ')' | '|' | '#' | '$' | '`' | '!'
    )
}
