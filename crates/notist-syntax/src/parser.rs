//! Shared parsing state for the native grammars.
//!
//! One token tape serves both syntax domains. The parser switches the tape's
//! lexical mode at a committed token boundary; neither grammar probes the other
//! through a source suffix.
use crate::lexed::{Lexed, Mode};
use crate::tree::SyntaxKind;
use crate::{Diagnostic, Parse, content::ParsedContent};
use rowan::{GreenNodeBuilder, TextRange};

pub(crate) struct State<'src> {
    pub lexed: Lexed<'src>,
    pub source: &'src str,
    pub pos: usize,
    pub builder: GreenNodeBuilder<'static>,
    pub diagnostics: Vec<Diagnostic>,
    pub contents: Vec<ParsedContent>,
    /// Code declaration grammar: keywords are reserved and `->` is an arrow.
    pub in_module: bool,
    /// Exclusive byte bound of the current container. Token indices change
    /// when a nested grammar switches lexical mode.
    pub limit: Option<usize>,
    /// The enclosing grammar is a Markup call: arguments end at a blank line.
    pub in_markup_args: bool,
}

impl<'src> State<'src> {
    /// Token 0 starts at byte `start`, which must be a char boundary.
    pub fn new(source: &'src str, start: usize, mode: Mode) -> Self {
        Self {
            lexed: Lexed::new(source, start, mode),
            source,
            pos: 0,
            builder: GreenNodeBuilder::new(),
            diagnostics: Vec::new(),
            contents: Vec::new(),
            in_module: false,
            limit: None,
            in_markup_args: false,
        }
    }

    pub fn kind_at(&self, i: usize) -> Option<SyntaxKind> {
        if self
            .limit
            .is_some_and(|limit| usize::from(self.lexed.offset(i)) >= limit)
        {
            None
        } else {
            self.lexed.kind(i)
        }
    }

    /// Byte offset of the cursor; the source length at the end.
    pub fn byte(&self) -> rowan::TextSize {
        self.lexed.offset(self.pos)
    }

    pub fn offset(&self, i: usize) -> rowan::TextSize {
        self.lexed.offset(i)
    }

    /// Consume the current token. At the end of the bounded region this is a
    /// no-op, so callers that consume an expected closer still finish the node.
    pub fn eat(&mut self) {
        if self
            .limit
            .is_some_and(|limit| usize::from(self.byte()) >= limit)
        {
            return;
        }
        let Some((kind, text)) = self.lexed.peek(self.pos) else {
            return;
        };
        let unclosed = if kind == SyntaxKind::BlockComment
            && !crate::code::lexer::block_comment_closed(text)
        {
            Some("unclosed block comment")
        } else if self.in_module
            && kind == SyntaxKind::Str
            && !crate::code::lexer::string_closed(text)
        {
            Some("unclosed string")
        } else if kind == SyntaxKind::PathLiteral && !crate::code::lexer::string_closed(&text[1..])
        {
            Some("unclosed path literal")
        } else {
            None
        };
        if let Some(message) = unclosed {
            self.diagnostics.push(Diagnostic {
                span: TextRange::new(self.lexed.offset(self.pos), self.lexed.offset(self.pos + 1)),
                message: message.into(),
            });
        }
        self.builder.token(kind.into(), text);
        self.pos += 1;
    }

    /// Continue at `end` after a construct consumed by another grammar: token
    /// `i` becomes the next token. Lookahead beyond the boundary is discarded.
    pub fn resume(&self, i: usize, end: rowan::TextSize, mode: Mode) {
        self.lexed.resume(i, end, mode);
    }

    /// Switch lexical mode at the current commit point. Lookahead beyond it is
    /// discarded.
    pub fn set_mode(&self, mode: Mode) {
        let byte = self.lexed.offset(self.pos);
        self.lexed.seek(self.pos, byte, mode);
    }

    pub fn finish(self) -> Parse {
        let mut parse = Parse::new(self.builder.finish(), self.diagnostics);
        parse.contents = self.contents;
        parse
    }
}
