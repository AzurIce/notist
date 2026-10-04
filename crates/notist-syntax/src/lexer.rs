use rowan::TextSize;

use crate::syntax::SyntaxKind;

/// Lexed source: parallel arrays of token kinds and start offsets, plus one
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

/// Lexical states within Markup calls and annotations. These are not a
/// language switch; a call body uses the surrounding Markup token rules.
#[derive(Clone, Copy)]
enum CallPhase {
    /// Just after `#`: expecting an identifier, `(`.
    Start,
    /// After a path segment: expecting `::`, `(` or `[`.
    AfterIdent,
    /// After `::`: expecting another adjacent identifier.
    PathSegment,
    /// Inside the argument group.
    Args { depth: usize },
    /// After the argument group: expecting `[` or exit.
    AfterArgs,
}

/// Lex a Markup document, including its call and annotation literals.
pub fn lex(src: &str) -> Lexed<'_> {
    lex_with_mode(src, false)
}

/// Lex a Code module. The whole source uses Code token rules; brackets never
/// enter Markup and parentheses do not end the Code region.
pub fn lex_module(src: &str) -> Lexed<'_> {
    lex_with_mode(src, true)
}

fn lex_with_mode(src: &str, module: bool) -> Lexed<'_> {
    let mut lx = Lexer {
        src,
        kinds: Vec::new(),
        starts: Vec::new(),
        stack: Vec::new(),
        module,
    };
    lx.run();
    Lexed {
        src,
        kinds: lx.kinds,
        starts: lx.starts,
    }
}

struct Lexer<'a> {
    src: &'a str,
    kinds: Vec<SyntaxKind>,
    starts: Vec<TextSize>,
    stack: Vec<CallPhase>,
    module: bool,
}

