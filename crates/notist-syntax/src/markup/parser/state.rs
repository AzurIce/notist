//! Native parser storage. Code parsing is invoked through source fragments.
use crate::tree::SyntaxKind;
use crate::{Diagnostic, Parse, content::ParsedContent, lexed::Lexed};
use rowan::{GreenNodeBuilder, TextRange};

pub(super) struct State<'a> {
    pub lexed: Lexed<'a>,
    pub source: &'a str,
    pub pos: usize,
    pub builder: GreenNodeBuilder<'static>,
    pub diagnostics: Vec<Diagnostic>,
    pub contents: Vec<ParsedContent>,
    pub limit: Option<usize>,
}

impl<'a> State<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            lexed: crate::markup::lexer::lex(source),
            source,
            pos: 0,
            builder: GreenNodeBuilder::new(),
            diagnostics: Vec::new(),
            contents: Vec::new(),
            limit: None,
        }
    }

    pub fn kind_at(&self, i: usize) -> Option<SyntaxKind> {
        if self.limit.is_some_and(|limit| i >= limit) {
            None
        } else {
            self.lexed.kind(i)
        }
    }

    pub fn eat(&mut self) {
        let kind = self.kind_at(self.pos).unwrap();
        let text = self.lexed.text(self.pos);
        if kind == SyntaxKind::BlockComment && !crate::code::lexer::block_comment_closed(text) {
            self.diagnostics.push(Diagnostic {
                span: TextRange::new(self.lexed.offset(self.pos), self.lexed.offset(self.pos + 1)),
                message: "unclosed block comment".into(),
            });
        }
        self.builder.token(kind.into(), text);
        self.pos += 1;
    }

    pub fn finish(self) -> Parse {
        let mut parse = Parse::new(self.builder.finish(), self.diagnostics);
        parse.contents = self.contents;
        parse
    }
}
