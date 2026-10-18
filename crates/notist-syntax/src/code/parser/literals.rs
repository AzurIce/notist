//! Shared paths, call headers, annotations and literal grammar.
use super::{Diagnostic, Parser};
use crate::tree::SyntaxKind;
use rowan::TextRange;

impl Parser<'_, '_> {
    pub(super) fn kind_at(&self, i: usize) -> Option<SyntaxKind> {
        self.state.lexed.kind(i)
    }
    pub(super) fn cur(&self) -> Option<SyntaxKind> {
        self.kind_at(self.state.pos)
    }
    pub(super) fn peek(&self, offset: usize) -> Option<SyntaxKind> {
        self.kind_at(self.state.pos + offset)
    }
    pub(super) fn eat(&mut self) {
        self.state.eat();
    }

    pub(super) fn content_literal(&mut self) {
        let open = self.state.lexed.offset(self.state.pos);
        let Some(frontend) = self.content else {
            self.state.diagnostics.push(Diagnostic {
                span: TextRange::empty(open),
                message: "content literals require a content parser".into(),
            });
            self.eat();
            return;
        };
        let parsed = frontend.parse_content(self.state.source, open);
        let end = parsed.range.end();
        let source = self.state.source;
        assert_eq!(
            parsed.range.start(),
            open,
            "content parser returned an invalid start"
        );
        assert!(
            end > open
                && usize::from(end) <= source.len()
                && source.is_char_boundary(usize::from(end)),
            "content parser returned an invalid end"
        );
        assert_eq!(parsed.body.start(), open + rowan::TextSize::new(1));
        assert_eq!(
            parsed.body.end(),
            end - rowan::TextSize::new(u32::from(parsed.closed))
        );
        if parsed.closed {
            assert_eq!(source.as_bytes()[usize::from(end) - 1], b']');
        }
        self.state
            .builder
            .start_node(SyntaxKind::ContentLiteral.into());
        self.state.builder.token(SyntaxKind::LBracket.into(), "[");
        self.state.builder.token(
            SyntaxKind::ContentText.into(),
            &source[usize::from(parsed.body.start())..usize::from(parsed.body.end())],
        );
        if parsed.closed {
            self.state.builder.token(SyntaxKind::RBracket.into(), "]");
        }
        self.state.builder.finish_node();
        self.state
            .diagnostics
            .extend(parsed.diagnostics.iter().cloned());
        self.state.lexed.resume(self.state.pos, end);
        self.state.contents.push(parsed);
    }

    /// Consume an already recognized call header, leaving its content body to the host.
    pub(super) fn call_head(&mut self, named: bool, has_args: bool) {
        self.eat(); // #
        if named {
            self.path();
        }
        if self.cur() == Some(SyntaxKind::Less) {
            self.expression_arguments(SyntaxKind::ComptimeArguments, SyntaxKind::Greater);
        }
        if has_args || self.cur() == Some(SyntaxKind::LParen) {
            self.eat();
            self.call_args();
            if self.cur() == Some(SyntaxKind::RParen) {
                self.eat();
            } else {
                let point = self.state.lexed.offset(self.state.pos);
                self.state.diagnostics.push(Diagnostic {
                    span: TextRange::new(point, point),
                    message: "unclosed call arguments".into(),
                });
            }
        }
    }
    /// Paths are adjacent identifier segments. A trailing separator invalidates
    /// the whole path rather than silently truncating the target name.
    pub(super) fn path_end(&self, from: usize) -> Option<usize> {
        if self.kind_at(from) != Some(SyntaxKind::Ident) {
            return None;
        }
        let mut end = from + 1;
        while self.kind_at(end) == Some(SyntaxKind::ColonColon) {
            if self.kind_at(end + 1) != Some(SyntaxKind::Ident) {
                return None;
            }
            end += 2;
        }
        Some(end)
    }

    pub(super) fn path(&mut self) {
        let end = self.path_end(self.state.pos).unwrap();
        self.state.builder.start_node(SyntaxKind::Path.into());
        while self.state.pos < end {
            self.eat();
        }
        self.state.builder.finish_node();
    }

    pub(super) fn annotation_contents(&mut self) {
        self.eat();
        if self.cur() == Some(SyntaxKind::Bang) {
            self.eat();
        }
        if self.cur() == Some(SyntaxKind::LParen) {
            let start = self.state.lexed.offset(self.state.pos);
            if self.literal_group() != Some(SyntaxKind::Dict) {
                let span = TextRange::new(start, self.state.lexed.offset(self.state.pos));
                self.state.diagnostics.push(Diagnostic {
                    span,
                    message: "annotation payload must be a dict literal".to_string(),
                });
            }
        } else {
            let point = self.state.lexed.offset(self.state.pos);
            self.state.diagnostics.push(Diagnostic {
                span: TextRange::new(point, point),
                message: "expected a dict literal after `@`".to_string(),
            });
        }
    }

