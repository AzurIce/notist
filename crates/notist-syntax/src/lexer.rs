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

/// The call-form state of an open code region: `#ident? (..)? [..]?`, each
/// slot optional but adjacent-only. `Args` tracks paren depth; the body slot
/// returns to markup (the body's extent is the parser's business).
#[derive(Clone, Copy)]
enum CodePhase {
    /// Just after `#`: expecting an identifier, `(`.
    Start,
    /// After the identifier: expecting `(` or `[`.
    AfterIdent,
    /// Inside the argument group.
    Args { depth: usize },
    /// After the argument group: expecting `[` or exit.
    AfterArgs,
}

/// The mode-stack lexer: markup by default; `@(` and `#ident`/`#(` push a
/// code region whose extent is lexically decidable (balanced parens; strings
/// are lexed as tokens inside, so their parens never count).
pub fn lex(src: &str) -> Lexed<'_> {
    let mut lx = Lexer {
        src,
        kinds: Vec::new(),
        starts: Vec::new(),
        stack: Vec::new(),
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
    stack: Vec<CodePhase>,
}

impl Lexer<'_> {
    fn run(&mut self) {
        let mut i = 0;
        while i < self.src.len() {
            self.starts.push(TextSize::new(i as u32));
            let (kind, len) = match self.stack.last() {
                Some(_) => self.lex_code(i),
                None => self.lex_markup(i),
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
                // code mode only on `@(` / `@!(`; otherwise `@` is plain text
                let mut j = i + 1;
                if self.src[j..].starts_with('!') {
                    j += 1;
                }
                if self.src[j..].starts_with('(') {
                    self.stack.push(CodePhase::Args { depth: 0 });
                }
                (SyntaxKind::At, 1)
            }
            '#' => {
                let next = self.src[i + 1..].chars().next();
                match next {
                    Some(c) if c.is_alphabetic() || c == '_' => self.stack.push(CodePhase::Start),
                    Some('(') => self.stack.push(CodePhase::Start),
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
                    (SyntaxKind::BlockComment, block_comment_len(rest))
                } else {
                    (SyntaxKind::Text, 1)
                }
            }
            '*' => (SyntaxKind::Star, 1),
            '_' => (SyntaxKind::Underscore, 1),
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
                        '=', '`', '/', '*', '_', '\\', '[', ']', '(', ')', '|', '#', '$', '-', '+',
                        '@', '!', ' ', '\t', '\n', '\r',
                    ])
                    .unwrap_or(rest.len());
                (SyntaxKind::Text, len)
            }
        }
    }

    // ── code mode ───────────────────────────────────────────────

    fn lex_code(&mut self, i: usize) -> (SyntaxKind, usize) {
        let src = self.src;
        let rest = &src[i..];
        let phase = *self.stack.last().unwrap();
        let ch = rest.chars().next().unwrap();

        // Phase boundaries: call-form slots only trigger when adjacent;
        // anything else pops back to markup.
        match phase {
            CodePhase::Start => match ch {
                '(' => {}
                c if c.is_alphabetic() || c == '_' => {}
                _ => {
                    self.stack.pop();
                    return self.lex_markup(i);
                }
            },
            CodePhase::AfterIdent | CodePhase::AfterArgs => match ch {
                '(' => {
                    self.stack.pop();
                    self.stack.push(CodePhase::Args { depth: 0 });
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
                    (SyntaxKind::BlockComment, block_comment_len(rest))
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
            '-' => (SyntaxKind::Minus, 1),
            ':' => (SyntaxKind::Colon, 1),
            ',' => (SyntaxKind::Comma, 1),
            '!' => (SyntaxKind::Bang, 1),
            '(' => {
                match self.stack.last_mut() {
                    Some(CodePhase::Start) => {
                        self.stack.pop();
                        self.stack.push(CodePhase::Args { depth: 1 });
                    }
                    Some(CodePhase::Args { depth }) => *depth += 1,
                    _ => {}
                }
                (SyntaxKind::LParen, 1)
            }
            ')' => {
                if let Some(CodePhase::Args { depth }) = self.stack.last_mut() {
                    *depth -= 1;
                    if *depth == 0 {
                        self.stack.pop();
                        self.stack.push(CodePhase::AfterArgs);
                    }
                }
                (SyntaxKind::RParen, 1)
            }
            '[' => (SyntaxKind::LBracket, 1),
            ']' => (SyntaxKind::RBracket, 1),
            c if c.is_alphabetic() || c == '_' => {
                let len = rest.len()
                    - rest
                        .trim_start_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '-')
                        .len();
                if let Some(CodePhase::Start) = self.stack.last() {
                    self.stack.pop();
                    self.stack.push(CodePhase::AfterIdent);
                }
                (SyntaxKind::Ident, len)
            }
            _ => {
                let len = rest
                    .find([
                        '"', ' ', '\t', '\n', '\r', '(', ')', '[', ']', ':', ',', '/', '-', '@',
                        '#', '$', '!',
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
    fn lex_code_string(&mut self, i: usize, hashes: usize) -> (SyntaxKind, usize) {
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
        loop {
            let Some(ch) = rest[n..].chars().next() else {
                break;
            };
            if ch == '\n' && !multiline {
                break;
            }
            if hashes == 0 && ch == '\\' && !multiline {
                n += 1;
                if n < rest.len() {
                    n += rest[n..].chars().next().map_or(0, |c| c.len_utf8());
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
        '=' | '*' | '_' | '\\' | '[' | ']' | '(' | ')' | '|' | '#' | '$' | '`'
    )
}

fn block_comment_len(rest: &str) -> usize {
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
    i
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
