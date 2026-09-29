use rowan::{GreenNode, GreenNodeBuilder, TextRange, TextSize};

use crate::lexer::lex;
use crate::syntax::{SyntaxKind, SyntaxNode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: TextRange,
    pub message: String,
}

pub struct Parse {
    green: GreenNode,
    pub diagnostics: Vec<Diagnostic>,
}

impl Parse {
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }
}

pub fn parse(src: &str) -> Parse {
    Parser::new(src).run()
}

struct Parser<'a> {
    tokens: Vec<(SyntaxKind, &'a str)>,
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            tokens: lex(src),
            pos: 0,
            builder: GreenNodeBuilder::new(),
            diagnostics: Vec::new(),
        }
    }

    fn run(mut self) -> Parse {
        self.builder.start_node(SyntaxKind::Document.into());
        while let Some(kind) = self.cur() {
            match kind {
                SyntaxKind::Newline => self.eat(),
                SyntaxKind::Eq if self.at_heading_marker(0) => self.heading(),
                SyntaxKind::Backtick if self.at_fence(0) => self.raw_block(),
                SyntaxKind::Whitespace if self.at_blank_line() => self.eat_blank_line(),
                SyntaxKind::LineComment | SyntaxKind::BlockComment => self.eat(),
                _ => self.paragraph(),
            }
        }
        self.builder.finish_node();
        Parse {
            green: self.builder.finish(),
            diagnostics: self.diagnostics,
        }
    }

    fn heading(&mut self) {
        self.builder.start_node(SyntaxKind::Heading.into());
        self.eat();
        self.eat();
        while let Some(kind) = self.cur() {
            if kind == SyntaxKind::Newline {
                break;
            }
            self.eat();
        }
        self.builder.finish_node();
    }

    fn raw_block(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.builder.start_node(SyntaxKind::Raw.into());
        let fence_len = self.tokens[self.pos].1.len();
        let fence_start = self.pos;
        self.eat();
        while let Some(kind) = self.cur() {
            if kind == SyntaxKind::Newline {
                break;
            }
            self.eat();
        }
        if self.cur() == Some(SyntaxKind::Newline) {
            self.eat();
        }
        let mut closed = false;
        loop {
            match self.cur() {
                None => break,
                Some(SyntaxKind::Backtick) if self.tokens[self.pos].1.len() >= fence_len => {
                    self.eat();
                    while let Some(kind) = self.cur() {
                        if kind == SyntaxKind::Newline {
                            break;
                        }
                        self.eat();
                    }
                    closed = true;
                    break;
                }
                Some(_) => {
                    while let Some(kind) = self.cur() {
                        if kind == SyntaxKind::Newline {
                            break;
                        }
                        self.eat();
                    }
                    if self.cur() == Some(SyntaxKind::Newline) {
                        self.eat();
                    }
                }
            }
        }
        self.builder.finish_node();
        if !closed {
            self.diagnostics.push(Diagnostic {
                span: TextRange::new(self.offset_at(fence_start), self.offset_at(self.pos)),
                message: "unclosed raw block".to_string(),
            });
            self.builder.start_node_at(checkpoint, SyntaxKind::Error.into());
            self.builder.finish_node();
        }
    }

    fn paragraph(&mut self) {
        self.builder.start_node(SyntaxKind::Paragraph.into());
        loop {
            while let Some(kind) = self.cur() {
                if kind == SyntaxKind::Newline {
                    break;
                }
                self.eat();
            }
            if self.line_ends_block() {
                break;
            }
            self.eat();
        }
        self.builder.finish_node();
    }

    fn line_ends_block(&self) -> bool {
        match self.cur() {
            None => true,
            Some(SyntaxKind::Newline) => {
                let mut i = self.pos + 1;
                while self.tokens.get(i).map(|t| t.0) == Some(SyntaxKind::Whitespace) {
                    i += 1;
                }
                if matches!(self.tokens.get(i).map(|t| t.0), None | Some(SyntaxKind::Newline)) {
                    return true;
                }
                let next = self.tokens.get(self.pos + 1);
                match next.map(|t| t.0) {
                    Some(SyntaxKind::Eq) => {
                        self.tokens.get(self.pos + 2).map(|t| t.0)
                            == Some(SyntaxKind::Whitespace)
                    }
                    Some(SyntaxKind::Backtick) => next.is_some_and(|t| t.1.len() >= 3),
                    _ => false,
                }
            }
            _ => unreachable!(),
        }
    }

    fn at_heading_marker(&self, ws_skip: usize) -> bool {
        self.peek(ws_skip) == Some(SyntaxKind::Eq)
            && self.peek(ws_skip + 1) == Some(SyntaxKind::Whitespace)
    }

    fn at_fence(&self, ws_skip: usize) -> bool {
        self.peek(ws_skip) == Some(SyntaxKind::Backtick)
            && self.tokens.get(self.pos + ws_skip).is_some_and(|t| t.1.len() >= 3)
    }

    fn at_blank_line(&self) -> bool {
        let mut i = self.pos;
        while self.tokens.get(i).map(|t| t.0) == Some(SyntaxKind::Whitespace) {
            i += 1;
        }
        matches!(self.tokens.get(i).map(|t| t.0), None | Some(SyntaxKind::Newline))
    }

    fn eat_blank_line(&mut self) {
        while self.cur() == Some(SyntaxKind::Whitespace) {
            self.eat();
        }
    }

    fn offset_at(&self, pos: usize) -> TextSize {
        let bytes: usize = self.tokens[..pos].iter().map(|t| t.1.len()).sum();
        TextSize::new(bytes as u32)
    }

    fn cur(&self) -> Option<SyntaxKind> {
        self.tokens.get(self.pos).map(|t| t.0)
    }

    fn peek(&self, offset: usize) -> Option<SyntaxKind> {
        self.tokens.get(self.pos + offset).map(|t| t.0)
    }

    fn eat(&mut self) {
        let (kind, text) = self.tokens[self.pos];
        if kind == SyntaxKind::BlockComment && !text.ends_with("*/") {
            self.diagnostics.push(Diagnostic {
                span: TextRange::new(self.offset_at(self.pos), self.offset_at(self.pos + 1)),
                message: "unclosed block comment".to_string(),
            });
        }
        self.builder.token(kind.into(), text);
        self.pos += 1;
    }
}
