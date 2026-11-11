//! Code modules and syntax fragments. Concrete content grammars are injected.
use crate::content::ContentParser;
use crate::lexed::Mode;
use crate::parser::State;
use crate::tree::SyntaxKind;
use crate::Parse;
use rowan::TextRange;

mod expressions;
mod literals;
mod module;

struct Parser<'state, 'src, 'content> {
    state: &'state mut State<'src>,
    content: Option<&'content dyn ContentParser>,
    in_comptime_args: usize,
    expression_depth: usize,
    loop_subject_depth: Option<usize>,
}

impl<'state, 'src, 'content> Parser<'state, 'src, 'content> {
    fn new(
        state: &'state mut State<'src>,
        content: Option<&'content dyn ContentParser>,
    ) -> Self {
        Self {
            state,
            content,
            in_comptime_args: 0,
            expression_depth: 0,
            loop_subject_depth: None,
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

/// A standalone Code state over `source`, in declaration or fragment mode.
fn state(source: &str, mode: Mode, in_module: bool) -> State<'_> {
    let mut state = State::new(source, 0, mode);
    state.in_module = in_module;
    state
}

/// Parse the implemented declaration grammar; no content dialect is required.
pub fn parse_module(source: &str, content: &dyn ContentParser) -> Parse {
    let mut state = state(source, Mode::Module, true);
    Parser::new(&mut state, Some(content)).run_module();
    state.finish()
}

/// Parse a complete call header, without a content dialect or trailing body.
pub fn parse_call_header(source: &str) -> Parse {
    let mut state = state(source, Mode::Module, false);
    {
        let mut parser = Parser::new(&mut state, None);
        parser.call_fragment();
        parser.fragment_tail();
        parser.state.builder.finish_node();
    }
    state.finish()
}

/// Parse a call header prefix. Following content and text remain unconsumed;
/// bracketed literals inside arguments use the supplied dialect.
pub fn parse_call_prefix(source: &str, content: &dyn ContentParser) -> Parse {
    let mut state = state(source, Mode::Module, false);
    {
        let mut parser = Parser::new(&mut state, Some(content));
        parser.call_fragment();
        parser.state.builder.finish_node();
    }
    state.finish()
}

pub fn parse_annotation(source: &str) -> Parse {
    let mut state = state(source, Mode::Module, false);
    {
        let mut parser = Parser::new(&mut state, None);
        parser.annotation_fragment();
        parser.fragment_tail();
        parser.state.builder.finish_node();
    }
    state.finish()
}

pub fn parse_annotation_prefix(source: &str, content: &dyn ContentParser) -> Parse {
    let mut state = state(source, Mode::Module, false);
    {
        let mut parser = Parser::new(&mut state, Some(content));
        parser.annotation_fragment();
        parser.state.builder.finish_node();
    }
    state.finish()
}

/// Parse a Code content literal with a caller-selected content dialect.
pub fn parse_content_literal(source: &str, content: &dyn ContentParser) -> Parse {
    let mut state = state(source, Mode::Module, false);
    {
        let mut parser = Parser::new(&mut state, Some(content));
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
    }
    state.finish()
}

/// Recognize a Markup import keyword without treating longer identifiers as directives.
pub fn is_markup_import(source: &str) -> bool {
    source.strip_prefix("#import").is_some_and(|tail| {
        tail.chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '-')
    })
}

pub fn is_markup_for(source: &str) -> bool {
    source.strip_prefix("#for").is_some_and(|tail| {
        tail.chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '-')
    })
}

thread_local! { static FOR_NESTING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
/// Shared syntax/lowering recursion bound across embedded frontend callbacks.
#[doc(hidden)]
pub struct ForNesting {
    _private: (),
}
impl ForNesting {
    pub fn enter() -> Option<Self> {
        if FOR_NESTING.get() >= 16 {
            return None;
        }
        FOR_NESTING.set(FOR_NESTING.get() + 1);
        Some(Self { _private: () })
    }
}
impl Drop for ForNesting {
    fn drop(&mut self) {
        FOR_NESTING.set(FOR_NESTING.get() - 1);
    }
}

/// A complete loop expression prefix; the content dialect owns its `[]` body.
pub fn parse_for_prefix(source: &str, content: &dyn ContentParser) -> Parse {
    let Some(_guard) = ForNesting::enter() else {
        let end = if source.starts_with("#for") {
            4
        } else if source.starts_with("for") {
            3
        } else {
            source.chars().next().map_or(0, char::len_utf8)
        };
        let mut builder = rowan::GreenNodeBuilder::new();
        builder.start_node(SyntaxKind::ForExpr.into());
        builder.token(SyntaxKind::Error.into(), &source[..end]);
        builder.finish_node();
        return Parse::new(
            builder.finish(),
            vec![crate::Diagnostic {
                span: TextRange::new(0.into(), (end as u32).into()),
                message: "for syntax nesting limit exceeded".into(),
            }],
        );
    };
    let mut state = state(source, Mode::Module, false);
    Parser::new(&mut state, Some(content)).for_expr();
    state.finish()
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
    let mut state = state(source, Mode::Module, true);
    {
        let mut parser = Parser::new(&mut state, None);
        parser.state.builder.start_node(SyntaxKind::ImportDecl.into());
        if parser.cur() == Some(SyntaxKind::Hash) {
            parser.eat();
        }
        parser.import_contents();
        if complete {
            parser.fragment_tail();
        }
        parser.state.builder.finish_node();
    }
    state.finish()
}

/// A Code construct parsed in place on a caller's state. The caller places the
/// cursor and, for `CallHeader`, owns the still-open `CodeCall` node.
pub(crate) enum Fragment {
    /// `#path<args>(args)`; leaves the `CodeCall` node open for its body.
    CallHeader,
    /// `@(!)(dict)` without a node, so the caller can choose the node kind.
    AnnotationBody,
    For,
    Import,
}

pub(crate) fn parse_fragment(
    state: &mut State<'_>,
    content: &dyn ContentParser,
    fragment: Fragment,
) {
    let mut parser = Parser::new(state, Some(content));
    match fragment {
        Fragment::CallHeader => parser.call_fragment(),
        Fragment::AnnotationBody => parser.annotation_contents(),
        Fragment::For => parser.for_expr(),
        Fragment::Import => {
            parser.state.builder.start_node(SyntaxKind::ImportDecl.into());
            if parser.cur() == Some(SyntaxKind::Hash) {
                parser.eat();
            }
            parser.import_contents();
            parser.state.builder.finish_node();
        }
    }
}
