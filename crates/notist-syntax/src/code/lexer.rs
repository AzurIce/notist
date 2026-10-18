//! Common Code and data-literal tokenization. Content frontends own their
//! text/delimiter rules and can delegate literal tokens to `literal_token`.
use crate::lexed::Lexed;
use crate::tree::SyntaxKind;

/// Tokenize shared call/annotation literals without reserving Code keywords.
pub fn lex(src: &str) -> Lexed<'_> {
    Lexed::scan(src, |offset| literal_token(src, offset))
}

/// Tokenize a Code declaration module.
pub fn lex_module(src: &str) -> Lexed<'_> {
    let scanner = Lexer { src, module: true };
    Lexed::scan(src, |offset| scanner.lex_literal_token(offset))
}

pub(crate) fn module_token(src: &str, offset: usize) -> (SyntaxKind, usize) {
    Lexer { src, module: true }.lex_literal_token(offset)
}

/// Scan one shared literal token at an absolute UTF-8 source offset.
pub fn literal_token(src: &str, offset: usize) -> (SyntaxKind, usize) {
    Lexer { src, module: false }.lex_literal_token(offset)
}

struct Lexer<'a> {
    src: &'a str,
    module: bool,
}
impl Lexer<'_> {
    fn rest(&self, offset: usize) -> &str {
        &self.src[offset..]
    }
    // Shared Code / Markup-literal token scanner, with no call-state effects.
    fn lex_literal_token(&self, i: usize) -> (SyntaxKind, usize) {
        let rest = self.rest(i);
        let ch = rest.chars().next().unwrap();

        match ch {
            '\n' => (SyntaxKind::Newline, 1),
            '\r' => {
                let len = if rest[1..].starts_with('\n') { 2 } else { 1 };
                (SyntaxKind::Newline, len)
            }
            ' ' | '\t' => {
                let len = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                (SyntaxKind::Whitespace, len)
            }
            '/' => {
                if rest[1..].starts_with('/') {
                    let mut len = rest.find('\n').unwrap_or(rest.len());
                    if rest[..len].ends_with('\r') {
                        len -= 1;
                    }
                    (SyntaxKind::LineComment, len)
                } else if rest[1..].starts_with('*') {
                    (SyntaxKind::BlockComment, block_comment_len(rest).0)
                } else {
                    (SyntaxKind::Slash, 1)
                }
            }
            '"' => self.lex_code_string(i, 0),
            'r' if raw_string_hashes(&self.src[i..]).is_some() => {
                let hashes = raw_string_hashes(&self.src[i..]).unwrap();
                self.lex_code_string(i, hashes)
            }
            '0'..='9' => {
                let mut len =
                    rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
                if rest[len..].starts_with('.')
                    && rest[len + 1..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit())
                {
                    len += 1;
                    len += rest[len..].len()
                        - rest[len..]
                            .trim_start_matches(|c: char| c.is_ascii_digit())
                            .len();
                }
                (SyntaxKind::Number, len)
            }
            '=' if rest.starts_with("==") => (SyntaxKind::EqEq, 2),
            '!' if rest.starts_with("!=") => (SyntaxKind::NotEq, 2),
            '<' if rest.starts_with("<=") => (SyntaxKind::LessEq, 2),
            '>' if rest.starts_with(">=") => (SyntaxKind::GreaterEq, 2),
            '&' if rest.starts_with("&&") => (SyntaxKind::AndAnd, 2),
            '|' if rest.starts_with("||") => (SyntaxKind::OrOr, 2),
            '=' if rest.starts_with("=>") => (SyntaxKind::FatArrow, 2),
            '{' => (SyntaxKind::LBrace, 1),
            '}' => (SyntaxKind::RBrace, 1),
            '+' => (SyntaxKind::Plus, 1),
            '*' => (SyntaxKind::Star, 1),
            '%' => (SyntaxKind::Percent, 1),
            '-' if self.module && rest.starts_with("->") => (SyntaxKind::Arrow, 2),
            '-' => (SyntaxKind::Minus, 1),
            ':' if rest.starts_with("::") => (SyntaxKind::ColonColon, 2),
            '=' => (SyntaxKind::Eq, 1),
            '?' => (SyntaxKind::Question, 1),
            ';' => (SyntaxKind::Semicolon, 1),
            '<' => (SyntaxKind::Less, 1),
            '>' => (SyntaxKind::Greater, 1),
            ':' => (SyntaxKind::Colon, 1),
            ',' => (SyntaxKind::Comma, 1),
            '!' => (SyntaxKind::Bang, 1),
            '#' => (SyntaxKind::Hash, 1),
            '@' => (SyntaxKind::At, 1),
            '(' => (SyntaxKind::LParen, 1),
            ')' => (SyntaxKind::RParen, 1),
            '[' => (SyntaxKind::LBracket, 1),
            ']' => (SyntaxKind::RBracket, 1),
            c if c.is_alphabetic() || c == '_' => {
                let len = rest
                    .char_indices()
                    .take_while(|(offset, c)| {
                        (c.is_alphanumeric() || *c == '_' || *c == '-')
                            && !(self.module && rest[*offset..].starts_with("->"))
                    })
                    .map(|(offset, c)| offset + c.len_utf8())
                    .last()
                    .unwrap();
                let kind = if self.module {
                    match &rest[..len] {
                        "fn" => SyntaxKind::FnKeyword,
                        "let" => SyntaxKind::LetKeyword,
                        "if" => SyntaxKind::IfKeyword,
                        "else" => SyntaxKind::ElseKeyword,
                        "match" => SyntaxKind::MatchKeyword,
                        "comptime" => SyntaxKind::ComptimeKeyword,
                        _ => SyntaxKind::Ident,
                    }
                } else {
                    SyntaxKind::Ident
                };
                (kind, len)
            }
            _ => {
                let len = rest
                    .find([
                        '"', ' ', '\t', '\n', '\r', '(', ')', '[', ']', ':', ',', '/', '-', '@',
                        '#', '$', '!', '=', '?', ';', '<', '>', '{', '}', '+', '*', '%', '&', '|',
                    ])
                    .unwrap_or(rest.len());
                if len == 0 {
                    (SyntaxKind::Text, ch.len_utf8())
                } else {
                    (SyntaxKind::Text, len)
                }
            }
        }
    }

    /// String literals: `".."` / `""".."""` / `r#".."#` / `r#""".."""#`.
    /// Unterminated runs to end of line (desugar diagnoses it).
    fn lex_code_string(&self, i: usize, hashes: usize) -> (SyntaxKind, usize) {
        let rest = self.rest(i);
        let mut j = hashes + 1; // opening quote after r#*
        if hashes > 0 {
            j += 1; // the r itself
        }
        let multiline = rest[j..].starts_with("\"\"");
        if multiline {
            j += 2;
        }
        let mut n = j;
        let closer_len = 1 + hashes + if multiline { 2 } else { 0 };
        while let Some(ch) = rest[n..].chars().next() {
            if matches!(ch, '\n' | '\r') && !multiline {
                break;
            }
            if hashes == 0 && ch == '\\' {
                n += 1;
                if n < rest.len() {
                    let escaped = rest[n..].chars().next().unwrap();
                    if !multiline && matches!(escaped, '\n' | '\r') {
                        break;
                    }
                    n += escaped.len_utf8();
                }
                continue;
            }
            if rest[n..].starts_with(&"\"".repeat(1 + if multiline { 2 } else { 0 }))
                && rest[n + 1 + if multiline { 2 } else { 0 }..].starts_with(&"#".repeat(hashes))
            {
                n += closer_len;
                return (SyntaxKind::Str, n);
            }
            n += ch.len_utf8();
        }
        (SyntaxKind::Str, n)
    }
}

