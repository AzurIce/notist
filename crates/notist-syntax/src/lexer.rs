use rowan::TextSize;

use crate::syntax::SyntaxKind;

/// Lexed source: SoA of token kinds and start offsets, plus one
/// sentinel entry equal to the source length. Tokens never own data; text is
/// sliced from the source on demand.
pub struct Lexed<'a> {
    src: &'a str,
    kinds: Vec<SyntaxKind>,
    starts: Vec<TextSize>,
}

impl<'a> Lexed<'a> {
    pub fn kind(&self, i: usize) -> Option<SyntaxKind> {
        self.kinds.get(i).copied()
    }

    pub fn text(&self, i: usize) -> &'a str {
        let start: usize = self.starts[i].into();
        let end: usize = self.starts[i + 1].into();
        &self.src[start..end]
    }

    pub fn len(&self, i: usize) -> usize {
        let start: usize = self.starts[i].into();
        let end: usize = self.starts[i + 1].into();
        end - start
    }

    /// Absolute byte offset of token `i` (O(1)); `offset(count())` is the
    /// source length.
    pub fn offset(&self, i: usize) -> TextSize {
        self.starts[i]
    }
}

pub fn lex(src: &str) -> Lexed<'_> {
    let mut kinds = Vec::new();
    let mut starts = Vec::new();
    let mut i = 0;
    while i < src.len() {
        starts.push(TextSize::new(i as u32));
        let rest = &src[i..];
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
                let prev_char = src[..i].chars().next_back();
                if rest[1..].starts_with('/') && prev_char != Some(':') {
                    let mut len = rest.find('\n').unwrap_or(rest.len());
                    if rest[..len].ends_with('\r') {
                        len -= 1;
                    }
                    (SyntaxKind::LineComment, len)
                } else if rest[1..].starts_with('*') {
                    let bytes = rest.as_bytes();
                    let mut depth = 0;
                    let mut n = 0;
                    while n < bytes.len() {
                        if bytes[n..].starts_with(b"/*") {
                            depth += 1;
                            n += 2;
                        } else if bytes[n..].starts_with(b"*/") {
                            depth -= 1;
                            n += 2;
                            if depth == 0 {
                                break;
                            }
                        } else {
                            n += 1;
                        }
                    }
                    (SyntaxKind::BlockComment, n)
                } else {
                    (SyntaxKind::Text, 1)
                }
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
            '-' => (SyntaxKind::Minus, 1),
            '+' => (SyntaxKind::Plus, 1),
            '@' => (SyntaxKind::At, 1),
            '!' => (SyntaxKind::Bang, 1),
            ' ' | '\t' => {
                let len = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                (SyntaxKind::Whitespace, len)
            }
            _ => {
                let len = rest
                    .find([
                        '=', '`', '/', '*', '_', '\\', '[', ']', '(', ')', '|', '#', '$', '-', '+',
                        '@', '!', ' ', '\t', '\n', '\r',
                    ])
                    .unwrap_or(rest.len());
                (SyntaxKind::Text, len)
            }
        };
        kinds.push(kind);
        i += len;
    }
    starts.push(TextSize::new(src.len() as u32));
    Lexed { src, kinds, starts }
}
