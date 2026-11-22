use crate::ast::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub span: Span,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Import,
    As,
    Fn,
    Let,
    If,
    Else,
    True,
    False,
    Ident,
    Int,
    String,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semi,
    Dot,
    Star,
    Eq,
    Plus,
    Arrow,
    Eof,
}

impl TokenKind {
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Import => "`import`",
            Self::As => "`as`",
            Self::Fn => "`fn`",
            Self::Let => "`let`",
            Self::If => "`if`",
            Self::Else => "`else`",
            Self::True => "`true`",
            Self::False => "`false`",
            Self::Ident => "identifier",
            Self::Int => "integer literal",
            Self::String => "string literal",
            Self::LParen => "`(`",
            Self::RParen => "`)`",
            Self::LBrace => "`{`",
            Self::RBrace => "`}`",
            Self::Comma => "`,`",
            Self::Colon => "`:`",
            Self::Semi => "`;`",
            Self::Dot => "`.`",
            Self::Star => "`*`",
            Self::Eq => "`=`",
            Self::Plus => "`+`",
            Self::Arrow => "`->`",
            Self::Eof => "end of file",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) span: Span,
}

pub(crate) fn lex(source: &str) -> (Vec<Token>, Vec<ParseDiagnostic>) {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::with_capacity(len / 4 + 1);
    let mut diagnostics = Vec::new();
    let mut i = 0usize;

    while i < len {
        let b = bytes[i];
        match b {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
            }
            b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                let start = i;
                i += 2;
                let mut depth = 1usize;
                while i < len && depth > 0 {
                    if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        depth += 1;
                        i += 2;
                    } else if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if depth > 0 {
                    diagnostics.push(ParseDiagnostic {
                        span: span(start, i),
                        message: "unterminated block comment".into(),
                    });
                }
            }
            b'"' => {
                let start = i;
                i += 1;
                let mut closed = false;
                while i < len {
                    match bytes[i] {
                        b'\\' if i + 1 < len => {
                            i += 2;
                        }
                        b'"' => {
                            i += 1;
                            closed = true;
                            break;
                        }
                        b'\n' => break,
                        _ => i += 1,
                    }
                }
                if closed {
                    tokens.push(Token {
                        kind: TokenKind::String,
                        span: span(start, i),
                    });
                } else {
                    diagnostics.push(ParseDiagnostic {
                        span: span(start, i),
                        message: "unterminated string literal".into(),
                    });
                }
            }
            b'(' => {
                tokens.push(Token {
                    kind: TokenKind::LParen,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b')' => {
                tokens.push(Token {
                    kind: TokenKind::RParen,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b'{' => {
                tokens.push(Token {
                    kind: TokenKind::LBrace,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b'}' => {
                tokens.push(Token {
                    kind: TokenKind::RBrace,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b',' => {
                tokens.push(Token {
                    kind: TokenKind::Comma,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b':' => {
                tokens.push(Token {
                    kind: TokenKind::Colon,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b';' => {
                tokens.push(Token {
                    kind: TokenKind::Semi,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b'.' => {
                tokens.push(Token {
                    kind: TokenKind::Dot,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b'*' => {
                tokens.push(Token {
                    kind: TokenKind::Star,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b'=' => {
                tokens.push(Token {
                    kind: TokenKind::Eq,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b'+' => {
                tokens.push(Token {
                    kind: TokenKind::Plus,
                    span: span(i, i + 1),
                });
                i += 1;
            }
            b'-' if i + 1 < len && bytes[i + 1] == b'>' => {
                tokens.push(Token {
                    kind: TokenKind::Arrow,
                    span: span(i, i + 2),
                });
                i += 2;
            }
            b'0'..=b'9' => {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                    i += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Int,
                    span: span(start, i),
                });
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let kind = match &source[start..i] {
                    "import" => TokenKind::Import,
                    "as" => TokenKind::As,
                    "fn" => TokenKind::Fn,
                    "let" => TokenKind::Let,
                    "if" => TokenKind::If,
                    "else" => TokenKind::Else,
                    "true" => TokenKind::True,
                    "false" => TokenKind::False,
                    _ => TokenKind::Ident,
                };
                tokens.push(Token {
                    kind,
                    span: span(start, i),
                });
            }
            _ => {
                let start = i;
                let ch = source[i..].chars().next().expect("valid utf-8 boundary");
                i += ch.len_utf8();
                diagnostics.push(ParseDiagnostic {
                    span: span(start, i),
                    message: format!("unexpected character `{ch}`"),
                });
            }
        }
    }

    let eof = span(len, len);
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: eof,
    });
    (tokens, diagnostics)
}

fn span(start: usize, end: usize) -> Span {
    Span::new(
        u32::try_from(start).expect("source exceeds u32 byte offset"),
        u32::try_from(end).expect("source exceeds u32 byte offset"),
    )
}
