//! Code modules and syntax fragments. Concrete content grammars are injected.
use std::cell::{Cell, RefCell};

use crate::code::lexer::{literal_token, module_token};
use crate::content::{ContentParser, ParsedContent};
use crate::tree::SyntaxKind;
use crate::{Diagnostic, Parse};
use rowan::{GreenNodeBuilder, TextRange, TextSize};

mod expressions;
mod literals;
mod module;

#[derive(Clone, Copy)]
struct Token {
    kind: SyntaxKind,
    range: TextRange,
}

/// Lex only requested Code tokens. A content callback can resume at its end
/// without applying Code string/comment rules to the opaque payload.
struct Tokens<'a> {
    source: &'a str,
    module: bool,
    tokens: RefCell<Vec<Token>>,
    next: Cell<usize>,
}

impl<'a> Tokens<'a> {
    fn ensure(&self, i: usize) {
        while self.tokens.borrow().len() <= i && self.next.get() < self.source.len() {
            let start = self.next.get();
            let (kind, len) = if self.module {
                module_token(self.source, start)
            } else {
                literal_token(self.source, start)
            };
            let end = start + len;
            self.tokens.borrow_mut().push(Token {
                kind,
                range: TextRange::new((start as u32).into(), (end as u32).into()),
            });
            self.next.set(end);
        }
    }

    fn kind(&self, i: usize) -> Option<SyntaxKind> {
        self.ensure(i);
        self.tokens.borrow().get(i).map(|token| token.kind)
    }

    fn offset(&self, i: usize) -> TextSize {
        self.ensure(i);
        self.tokens
            .borrow()
            .get(i)
            .map_or((self.source.len() as u32).into(), |token| {
                token.range.start()
            })
    }

    fn text(&self, i: usize) -> &'a str {
        self.ensure(i);
        let range = self.tokens.borrow()[i].range;
        &self.source[usize::from(range.start())..usize::from(range.end())]
    }

    fn resume(&mut self, i: usize, end: TextSize) {
        self.tokens.get_mut().truncate(i);
        self.next.set(usize::from(end));
    }
}

struct ParserState<'a> {
    lexed: Tokens<'a>,
    source: &'a str,
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    diagnostics: Vec<Diagnostic>,
    contents: Vec<ParsedContent>,
    in_module: bool,
}

impl<'a> ParserState<'a> {
    fn new(source: &'a str, in_module: bool) -> Self {
        Self {
            lexed: Tokens {
                source,
                module: true,
                tokens: RefCell::new(Vec::new()),
                next: Cell::new(0),
            },
            source,
            pos: 0,
            builder: GreenNodeBuilder::new(),
            diagnostics: Vec::new(),
            contents: Vec::new(),
            in_module,
        }
    }

    fn finish(self) -> Parse {
        Parse {
            green: self.builder.finish(),
            diagnostics: self.diagnostics,
            contents: self.contents,
        }
    }

