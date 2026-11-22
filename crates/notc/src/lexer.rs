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
    Where,
    Match,
    Let,
    If,
    Else,
    True,
    False,
    Ident,
    Int,
    Float,
    String,
    Content,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semi,
    Dot,
    Eq,
    EqEq,
    FatArrow,
    Bang,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    AndAnd,
    OrOr,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Arrow,
    Eof,
}

impl TokenKind {
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Import => "`import`",
            Self::As => "`as`",
            Self::Fn => "`fn`",
            Self::Where => "`where`",
            Self::Match => "`match`",
            Self::Let => "`let`",
            Self::If => "`if`",
            Self::Else => "`else`",
            Self::True => "`true`",
            Self::False => "`false`",
            Self::Ident => "identifier",
            Self::Int => "integer literal",
            Self::Float => "float literal",
            Self::String => "string literal",
            Self::Content => "content literal",
            Self::LParen => "`(`",
            Self::RParen => "`)`",
            Self::LBrace => "`{`",
            Self::RBrace => "`}`",
            Self::Comma => "`,`",
            Self::Colon => "`:`",
            Self::Semi => "`;`",
            Self::Dot => "`.`",
            Self::Eq => "`=`",
            Self::EqEq => "`==`",
            Self::FatArrow => "`=>`",
            Self::Bang => "`!`",
            Self::BangEq => "`!=`",
            Self::Lt => "`<`",
            Self::LtEq => "`<=`",
            Self::Gt => "`>`",
            Self::GtEq => "`>=`",
            Self::AndAnd => "`&&`",
            Self::OrOr => "`||`",
            Self::Plus => "`+`",
            Self::Minus => "`-`",
            Self::Star => "`*`",
            Self::Slash => "`/`",
            Self::Percent => "`%`",
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
                i = scan_escaped_string(bytes, i, &mut tokens, &mut diagnostics);
            }
            b'[' => {
                i = scan_content_literal(bytes, i, &mut tokens, &mut diagnostics);
            }
            b'r' if i + 1 < len && bytes[i + 1] == b'#' => {
                i = scan_raw_string(bytes, i, &mut tokens, &mut diagnostics);
            }
            b'(' => push_single(&mut tokens, TokenKind::LParen, &mut i),
            b')' => push_single(&mut tokens, TokenKind::RParen, &mut i),
            b'{' => push_single(&mut tokens, TokenKind::LBrace, &mut i),
            b'}' => push_single(&mut tokens, TokenKind::RBrace, &mut i),
            b',' => push_single(&mut tokens, TokenKind::Comma, &mut i),
            b':' => push_single(&mut tokens, TokenKind::Colon, &mut i),
            b';' => push_single(&mut tokens, TokenKind::Semi, &mut i),
            b'.' => push_single(&mut tokens, TokenKind::Dot, &mut i),
            b'+' => push_single(&mut tokens, TokenKind::Plus, &mut i),
            b'*' => push_single(&mut tokens, TokenKind::Star, &mut i),
            b'/' => push_single(&mut tokens, TokenKind::Slash, &mut i),
            b'%' => push_single(&mut tokens, TokenKind::Percent, &mut i),
            b'-' => {
                if i + 1 < len && bytes[i + 1] == b'>' {
                    push_double(&mut tokens, TokenKind::Arrow, &mut i);
                } else {
                    push_single(&mut tokens, TokenKind::Minus, &mut i);
                }
            }
            b'=' => {
                if i + 1 < len && bytes[i + 1] == b'=' {
                    push_double(&mut tokens, TokenKind::EqEq, &mut i);
                } else if i + 1 < len && bytes[i + 1] == b'>' {
                    push_double(&mut tokens, TokenKind::FatArrow, &mut i);
                } else {
                    push_single(&mut tokens, TokenKind::Eq, &mut i);
                }
            }
            b'!' => {
                if i + 1 < len && bytes[i + 1] == b'=' {
                    push_double(&mut tokens, TokenKind::BangEq, &mut i);
                } else {
                    push_single(&mut tokens, TokenKind::Bang, &mut i);
                }
            }
            b'<' => {
                if i + 1 < len && bytes[i + 1] == b'=' {
                    push_double(&mut tokens, TokenKind::LtEq, &mut i);
                } else {
                    push_single(&mut tokens, TokenKind::Lt, &mut i);
                }
            }
            b'>' => {
                if i + 1 < len && bytes[i + 1] == b'=' {
                    push_double(&mut tokens, TokenKind::GtEq, &mut i);
                } else {
                    push_single(&mut tokens, TokenKind::Gt, &mut i);
                }
            }
            b'&' if i + 1 < len && bytes[i + 1] == b'&' => {
                push_double(&mut tokens, TokenKind::AndAnd, &mut i);
            }
            b'|' if i + 1 < len && bytes[i + 1] == b'|' => {
                push_double(&mut tokens, TokenKind::OrOr, &mut i);
            }
            b'0'..=b'9' => {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                    i += 1;
                }
                // Only treat `.` as a decimal point when immediately followed by a digit,
                // so `41.inc()` lexes as Int + Dot + Ident.
                if i + 1 < len && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                        i += 1;
                    }
                    tokens.push(Token {
                        kind: TokenKind::Float,
                        span: span(start, i),
                    });
                } else {
                    tokens.push(Token {
                        kind: TokenKind::Int,
                        span: span(start, i),
                    });
                }
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
                    "where" => TokenKind::Where,
                    "match" => TokenKind::Match,
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

