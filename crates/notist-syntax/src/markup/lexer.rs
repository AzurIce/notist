//! Native Markup tokenization and transitions into shared literal syntax.
use crate::code::lexer::literal_token;
use crate::lexed::Lexed;
use crate::tree::SyntaxKind;

/// Lexical states within Markup calls and annotations. These are not a
/// language switch; a call body uses the surrounding Markup token rules.
#[derive(Clone, Copy)]
enum CallPhase {
    /// Just after `#`: expecting an identifier, `(`.
    Start,
    /// After a path segment: expecting `.`, `(` or `[`.
    AfterIdent,
    /// After `.`: expecting another adjacent identifier.
    PathSegment,
    /// Inside the argument group.
    Args {
        depth: usize,
    },
    /// After the argument group: expecting `[` or exit.
    AfterArgs,
    Comptime {
        depth: usize,
        grouped: usize,
    },
}

pub fn lex(src: &str) -> Lexed<'_> {
    let mut lexer = Lexer {
        src,
        stack: Vec::new(),
    };
    Lexed::scan(src, |offset| {
        if lexer.stack.is_empty() {
            lexer.lex_markup(offset)
        } else {
            lexer.lex_call(offset)
        }
    })
}

struct Lexer<'a> {
    src: &'a str,
    stack: Vec<CallPhase>,
}
impl Lexer<'_> {
    fn lex_literal_token(&self, offset: usize) -> (SyntaxKind, usize) {
        literal_token(self.src, offset)
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
                    (SyntaxKind::BlockComment, literal_token(self.src, i).1)
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
            CallPhase::AfterIdent if rest.starts_with(".") => {
                self.stack.pop();
                self.stack.push(CallPhase::PathSegment);
                return (SyntaxKind::Dot, 1);
            }
            CallPhase::AfterIdent | CallPhase::AfterArgs => match ch {
                '<' => {
                    self.stack.pop();
                    self.stack.push(CallPhase::Comptime {
                        depth: 0,
                        grouped: 0,
                    });
                }
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
        if let Some(CallPhase::Comptime { grouped, .. }) = self.stack.last_mut() {
            match token.0 {
                SyntaxKind::LParen | SyntaxKind::LBrace => *grouped += 1,
                SyntaxKind::RParen | SyntaxKind::RBrace => *grouped = grouped.saturating_sub(1),
                _ => {}
            }
        }
        match token.0 {
            SyntaxKind::Less => {
                if let Some(CallPhase::Comptime { depth, grouped }) = self.stack.last_mut()
                    && *grouped == 0
                {
                    *depth += 1;
                }
            }
            SyntaxKind::Greater => {
                if let Some(CallPhase::Comptime { depth, grouped }) = self.stack.last_mut() {
                    if *grouped > 0 {
                        return token;
                    }
                    *depth -= 1;
                    if *depth == 0 {
                        self.stack.pop();
                        self.stack.push(CallPhase::AfterArgs);
                    }
                }
            }
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
}

/// Chars whose special meaning `\` cancels. `\`+newline is not among them:
/// it is the explicit paragraph break, which the parser decides.
fn is_escapable(c: char) -> bool {
    matches!(
        c,
        '=' | '*' | '_' | '~' | '\\' | '[' | ']' | '(' | ')' | '|' | '#' | '$' | '`' | '!'
    )
}