    fn eat(&mut self) {
        let kind = self.lexed.kind(self.pos).unwrap();
        let text = self.lexed.text(self.pos);
        let unclosed = if kind == SyntaxKind::BlockComment
            && !crate::code::lexer::block_comment_closed(text)
        {
            Some("unclosed block comment")
        } else if self.in_module
            && kind == SyntaxKind::Str
            && !crate::code::lexer::string_closed(text)
        {
            Some("unclosed string")
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
}

struct Parser<'src, 'content> {
    state: ParserState<'src>,
    content: Option<&'content dyn ContentParser>,
    in_comptime_args: usize,
    expression_depth: usize,
}

impl<'src, 'content> Parser<'src, 'content> {
    fn new(
        source: &'src str,
        in_module: bool,
        content: Option<&'content dyn ContentParser>,
    ) -> Self {
        Self {
            state: ParserState::new(source, in_module),
            content,
            in_comptime_args: 0,
            expression_depth: 0,
        }
    }

    fn call_fragment(&mut self) {
        self.state.builder.start_node(SyntaxKind::CodeCall.into());
        if self.cur() == Some(SyntaxKind::Hash) {
            let named = self.path_end(self.state.pos + 1).is_some();
            let end = if named {
                self.path_end(self.state.pos + 1).unwrap()
            } else {
                self.state.pos + 1
            };
            let has_args = self.kind_at(end) == Some(SyntaxKind::LParen);
            self.call_head(named, has_args);
        } else {
            self.module_error("expected `#` before a call header");
        }
    }

    fn annotation_fragment(&mut self) {
        self.state.builder.start_node(SyntaxKind::Annotation.into());
        if self.cur() == Some(SyntaxKind::At) {
            self.annotation_contents();
        } else {
            self.module_error("expected `@` before an annotation");
        }
    }

    fn fragment_tail(&mut self) {
        self.eat_trivia();
        if self.cur().is_some() {
            self.module_error("unexpected trailing syntax");
            self.state.builder.start_node(SyntaxKind::Error.into());
            while self.cur().is_some() {
                self.eat();
            }
            self.state.builder.finish_node();
        }
    }
}

/// Parse the implemented declaration grammar; no content dialect is required.
pub fn parse_module(source: &str, content: &dyn ContentParser) -> Parse {
    Parser::new(source, true, Some(content)).run_module()
}

/// Parse a complete call header, without a content dialect or trailing body.
pub fn parse_call_header(source: &str) -> Parse {
    let mut parser = Parser::new(source, false, None);
    parser.call_fragment();
    parser.fragment_tail();
    parser.state.builder.finish_node();
    parser.state.finish()
}

/// Parse a call header prefix. Following content and text remain unconsumed;
/// bracketed literals inside arguments use the supplied dialect.
pub fn parse_call_prefix(source: &str, content: &dyn ContentParser) -> Parse {
    let mut parser = Parser::new(source, false, Some(content));
    parser.call_fragment();
    parser.state.builder.finish_node();
    parser.state.finish()
}

pub fn parse_annotation(source: &str) -> Parse {
    let mut parser = Parser::new(source, false, None);
    parser.annotation_fragment();
    parser.fragment_tail();
    parser.state.builder.finish_node();
    parser.state.finish()
}

pub fn parse_annotation_prefix(source: &str, content: &dyn ContentParser) -> Parse {
    let mut parser = Parser::new(source, false, Some(content));
    parser.annotation_fragment();
    parser.state.builder.finish_node();
    parser.state.finish()
}

/// Parse a Code content literal with a caller-selected content dialect.
pub fn parse_content_literal(source: &str, content: &dyn ContentParser) -> Parse {
    let mut parser = Parser::new(source, false, Some(content));
    if parser.cur() == Some(SyntaxKind::LBracket) {
        parser.content_literal();
    } else {
        parser.state.builder.start_node(SyntaxKind::Error.into());
        parser.module_error("expected a content literal");
        while parser.cur().is_some() {
            parser.eat();
        }
        parser.state.builder.finish_node();
    }
    // This is a prefix entry: its enclosing Code parser owns the continuation.
    parser.state.finish()
}

/// Recognize a Markup import keyword without treating longer identifiers as directives.
pub fn is_markup_import(source: &str) -> bool {
    source.strip_prefix("#import").is_some_and(|tail| {
        tail.chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '-')
    })
}

/// Parse a single import prefix, optionally including the Markup `#` marker.
/// The caller supplies one physical line and owns its trailing text.
pub fn parse_import_prefix(source: &str) -> Parse {
    import_fragment(source, false)
}

/// Parse a complete import fragment, retaining invalid trailing syntax.
pub fn parse_import(source: &str) -> Parse {
    import_fragment(source, true)
}

fn import_fragment(source: &str, complete: bool) -> Parse {
    let mut parser = Parser::new(source, true, None);
    parser
        .state
        .builder
        .start_node(SyntaxKind::ImportDecl.into());
    if parser.cur() == Some(SyntaxKind::Hash) {
        parser.eat();
    }
    parser.import_contents();
    if complete {
        parser.fragment_tail();
    }
    parser.state.builder.finish_node();
    parser.state.finish()
}