fn push_single(tokens: &mut Vec<Token>, kind: TokenKind, i: &mut usize) {
    tokens.push(Token {
        kind,
        span: span(*i, *i + 1),
    });
    *i += 1;
}

fn push_double(tokens: &mut Vec<Token>, kind: TokenKind, i: &mut usize) {
    tokens.push(Token {
        kind,
        span: span(*i, *i + 2),
    });
    *i += 2;
}

fn scan_escaped_string(
    bytes: &[u8],
    start: usize,
    tokens: &mut Vec<Token>,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> usize {
    let len = bytes.len();
    let is_multiline =
        start + 2 < len && bytes[start + 1] == b'"' && bytes[start + 2] == b'"';
    if is_multiline {
        let mut i = start + 3;
        let mut closed = false;
        while i < len {
            if bytes[i] == b'\\' && i + 1 < len {
                i += 2;
            } else if i + 2 < len
                && bytes[i] == b'"'
                && bytes[i + 1] == b'"'
                && bytes[i + 2] == b'"'
            {
                i += 3;
                closed = true;
                break;
            } else {
                i += 1;
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
                message: "unterminated multiline string literal".into(),
            });
        }
        return i;
    }

    let mut i = start + 1;
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
    i
}

fn scan_raw_string(
    bytes: &[u8],
    start: usize,
    tokens: &mut Vec<Token>,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> usize {
    let len = bytes.len();
    let mut i = start + 1;
    let mut hashes = 0usize;
    while i < len && bytes[i] == b'#' {
        hashes += 1;
        i += 1;
    }
    if i >= len || bytes[i] != b'"' {
        diagnostics.push(ParseDiagnostic {
            span: span(start, i),
            message: "expected `\"` after raw string prefix".into(),
        });
        return i;
    }
    let is_multiline = i + 2 < len && bytes[i + 1] == b'"' && bytes[i + 2] == b'"';
    let quote_count = if is_multiline { 3 } else { 1 };
    i += quote_count;

    let mut closed = false;
    while i < len {
        if !is_multiline && bytes[i] == b'\n' {
            break;
        }
        if bytes[i] == b'"'
            && i + quote_count + hashes <= len
            && bytes[i..i + quote_count].iter().all(|&b| b == b'"')
            && bytes[i + quote_count..i + quote_count + hashes]
                .iter()
                .all(|&b| b == b'#')
        {
            i += quote_count + hashes;
            closed = true;
            break;
        }
        i += 1;
    }

    if closed {
        tokens.push(Token {
            kind: TokenKind::String,
            span: span(start, i),
        });
    } else {
        diagnostics.push(ParseDiagnostic {
            span: span(start, i),
            message: "unterminated raw string literal".into(),
        });
    }
    i
}

/// Scan a balanced `[...]` content literal token, skipping backslash escapes,
/// inline/fenced raw backticks, and strings inside embedded code parentheses.
fn scan_content_literal(
    bytes: &[u8],
    start: usize,
    tokens: &mut Vec<Token>,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> usize {
    let len = bytes.len();
    let mut i = start + 1;
    let mut depth = 1usize;
    let mut paren_depth = 0usize;

    while i < len && depth > 0 {
        match bytes[i] {
            b'\\' if i + 1 < len => {
                i += 2;
            }
            b'`' if paren_depth == 0 => {
                let tick_start = i;
                while i < len && bytes[i] == b'`' {
                    i += 1;
                }
                let tick_count = i - tick_start;
                while i < len {
                    if bytes[i] == b'`'
                        && i + tick_count <= len
                        && bytes[i..i + tick_count].iter().all(|&b| b == b'`')
                    {
                        i += tick_count;
                        break;
                    }
                    i += 1;
                }
            }
            b'(' => {
                paren_depth += 1;
                i += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                i += 1;
            }
            b'"' if paren_depth > 0 => {
                // Skip string literals inside embedded `#call("]")` / `@(k: "]")`.
                i += 1;
                while i < len {
                    match bytes[i] {
                        b'\\' if i + 1 < len => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        b'\n' => break,
                        _ => i += 1,
                    }
                }
            }
            b'[' => {
                depth += 1;
                i += 1;
            }
            b']' => {
                depth -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }

    if depth == 0 {
        tokens.push(Token {
            kind: TokenKind::Content,
            span: span(start, i),
        });
    } else {
        diagnostics.push(ParseDiagnostic {
            span: span(start, i),
            message: "unterminated content literal".into(),
        });
    }
    i
}

fn span(start: usize, end: usize) -> Span {
    Span::new(
        u32::try_from(start).expect("source exceeds u32 byte offset"),
        u32::try_from(end).expect("source exceeds u32 byte offset"),
    )
}