impl Lexer<'_> {
    fn run(&mut self) {
        let mut i = 0;
        while i < self.src.len() {
            self.starts.push(TextSize::new(i as u32));
            let (kind, len) = if self.module {
                self.lex_literal_token(i)
            } else if self.stack.is_empty() {
                self.lex_markup(i)
            } else {
                self.lex_call(i)
            };
            self.kinds.push(kind);
            i += len;
        }
        self.starts.push(TextSize::new(self.src.len() as u32));
    }

    fn rest(&self, i: usize) -> &str {
        &self.src[i..]
    }

    // ── markup mode ─────────────────────────────────────────────

    fn lex_markup(&mut self, i: usize) -> (SyntaxKind, usize) {
        let src = self.src;
        let rest = &src[i..];
        match rest.chars().next().unwrap() {
            '@' => {
                // Annotation literal state only on `@(` / `@!(`.
                let mut j = i + 1;
                if self.src[j..].starts_with('!') {
                    j += 1;
                }
                if self.src[j..].starts_with('(') {
                    self.stack.push(CallPhase::Args { depth: 0 });
                }
                (SyntaxKind::At, 1)
            }
            '#' => {
                let next = self.src[i + 1..].chars().next();
                match next {
                    Some(c) if c.is_alphabetic() || c == '_' => self.stack.push(CallPhase::Start),
                    Some('(') => self.stack.push(CallPhase::Start),
                    _ => {}
                }
                (SyntaxKind::Hash, 1)
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
                let prev_char = self.src[..i].chars().next_back();
                if rest[1..].starts_with('/') && prev_char != Some(':') {
                    let mut len = rest.find('\n').unwrap_or(rest.len());
                    if rest[..len].ends_with('\r') {
                        len -= 1;
                    }
                    (SyntaxKind::LineComment, len)
                } else if rest[1..].starts_with('*') {
                    (SyntaxKind::BlockComment, block_comment_len(rest).0)
                } else {
                    (SyntaxKind::Text, 1)
                }
            }
            '*' => (SyntaxKind::Star, 1),
            '_' => (SyntaxKind::Underscore, 1),
            '~' => (SyntaxKind::Tilde, 1),
            '\\' => {
                // `\`+escapable is assembled into a single Escape token here,
                // so parser lookahead scans never see an escaped delimiter.
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
                let len = rest
                    .find([
                        '=', '`', '/', '*', '_', '~', '\\', '[', ']', '(', ')', '|', '#', '$', '-',
                        '+', '@', '!', ' ', '\t', '\n', '\r',
                    ])
                    .unwrap_or(rest.len());
                (SyntaxKind::Text, len)
            }
        }
    }

    // ── Markup call states ──────────────────────────────────────

    fn lex_call(&mut self, i: usize) -> (SyntaxKind, usize) {
        let src = self.src;
        let rest = &src[i..];
        let phase = *self.stack.last().unwrap();
        let ch = rest.chars().next().unwrap();

        // Phase boundaries: call-form slots only trigger when adjacent;
        // anything else pops back to markup.
        match phase {
            CallPhase::Start => match ch {
                '(' => {}
                c if c.is_alphabetic() || c == '_' => {}
                _ => {
                    self.stack.pop();
                    return self.lex_markup(i);
                }
            },
            CallPhase::PathSegment => {
                if !ch.is_alphabetic() && ch != '_' {
                    self.stack.pop();
                    return self.lex_markup(i);
                }
            }
            CallPhase::AfterIdent if rest.starts_with("::") => {
                self.stack.pop();
                self.stack.push(CallPhase::PathSegment);
                return (SyntaxKind::ColonColon, 2);
            }
            CallPhase::AfterIdent | CallPhase::AfterArgs => match ch {
                '(' => {
                    self.stack.pop();
                    self.stack.push(CallPhase::Args { depth: 0 });
                }
                '[' => {
                    self.stack.pop();
                    return (SyntaxKind::LBracket, 1);
                }
                _ => {
                    self.stack.pop();
                    return self.lex_markup(i);
                }
            },
            _ => {}
        }

        let token = self.lex_literal_token(i);
        match token.0 {
            SyntaxKind::LParen => match self.stack.last_mut() {
                Some(CallPhase::Start) => {
                    self.stack.pop();
                    self.stack.push(CallPhase::Args { depth: 1 });
                }
                Some(CallPhase::Args { depth }) => *depth += 1,
                _ => {}
            },
            SyntaxKind::RParen => {
                if let Some(CallPhase::Args { depth }) = self.stack.last_mut() {
                    *depth -= 1;
                    if *depth == 0 {
                        self.stack.pop();
                        self.stack.push(CallPhase::AfterArgs);
                    }
                }
            }
            SyntaxKind::Ident if matches!(phase, CallPhase::Start | CallPhase::PathSegment) => {
                self.stack.pop();
                self.stack.push(CallPhase::AfterIdent);
            }
            _ => {}
        }
        token
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
                    (SyntaxKind::Text, 1)
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
            '-' if self.module && rest.starts_with("->") => (SyntaxKind::Arrow, 2),
            '-' => (SyntaxKind::Minus, 1),
            ':' if rest.starts_with("::") => (SyntaxKind::ColonColon, 2),
            '=' if self.module => (SyntaxKind::Eq, 1),
            '?' if self.module => (SyntaxKind::Question, 1),
            ';' if self.module => (SyntaxKind::Semicolon, 1),
            '<' if self.module => (SyntaxKind::Less, 1),
            '>' if self.module => (SyntaxKind::Greater, 1),
            ':' => (SyntaxKind::Colon, 1),
            ',' => (SyntaxKind::Comma, 1),
            '!' => (SyntaxKind::Bang, 1),
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
                let kind = if self.module && &rest[..len] == "fn" {
                    SyntaxKind::FnKeyword
                } else {
                    SyntaxKind::Ident
                };
                (kind, len)
            }
            _ => {
                let len = rest
                    .find([
                        '"', ' ', '\t', '\n', '\r', '(', ')', '[', ']', ':', ',', '/', '-', '@',
                        '#', '$', '!', '=', '?', ';', '<', '>',
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
            if hashes == 0 && ch == '\\' && !multiline {
                n += 1;
                if n < rest.len() {
                    let escaped = rest[n..].chars().next().unwrap();
                    if matches!(escaped, '\n' | '\r') {
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

/// Chars whose special meaning `\` cancels. `\`+newline is not among them:
/// it is the explicit paragraph break, which the parser decides.
fn is_escapable(c: char) -> bool {
    matches!(
        c,
        '=' | '*' | '_' | '~' | '\\' | '[' | ']' | '(' | ')' | '|' | '#' | '$' | '`' | '!'
    )
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

pub(crate) fn block_comment_closed(text: &str) -> bool {
    block_comment_len(text).1
}

pub(crate) fn string_closed(text: &str) -> bool {
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
    if hashes == 0 && quotes == 1 {
        let backslashes = text[..text.len() - 1]
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
