use rowan::{GreenNode, GreenNodeBuilder, TextRange};

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
                SyntaxKind::Eq if self.peek(1) == Some(SyntaxKind::Whitespace) => self.heading(),
                SyntaxKind::Whitespace if self.at_blank_line() => self.eat_blank_line(),
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
                match self.tokens.get(i).map(|t| t.0) {
                    None | Some(SyntaxKind::Newline) => true,
                    Some(SyntaxKind::Eq) => {
                        self.tokens.get(i + 1).map(|t| t.0) == Some(SyntaxKind::Whitespace)
                    }
                    _ => false,
                }
            }
            _ => unreachable!(),
        }
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

    fn cur(&self) -> Option<SyntaxKind> {
        self.tokens.get(self.pos).map(|t| t.0)
    }

    fn peek(&self, offset: usize) -> Option<SyntaxKind> {
        self.tokens.get(self.pos + offset).map(|t| t.0)
    }

    fn eat(&mut self) {
        let (kind, text) = self.tokens[self.pos];
        self.builder.token(kind.into(), text);
        self.pos += 1;
    }
}