    /// A comma-separated run of literal entries (`key: value` or bare
    /// values), shared by annotation payloads and call arguments.
    pub(super) fn call_args(&mut self) {
        loop {
            self.eat_trivia();
            match self.cur() {
                None | Some(SyntaxKind::RParen) => break,
                Some(SyntaxKind::Comma) => self.eat(),
                _ => {
                    let named = matches!(self.cur(), Some(SyntaxKind::Ident | SyntaxKind::Str))
                        && self.peek(1) == Some(SyntaxKind::Colon);
                    if named {
                        self.state.builder.start_node(SyntaxKind::Entry.into());
                        self.eat();
                        self.eat();
                    }
                    let start = self.state.pos;
                    self.expression(0);
                    if named {
                        self.state.builder.finish_node();
                    }
                    if self.state.pos == start && self.cur().is_some() {
                        self.eat();
                    }
                }
            }
        }
    }

    /// `key: value` gets an `Entry` node; a bare literal stands alone.
    /// Returns whether an entry was produced.
    pub(super) fn literal_entry_or_value(&mut self) -> bool {
        let is_key = matches!(self.cur(), Some(SyntaxKind::Ident) | Some(SyntaxKind::Str))
            && self.peek(1) == Some(SyntaxKind::Colon);
        if is_key {
            self.state.builder.start_node(SyntaxKind::Entry.into());
            self.eat();
            self.eat();
            self.eat_trivia();
            self.literal_value();
            self.state.builder.finish_node();
            true
        } else {
            self.literal_value();
            false
        }
    }

    pub(super) fn eat_trivia(&mut self) {
        while matches!(
            self.cur(),
            Some(SyntaxKind::Whitespace)
                | Some(SyntaxKind::Newline)
                | Some(SyntaxKind::LineComment)
                | Some(SyntaxKind::BlockComment)
        ) {
            self.eat();
        }
    }

    pub(super) fn literal_value(&mut self) {
        match self.cur() {
            Some(SyntaxKind::Str) | Some(SyntaxKind::Number) | Some(SyntaxKind::Ident) => {
                self.eat()
            }
            Some(SyntaxKind::Minus) if self.peek(1) == Some(SyntaxKind::Number) => {
                self.state.builder.start_node(SyntaxKind::Neg.into());
                self.eat();
                self.eat();
                self.state.builder.finish_node();
            }
            Some(SyntaxKind::LParen) => {
                self.literal_group();
            }
            Some(SyntaxKind::LBracket) if !self.state.in_module => self.content_literal(),
            _ => {
                let point = self.state.lexed.offset(self.state.pos);
                self.state.diagnostics.push(Diagnostic {
                    span: TextRange::new(point, point),
                    message: "expected a literal".to_string(),
                });
                if self.cur().is_some()
                    && !(self.state.in_module
                        && matches!(
                            self.cur(),
                            Some(
                                SyntaxKind::FnKeyword | SyntaxKind::Semicolon | SyntaxKind::RParen
                            )
                        ))
                {
                    self.eat();
                }
            }
        }
    }

    /// `(..)` — unit / array / dict / grouping, discriminated by content:
    /// any `Entry` child makes a dict, a lone value without comma is a
    /// transparent `Group`, otherwise an array. The node kind is attached
    /// retroactively via checkpoint. Returns the produced node kind.
    pub(super) fn literal_group(&mut self) -> Option<SyntaxKind> {
        if self.peek(1) == Some(SyntaxKind::RParen) {
            self.state.builder.start_node(SyntaxKind::Unit.into());
            self.eat();
            self.eat();
            self.state.builder.finish_node();
            return Some(SyntaxKind::Unit);
        }
        if self.peek(1) == Some(SyntaxKind::Colon) && self.peek(2) == Some(SyntaxKind::RParen) {
            self.state.builder.start_node(SyntaxKind::Dict.into());
            self.eat();
            self.eat();
            self.eat();
            self.state.builder.finish_node();
            return Some(SyntaxKind::Dict);
        }
        if self.peek(1) == Some(SyntaxKind::Comma) && self.peek(2) == Some(SyntaxKind::RParen) {
            self.state.builder.start_node(SyntaxKind::Array.into());
            self.eat();
            self.eat();
            self.eat();
            self.state.builder.finish_node();
            return Some(SyntaxKind::Array);
        }
        let checkpoint = self.state.builder.checkpoint();
        self.eat();
        let mut has_named = false;
        let mut values = 0usize;
        let mut had_comma = false;
        loop {
            self.eat_trivia();
            match self.cur() {
                None | Some(SyntaxKind::Semicolon | SyntaxKind::FnKeyword)
                    if self.state.in_module || self.cur().is_none() =>
                {
                    let point = self.state.lexed.offset(self.state.pos);
                    self.state.diagnostics.push(Diagnostic {
                        span: TextRange::new(point, point),
                        message: "unclosed group".to_string(),
                    });
                    break;
                }
                Some(SyntaxKind::RParen) => {
                    self.eat();
                    break;
                }
                Some(SyntaxKind::Comma) => {
                    self.eat();
                    had_comma = true;
                }
                _ => {
                    if self.literal_entry_or_value() {
                        has_named = true;
                    } else {
                        values += 1;
                    }
                }
            }
        }
        let kind = if has_named {
            SyntaxKind::Dict
        } else if values != 1 || had_comma {
            SyntaxKind::Array
        } else {
            SyntaxKind::Group
        };
        self.state.builder.start_node_at(checkpoint, kind.into());
        self.state.builder.finish_node();
        Some(kind)
    }
}