fn block_comment_len(rest: &str) -> (usize, bool) {
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
    (i, depth == 0)
}

pub fn block_comment_closed(text: &str) -> bool {
    block_comment_len(text).1
}

pub fn string_closed(text: &str) -> bool {
    let hashes = if text.starts_with('r') {
        raw_string_hashes(text).unwrap_or(0)
    } else {
        0
    };
    let prefix = if hashes > 0 { hashes + 1 } else { 0 };
    let quotes = if text[prefix..].starts_with("\"\"\"") {
        3
    } else {
        1
    };
    let closer = format!("{}{}", "\"".repeat(quotes), "#".repeat(hashes));
    if text.len() < prefix + quotes + closer.len() || !text.ends_with(&closer) {
        return false;
    }
    if hashes == 0 {
        let backslashes = text[..text.len() - quotes]
            .chars()
            .rev()
            .take_while(|c| *c == '\\')
            .count();
        return backslashes % 2 == 0;
    }
    true
}

/// Start of a raw string `r#"..`; returns the hash count. Raw strings need at
/// least one `#`.
fn raw_string_hashes(rest: &str) -> Option<usize> {
    let after_r = &rest[1..];
    let hashes = after_r.bytes().take_while(|&b| b == b'#').count();
    if hashes >= 1 && after_r[hashes..].starts_with('"') {
        Some(hashes)
    } else {
        None
    }
}
