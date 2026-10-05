use rowan::{GreenNode, GreenNodeBuilder, TextRange};

use crate::lexer::{Lexed, lex, lex_module};
use crate::syntax::{SyntaxKind, SyntaxNode};

mod module;
mod table;

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

/// The closing convention of the enclosing inline construct. `Flanked` is
/// for emphasis delimiters (whitespace flanking); `Single`/`Pair` close on a
/// literal token, no flanking involved.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Active {
    Flanked(SyntaxKind),
    Single(SyntaxKind),
    Pair(SyntaxKind),
}

/// Outcome of a balanced-group scan: the closer's index, or why the scan
/// failed — the newline ending the enclosing block, or EOF.
enum Balanced {
    Closed(usize),
    BlockEnd,
    Unclosed(usize),
}

/// Parse a Markup document.
pub fn parse_document(src: &str) -> Parse {
    Parser::new(src).run()
}

/// Parse one Code module's source. This entry performs no file or dependency IO.
pub fn parse_module(src: &str) -> Parse {
    Parser::with_lexed(src, lex_module(src), true).run_module()
}

/// Recursive-descent over the lexed token stream, emitting a lossless green
/// tree. `pos` only moves forward: every decision is made by bounded
/// peeking, never by reparsing.
struct Parser<'a> {
    lexed: Lexed<'a>,
    source: &'a str,
    block_indent: usize,
    block_start: Option<usize>,
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    diagnostics: Vec<Diagnostic>,
    /// Inside a `[...]` body: `]` terminates any inline run (see
    /// `inline_delimited`). Set by `bracket_body`, restored on exit.
    in_body: bool,
    /// Table cells and list items bound lookahead and consumption, so paired
    /// inline constructs cannot consume another cell, row, or item.
    limit: Option<usize>,
    /// Code defaults reuse the literal parser, with declaration boundaries
    /// as additional recovery stops and no Markup content literals.
    in_module: bool,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self::with_lexed(src, lex(src), false)
    }

    fn with_lexed(src: &'a str, lexed: Lexed<'a>, in_module: bool) -> Self {
        Self {
            lexed,
            source: src,
            block_indent: 0,
            block_start: None,
            pos: 0,
            builder: GreenNodeBuilder::new(),
            diagnostics: Vec::new(),
            in_body: false,
            limit: None,
            in_module,
        }
    }

    /// Whether an `@` at a block start begins a standalone annotation block:
    /// false when the payload is immediately followed by an inline element
    /// (then it is an inline annotation inside a paragraph).
    fn at_block_annotation(&self) -> bool {
        let mut i = self.pos + 1;
        if self.kind_at(i) == Some(SyntaxKind::Bang) {
            i += 1;
        }
        if self.kind_at(i) != Some(SyntaxKind::LParen) {
            return true; // malformed payload: let the block path diagnose it
        }
        match self.balanced(i, SyntaxKind::LParen, SyntaxKind::RParen, false) {
            Balanced::Closed(end) => matches!(
                self.kind_at(end + 1),
                None | Some(SyntaxKind::Whitespace | SyntaxKind::Newline)
            ),
            _ => true,
        }
    }

    /// Entry point: wraps the whole file in a `Document`. Only called at
    /// block boundaries; every arm there is at a line start.
    ///
    /// Block-level trivia (blank lines, standalone comment lines) belongs to
    /// `Document` directly, so a block's span is exactly its content.
    fn run(mut self) -> Parse {
        self.builder.start_node(SyntaxKind::Document.into());
        while let Some(kind) = self.cur() {
            match kind {
                SyntaxKind::Newline => self.eat(),
                SyntaxKind::Eq if self.at_heading_marker(0) => self.heading(),
                SyntaxKind::Backtick if self.at_fence(0) => self.raw_block(),
                SyntaxKind::Minus if self.at_divider_at(self.pos) => self.divider(),
                SyntaxKind::Minus | SyntaxKind::Plus if self.at_list_marker() => self.list_at(0),
                _ if self.at_table_at(self.pos) => self.table(),
                SyntaxKind::Whitespace if self.at_blank_line() => self.eat_blank_line(),
                SyntaxKind::LineComment | SyntaxKind::BlockComment => self.eat(),
                SyntaxKind::At if self.at_block_annotation() => self.annotation(),
                SyntaxKind::Backslash if self.peek(1) == Some(SyntaxKind::Newline) => {
                    self.parbreak()
                }
                _ => self.inline(Self::line_ends_block),
            }
        }
        self.builder.finish_node();
        Parse {
            green: self.builder.finish(),
            diagnostics: self.diagnostics,
        }
    }

    /// Entry: at `=` followed by whitespace, at a line start. The marker and
    /// its following whitespace are eaten before the inline content.
    fn heading(&mut self) {
        self.builder.start_node(SyntaxKind::Heading.into());
        self.eat();
        self.eat();
        self.inline(|_: &Self| true);
        self.builder.finish_node();
    }

    /// `\`+newline at block level: an explicit paragraph break, equivalent
    /// to a blank line.
    fn parbreak(&mut self) {
        self.builder.start_node(SyntaxKind::ParBreak.into());
        self.eat();
        self.eat();
        self.builder.finish_node();
    }

    /// Exactly `---` at a block start, with only optional trailing whitespace.
    /// The marker's span excludes trailing whitespace and the newline.
    fn divider(&mut self) {
        self.builder.start_node(SyntaxKind::Divider.into());
        for _ in 0..3 {
            self.eat();
        }
        self.builder.finish_node();
    }

    /// Entry: at a fence (`>= 3` backticks) at a line start. Consumes through
    /// the closing fence (equal or longer run) or container end; the closing
    /// fence's trailing newline belongs to the enclosing block sequence.
    ///
    /// An unclosed block swallows to container end and is reported; the `Error` wrapper
    /// is attached retroactively via the checkpoint taken at entry, so a
    /// broken block is structurally marked, not just diagnosed.
    fn raw_block(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.builder.start_node(SyntaxKind::Raw.into());
        let fence_len = self.lexed.len(self.pos);
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
            if self.cur() == Some(SyntaxKind::Whitespace)
                && self.line_content_at(self.pos).1 == self.block_indent
                && self.peek(1) == Some(SyntaxKind::Backtick)
                && self.lexed.len(self.pos + 1) >= fence_len
            {
                self.eat();
            }
            match self.cur() {
                None => break,
                Some(SyntaxKind::Backtick) if self.lexed.len(self.pos) >= fence_len => {
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
                span: TextRange::new(self.lexed.offset(fence_start), self.lexed.offset(self.pos)),
                message: "unclosed raw block".to_string(),
            });
            self.builder
                .start_node_at(checkpoint, SyntaxKind::Error.into());
            self.builder.finish_node();
        }
    }

    fn inline(&mut self, stop: impl Fn(&Self) -> bool) {
        self.inline_delimited(&stop, None);
    }

    /// The one inline loop; all inline constructs are dispatched here.
    ///
    /// `stop` is called only when the current token is a newline and decides
    /// whether the inline run ends there. `active` is the delimiter of the
    /// enclosing construct (e.g. `*` while inside a strong): the loop breaks
    /// before it if it may legally close, letting the caller consume it.
    fn inline_delimited(&mut self, stop: &impl Fn(&Self) -> bool, active: Option<Active>) {
        loop {
            match self.cur() {
                None => break,
                Some(SyntaxKind::Newline) => {
                    if stop(self) {
                        break;
                    }
                    self.eat();
                }
                Some(SyntaxKind::RBracket) if self.in_body => break,
                Some(kind) if self.active_closes(active, kind) => break,
                Some(SyntaxKind::Backslash) if self.peek(1) == Some(SyntaxKind::Newline) => break,
                Some(SyntaxKind::Backtick) => self.raw_inline(),
                Some(SyntaxKind::Star) => {
                    self.delimited(SyntaxKind::Star, SyntaxKind::Strong, stop)
                }
                Some(SyntaxKind::Underscore) => {
                    self.delimited(SyntaxKind::Underscore, SyntaxKind::Emph, stop)
                }
                Some(SyntaxKind::Tilde) => {
                    self.delimited(SyntaxKind::Tilde, SyntaxKind::Strike, stop)
                }
                Some(SyntaxKind::Dollar) => self.math_inline(),
                Some(SyntaxKind::Bang) if self.peek(1) == Some(SyntaxKind::LBracket) => {
                    self.md_link(stop, true)
                }
                Some(SyntaxKind::LBracket) => self.link(stop),
                Some(SyntaxKind::Hash) => self.code_call(),
                // `@(..)` inline annotates the immediately following element;
                // bare `@` stays prose
                Some(SyntaxKind::At)
                    if self.peek(1) == Some(SyntaxKind::LParen)
                        || (self.peek(1) == Some(SyntaxKind::Bang)
                            && self.peek(2) == Some(SyntaxKind::LParen)) =>
                {
                    self.annotation()
                }
                Some(_) => self.eat(),
            }
        }
    }

    fn active_closes(&self, active: Option<Active>, kind: SyntaxKind) -> bool {
        let Some(active) = active else {
            return false;
        };
        match active {
            Active::Flanked(delim) => kind == delim && self.can_close_at(self.pos),
            Active::Single(close) => kind == close,
            Active::Pair(close) => kind == close && self.peek(1) == Some(close),
        }
    }

    /// A paired-delimiter construct (`*..*`, `_.._`, `~..~`). Entry: at the delimiter.
    ///
    /// The construct only comes into being if the opener can open and a legal
    /// closer exists with non-empty content; otherwise the delimiter is plain
    /// text and no node is built (failed pairing never starts).
    fn delimited(&mut self, delim: SyntaxKind, node: SyntaxKind, stop: &impl Fn(&Self) -> bool) {
        if !self.can_open_at(self.pos) {
            self.eat();
            return;
        }
        match self.find_close(self.pos + 1, delim) {
            Some(close) if close > self.pos + 1 => {
                self.builder.start_node(node.into());
                self.eat();
                self.inline_delimited(stop, Some(Active::Flanked(delim)));
                // the walk may have stopped at a body's `]` instead of the closer
                if self.cur() == Some(delim) {
                    self.eat();
                }
                self.builder.finish_node();
            }
            _ => self.eat(),
        }
    }

    /// Depth-counting scan for the closer matching an opener. A delimiter
    /// that can only open increments, one that can close decrements (and wins
    /// ties). Bounded by the enclosing block: a newline that ends the block
    /// also ends the search.
    fn find_close(&self, mut i: usize, delim: SyntaxKind) -> Option<usize> {
        let mut depth = 0usize;
        loop {
            let kind = self.kind_at(i)?;
            if kind == SyntaxKind::Newline && self.line_ends_block_at(i) {
                return None;
            }
            if kind == SyntaxKind::Backslash && self.kind_at(i + 1) == Some(SyntaxKind::Newline) {
                return None;
            }
            if kind == delim {
                if self.can_close_at(i) {
                    if depth == 0 {
                        return Some(i);
                    }
                    depth -= 1;
                } else if self.can_open_at(i) {
                    depth += 1;
                }
            }
            i += 1;
        }
    }

    /// Whitespace flanking, open side: the next token must exist and not be
    /// whitespace.
    fn can_open_at(&self, i: usize) -> bool {
        let Some(next) = self.kind_at(i + 1) else {
            return false;
        };
        !matches!(next, SyntaxKind::Whitespace | SyntaxKind::Newline)
    }

    /// Whitespace flanking, close side: the previous token must exist and not
    /// be whitespace.
    fn can_close_at(&self, i: usize) -> bool {
        if i == 0 {
            return false;
        }
        let Some(prev) = self.kind_at(i - 1) else {
            return false;
        };
        !matches!(prev, SyntaxKind::Whitespace | SyntaxKind::Newline)
    }

    /// Inline math: `$..$`, same flanking rules as paired delimiters, but the
    /// content is an opaque payload (the math grammar is not parsed). Empty
    /// or unclosed means the dollars are plain text.
    fn math_inline(&mut self) {
        if !self.can_open_at(self.pos) {
            self.eat();
            return;
        }
        let mut i = self.pos + 1;
        let close = loop {
            match self.kind_at(i) {
                None => break None,
                Some(SyntaxKind::Newline) if self.line_ends_block_at(i) => break None,
                Some(SyntaxKind::Dollar) if self.can_close_at(i) => break Some(i),
                Some(_) => i += 1,
            }
        };
        match close {
            Some(close) if close > self.pos + 1 => {
                self.builder.start_node(SyntaxKind::Math.into());
                while self.pos <= close {
                    self.eat();
                }
                self.builder.finish_node();
            }
            _ => self.eat(),
        }
    }

    /// Inline raw: a backtick run closed by an equal-length run on the same
    /// line. Longer/shorter runs inside are content. Unclosed means it never
    /// was raw: the opening run stays literal text, no diagnostic.
    fn raw_inline(&mut self) {
        let len = self.lexed.len(self.pos);
        let mut i = self.pos + 1;
        let closed = loop {
            match self.kind_at(i) {
                None => break false,
                Some(SyntaxKind::Newline) => break false,
                Some(SyntaxKind::Backtick) if self.lexed.len(i) == len => break true,
                Some(_) => i += 1,
            }
        };
        if !closed {
            self.eat();
            return;
        }
        self.builder.start_node(SyntaxKind::RawInline.into());
        self.eat();
        loop {
            match self.cur() {
                None => break,
                Some(SyntaxKind::Backtick) if self.lexed.len(self.pos) == len => {
                    self.eat();
                    break;
                }
                Some(_) => self.eat(),
            }
        }
        self.builder.finish_node();
    }

    /// Links, both spellings: `[[target]]` / `[[target|text]]` and
    /// `[text](target)`. Entry: at `[`. Any shape mismatch (no `]]`, no `]`,
    /// no `(...)`, or a block boundary in between) leaves the bracket as
    /// literal text.
    fn link(&mut self, stop: &impl Fn(&Self) -> bool) {
        if self.peek(1) == Some(SyntaxKind::LBracket) {
            self.wikilink(stop);
        } else {
            self.md_link(stop, false);
        }
    }

    fn wikilink(&mut self, stop: &impl Fn(&Self) -> bool) {
        let mut i = self.pos + 2;
        let mut pipe = None;
        let close = loop {
            match self.kind_at(i) {
                None => break None,
                Some(SyntaxKind::Newline) => break None,
                Some(SyntaxKind::Pipe) if pipe.is_none() => {
                    pipe = Some(i);
                    i += 1;
                }
                Some(SyntaxKind::RBracket) if self.kind_at(i + 1) == Some(SyntaxKind::RBracket) => {
                    break Some(i);
                }
                Some(_) => i += 1,
            }
        };
        let Some(close) = close else {
            self.eat();
            return;
        };
        self.builder.start_node(SyntaxKind::WikiLink.into());
        self.eat();
        self.eat();
        let target_end = pipe.unwrap_or(close);
        while self.pos < target_end {
            self.eat();
        }
        if pipe.is_some() {
            self.eat();
            self.inline_delimited(stop, Some(Active::Pair(SyntaxKind::RBracket)));
        }
        self.eat();
        self.eat();
        self.builder.finish_node();
    }

    fn md_link(&mut self, stop: &impl Fn(&Self) -> bool, embed: bool) {
        let open = self.pos + usize::from(embed);
        let mut i = open + 1;
        let mut depth = 1usize;
        let close = loop {
            match self.kind_at(i) {
                None => break None,
                Some(SyntaxKind::Newline) if self.line_ends_block_at(i) => break None,
                Some(SyntaxKind::Backslash) if self.kind_at(i + 1) == Some(SyntaxKind::Newline) => {
                    break None;
                }
                Some(kind @ (SyntaxKind::Backtick | SyntaxKind::Dollar)) => {
                    // Brackets inside opaque inline payloads are not label delimiters.
                    let mut end = i + 1;
                    let mut found = None;
                    if kind == SyntaxKind::Backtick || self.can_open_at(i) {
                        while let Some(next) = self.kind_at(end) {
                            if next == SyntaxKind::Newline
                                && (kind == SyntaxKind::Backtick || self.line_ends_block_at(end))
                            {
                                break;
                            }
                            if next == kind
                                && ((kind == SyntaxKind::Backtick
                                    && self.lexed.len(end) == self.lexed.len(i))
                                    || (kind == SyntaxKind::Dollar && self.can_close_at(end)))
                            {
                                found = Some(end);
                                break;
                            }
                            end += 1;
                        }
                    }
                    i = found.map_or(i + 1, |end| end + 1);
                }
                Some(SyntaxKind::LBracket) => {
                    depth += 1;
                    i += 1;
                }
                Some(SyntaxKind::RBracket) => {
                    depth -= 1;
                    if depth == 0 {
                        break Some(i);
                    }
                    i += 1;
                }
                Some(_) => i += 1,
            }
        };
        let Some(close) = close else {
            self.eat();
            return;
        };
        if self.kind_at(close + 1) != Some(SyntaxKind::LParen) {
            self.eat();
            return;
        }
        let mut j = close + 2;
        let mut parens = 1usize;
        let mut quote = None;
        let mut escaped = false;
        let mut previous_space = false;
        let close_paren = loop {
            match self.kind_at(j) {
                None => break None,
                Some(SyntaxKind::Newline) => break None,
                Some(_) => {
                    for ch in self.lexed.text(j).chars() {
                        if escaped {
                            escaped = false;
                            previous_space = false;
                            continue;
                        }
                        if ch == '\\' {
                            escaped = true;
                            previous_space = false;
                            continue;
                        }
                        if let Some(q) = quote {
                            if ch == q {
                                quote = None;
                            }
                        } else {
                            match ch {
                                '"' | '\'' if previous_space => quote = Some(ch),
                                '(' => parens += 1,
                                ')' => parens -= 1,
                                _ => {}
                            }
                        }
                        previous_space = ch.is_whitespace();
                    }
                    if parens == 0 {
                        break Some(j);
                    }
                    j += 1;
                }
            }
        };
        let Some(close_paren) = close_paren else {
            self.eat();
            return;
        };
        self.builder.start_node(
            if embed {
                SyntaxKind::Embed
            } else {
                SyntaxKind::Link
            }
            .into(),
        );
        if embed {
            self.eat();
        }
        self.eat();
        self.inline_delimited(stop, Some(Active::Single(SyntaxKind::RBracket)));
        self.eat();
        self.eat();
        while self.pos < close_paren {
            self.eat();
        }
        self.eat();
        self.builder.finish_node();
    }

    /// `#name(args)[body]` — an atomic constructor call; `#[body]` is the
    /// anonymous form, producing a transparent group node. Anything else
    /// after `#` (bare `#name`, `#(..)`) is literal text.
    fn code_call(&mut self) {
        let named = self.peek(1) == Some(SyntaxKind::Ident);
        let args_at = if named {
            let Some(end) = self.path_end(self.pos + 1) else {
                self.eat();
                return;
            };
            end
        } else {
            self.pos + 1
        };
        let has_args = self.kind_at(args_at) == Some(SyntaxKind::LParen);
        let mut after = args_at;
        if has_args {
            match self.balanced(args_at, SyntaxKind::LParen, SyntaxKind::RParen, false) {
                Balanced::Closed(end) => after = end + 1,
                _ => {
                    self.eat();
                    return;
                }
            }
        }
        let mut body_close = None;
        let has_body = if self.kind_at(after) == Some(SyntaxKind::LBracket) {
            match self.balanced(after, SyntaxKind::LBracket, SyntaxKind::RBracket, true) {
                Balanced::Closed(close) => {
                    body_close = Some(close);
                    true
                }
                Balanced::Unclosed(eof) => {
                    let point = self.lexed.offset(eof);
                    self.diagnostics.push(Diagnostic {
                        span: TextRange::new(point, point),
                        message: "unclosed code call body".to_string(),
                    });
                    false
                }
                Balanced::BlockEnd => unreachable!("body scans cross blocks"),
            }
        } else {
            false
        };
        let is_call = (named && (has_args || has_body)) || (!named && !has_args && has_body);
        if !is_call {
            self.eat();
            return;
        }
        self.builder.start_node(SyntaxKind::CodeCall.into());
        self.eat();
        if named {
            self.path();
        }
        if has_args {
            self.eat();
            self.call_args();
            self.eat();
        }
        if has_body {
            let close = body_close.unwrap();
            let block = self.is_block_body(after, close);
            if !block {
                self.diagnose_inline_parbreak(after, close);
            }
            self.bracket_body(block);
        }
        self.builder.finish_node();
    }

    /// Paths are adjacent identifier segments. A trailing separator invalidates
    /// the whole path rather than silently truncating the target name.
    fn path_end(&self, from: usize) -> Option<usize> {
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

    fn path(&mut self) {
        let end = self.path_end(self.pos).unwrap();
        self.builder.start_node(SyntaxKind::Path.into());
        while self.pos < end {
            self.eat();
        }
        self.builder.finish_node();
    }

    /// Whether the bracketed content spanning `open..=close` is block-level:
    /// both brackets have whitespace immediately inside (`[ x ]` vs `[x]`).
    fn is_block_body(&self, open: usize, close: usize) -> bool {
        let flank = |i: usize| {
            matches!(
                self.kind_at(i),
                Some(SyntaxKind::Whitespace | SyntaxKind::Newline)
            )
        };
        flank(open + 1) && flank(close - 1)
    }

    /// An inline-flanked body must not contain a paragraph break; diagnose
    /// the first one (the parser recovers by absorbing it as a soft break).
    fn diagnose_inline_parbreak(&mut self, open: usize, close: usize) {
        for i in open + 1..close {
            if self.kind_at(i) == Some(SyntaxKind::Newline) && self.line_ends_block_at(i) {
                let point = self.lexed.offset(i);
                self.diagnostics.push(Diagnostic {
                    span: TextRange::new(point, point),
                    message: "inline content cannot contain a blank line".to_string(),
                });
                return;
            }
        }
    }

    /// Parse the bracketed content at cur == `[` with known flavor. `]`
    /// terminates any inline run inside via `in_body`.
    fn bracket_body(&mut self, block: bool) {
        self.eat();
        let was = std::mem::replace(&mut self.in_body, true);
        if block {
            self.block_body();
        } else {
            self.inline_delimited(
                &|_: &Self| false,
                Some(Active::Single(SyntaxKind::RBracket)),
            );
        }
        self.in_body = was;
        self.eat();
    }

    /// A block-level `[...]` body: full block grammar up to the closing `]`.
    /// Inter-block trivia (blank lines, comment lines) belongs to the body.
    fn block_body(&mut self) {
        loop {
            match self.cur() {
                None | Some(SyntaxKind::RBracket) => break,
                Some(SyntaxKind::Newline) => self.eat(),
                Some(SyntaxKind::Eq) if self.at_heading_marker(0) => self.heading(),
                Some(SyntaxKind::Backtick) if self.at_fence(0) => self.raw_block(),
                Some(SyntaxKind::Minus) if self.at_divider_at(self.pos) => self.divider(),
                Some(SyntaxKind::Minus) | Some(SyntaxKind::Plus) if self.at_list_marker() => {
                    self.list_at(self.column_at(self.pos))
                }
                _ if self.at_table_at(self.pos) => self.table(),
                Some(SyntaxKind::Whitespace) if self.at_blank_line() => self.eat_blank_line(),
                Some(SyntaxKind::Whitespace) if self.block_indent > 0 => self.eat(),
                Some(SyntaxKind::LineComment) | Some(SyntaxKind::BlockComment) => self.eat(),
                Some(SyntaxKind::At) if self.at_block_annotation() => self.annotation(),
                Some(SyntaxKind::Backslash) if self.peek(1) == Some(SyntaxKind::Newline) => {
                    self.parbreak()
                }
                _ => self.inline(Self::line_ends_block),
            }
        }
    }

    /// A `[...]` content literal in code: parsed like a call body.
    fn content_literal(&mut self) {
        let open = self.pos;
        match self.balanced(open, SyntaxKind::LBracket, SyntaxKind::RBracket, true) {
            Balanced::Closed(close) => {
                let block = self.is_block_body(open, close);
                if !block {
                    self.diagnose_inline_parbreak(open, close);
                }
                self.bracket_body(block);
            }
            Balanced::Unclosed(eof) => {
                let point = self.lexed.offset(eof);
                self.diagnostics.push(Diagnostic {
                    span: TextRange::new(point, point),
                    message: "unclosed content literal".to_string(),
                });
                self.eat();
            }
            Balanced::BlockEnd => unreachable!(),
        }
    }

    /// Scan for the closer of the group opened at `from`. When
    /// `cross_blocks` is set (for `[...]` bodies, which may span blank
    /// lines), only EOF bounds the scan; otherwise the enclosing block does.
    fn balanced(
        &self,
        mut i: usize,
        open: SyntaxKind,
        close: SyntaxKind,
        cross_blocks: bool,
    ) -> Balanced {
        let mut depth = 0usize;
        loop {
            let Some(kind) = self.kind_at(i) else {
                return Balanced::Unclosed(i);
            };
            if !cross_blocks && kind == SyntaxKind::Newline && self.line_ends_block_at(i) {
                return Balanced::BlockEnd;
            }
            if kind == open {
                depth += 1;
            }
            if kind == close {
                depth -= 1;
                if depth == 0 {
                    return Balanced::Closed(i);
                }
            }
            i += 1;
        }
    }

    /// `@(dict)` annotates the immediately following block; `@!(dict)` at the
    /// top of the file annotates the module. The payload must be a complete
    /// dict literal (empty: `@(:)`); anything else is diagnosed.
    fn annotation(&mut self) {
        self.builder.start_node(SyntaxKind::Annotation.into());
        self.eat();
        if self.cur() == Some(SyntaxKind::Bang) {
            self.eat();
        }
        if self.cur() == Some(SyntaxKind::LParen) {
            let start = self.lexed.offset(self.pos);
            if self.literal_group() != Some(SyntaxKind::Dict) {
                let span = TextRange::new(start, self.lexed.offset(self.pos));
                self.diagnostics.push(Diagnostic {
                    span,
                    message: "annotation payload must be a dict literal".to_string(),
                });
            }
        } else {
            let point = self.lexed.offset(self.pos);
            self.diagnostics.push(Diagnostic {
                span: TextRange::new(point, point),
                message: "expected a dict literal after `@`".to_string(),
            });
        }
        self.builder.finish_node();
    }

    /// A comma-separated run of literal entries (`key: value` or bare
    /// values), shared by annotation payloads and call arguments.
    fn call_args(&mut self) {
        loop {
            self.eat_trivia();
            match self.cur() {
                None | Some(SyntaxKind::RParen) => break,
                Some(SyntaxKind::Comma) => self.eat(),
                _ => {
                    self.literal_entry_or_value();
                }
            }
        }
    }

    /// `key: value` gets an `Entry` node; a bare literal stands alone.
    /// Returns whether an entry was produced.
    fn literal_entry_or_value(&mut self) -> bool {
        let is_key = matches!(self.cur(), Some(SyntaxKind::Ident) | Some(SyntaxKind::Str))
            && self.peek(1) == Some(SyntaxKind::Colon);
        if is_key {
            self.builder.start_node(SyntaxKind::Entry.into());
            self.eat();
            self.eat();
            self.eat_trivia();
            self.literal_value();
            self.builder.finish_node();
            true
        } else {
            self.literal_value();
            false
        }
    }

    fn eat_trivia(&mut self) {
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

    fn literal_value(&mut self) {
        match self.cur() {
            Some(SyntaxKind::Str) | Some(SyntaxKind::Number) | Some(SyntaxKind::Ident) => {
                self.eat()
            }
            Some(SyntaxKind::Minus) if self.peek(1) == Some(SyntaxKind::Number) => {
                self.builder.start_node(SyntaxKind::Neg.into());
                self.eat();
                self.eat();
                self.builder.finish_node();
            }
            Some(SyntaxKind::LParen) => {
                self.literal_group();
            }
            Some(SyntaxKind::LBracket) if !self.in_module => self.content_literal(),
            _ => {
                let point = self.lexed.offset(self.pos);
                self.diagnostics.push(Diagnostic {
                    span: TextRange::new(point, point),
                    message: "expected a literal".to_string(),
                });
                if self.cur().is_some()
                    && !(self.in_module
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
    fn literal_group(&mut self) -> Option<SyntaxKind> {
        if self.peek(1) == Some(SyntaxKind::RParen) {
            self.builder.start_node(SyntaxKind::Unit.into());
            self.eat();
            self.eat();
            self.builder.finish_node();
            return Some(SyntaxKind::Unit);
        }
        if self.peek(1) == Some(SyntaxKind::Colon) && self.peek(2) == Some(SyntaxKind::RParen) {
            self.builder.start_node(SyntaxKind::Dict.into());
            self.eat();
            self.eat();
            self.eat();
            self.builder.finish_node();
            return Some(SyntaxKind::Dict);
        }
        if self.peek(1) == Some(SyntaxKind::Comma) && self.peek(2) == Some(SyntaxKind::RParen) {
            self.builder.start_node(SyntaxKind::Array.into());
            self.eat();
            self.eat();
            self.eat();
            self.builder.finish_node();
            return Some(SyntaxKind::Array);
        }
        let checkpoint = self.builder.checkpoint();
        self.eat();
        let mut has_named = false;
        let mut values = 0usize;
        let mut had_comma = false;
        loop {
            self.eat_trivia();
            match self.cur() {
                None | Some(SyntaxKind::Semicolon | SyntaxKind::FnKeyword)
                    if self.in_module || self.cur().is_none() =>
                {
                    let point = self.lexed.offset(self.pos);
                    self.diagnostics.push(Diagnostic {
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
        self.builder.start_node_at(checkpoint, kind.into());
        self.builder.finish_node();
        Some(kind)
    }

    /// A structural sequence of same-type items. Blank lines between items
    /// belong to the list; an item's indented block sequence owns its body.
    fn list_at(&mut self, indent: usize) {
        let marker = self.cur();
        self.builder.start_node(SyntaxKind::List.into());
        loop {
            self.list_item(indent);
            if self.cur() != Some(SyntaxKind::Newline) {
                break;
            }
            let mut next = self.pos + 1;
            loop {
                let (content, _) = self.line_content_at(next);
                if self.kind_at(content) == Some(SyntaxKind::Newline) {
                    next = content + 1;
                } else {
                    next = content;
                    break;
                }
            }
            if self.column_at(next) != indent
                || self.kind_at(next) != marker
                || self.kind_at(next + 1) != Some(SyntaxKind::Whitespace)
            {
                break;
            }
            while self.pos < next {
                self.eat();
            }
        }
        self.builder.finish_node();
    }

    fn list_item(&mut self, indent: usize) {
        self.builder.start_node(SyntaxKind::ListItem.into());
        self.eat(); // marker
        let body_indent = self
            .lexed
            .text(self.pos)
            .chars()
            .fold(indent + 1, |column, ch| {
                if ch == '\t' {
                    (column / 4 + 1) * 4
                } else {
                    column + 1
                }
            });
        self.eat(); // marker whitespace
        let start = self.pos;
        let mut end = start;
        while let Some(kind) = self.kind_at(end) {
            if kind == SyntaxKind::Newline {
                let boundary = end;
                let mut next = end + 1;
                loop {
                    let (content, width) = self.line_content_at(next);
                    if self.kind_at(content) == Some(SyntaxKind::Newline) {
                        next = content + 1;
                        continue;
                    }
                    if self.kind_at(content).is_none() || width < body_indent {
                        end = boundary;
                    } else {
                        end = next;
                    }
                    break;
                }
                if end == boundary {
                    break;
                }
            } else {
                end += 1;
            }
        }
        let old_limit = self.limit.replace(end);
        let old_indent = std::mem::replace(&mut self.block_indent, body_indent);
        let old_start = self.block_start.replace(start);
        while let Some(kind) = self.cur() {
            match kind {
                SyntaxKind::RBracket if self.in_body => break,
                SyntaxKind::Newline
                | SyntaxKind::Whitespace
                | SyntaxKind::LineComment
                | SyntaxKind::BlockComment => self.eat(),
                SyntaxKind::Eq if self.at_heading_marker(0) => self.heading(),
                SyntaxKind::Backtick if self.at_fence(0) => self.raw_block(),
                SyntaxKind::Minus if self.at_divider_at(self.pos) => self.divider(),
                SyntaxKind::Minus | SyntaxKind::Plus if self.at_list_marker() => {
                    self.list_at(self.column_at(self.pos))
                }
                _ if self.at_table_at(self.pos) => self.table(),
                SyntaxKind::At if self.at_block_annotation() => self.annotation(),
                SyntaxKind::Backslash if self.peek(1) == Some(SyntaxKind::Newline) => {
                    self.parbreak()
                }
                _ => self.inline(Self::line_ends_block),
            }
        }
        self.block_start = old_start;
        self.block_indent = old_indent;
        self.limit = old_limit;
        self.builder.finish_node();
    }

    fn line_content_at(&self, mut pos: usize) -> (usize, usize) {
        let mut width = 0;
        while self.kind_at(pos) == Some(SyntaxKind::Whitespace) {
            width = self.lexed.text(pos).chars().fold(width, |column, ch| {
                if ch == '\t' {
                    (column / 4 + 1) * 4
                } else {
                    column + 1
                }
            });
            pos += 1;
        }
        (pos, width)
    }

    fn column_at(&self, pos: usize) -> usize {
        let offset = usize::from(self.lexed.offset(pos));
        let start = self.source[..offset]
            .rfind(['\n', '\r'])
            .map_or(0, |i| i + 1);
        self.source[start..offset].chars().fold(0, |column, ch| {
            if ch == '\t' {
                (column / 4 + 1) * 4
            } else {
                column + 1
            }
        })
    }

    fn at_block_start(&self, pos: usize) -> bool {
        if self.block_start == Some(pos) {
            return true;
        }
        let offset = usize::from(self.lexed.offset(pos));
        let column = self.column_at(pos);
        let start = self.source[..offset]
            .rfind(['\n', '\r'])
            .map_or(0, |i| i + 1);
        let prefix = &self.source[start..offset];
        prefix.bytes().all(|ch| ch == b' ' || ch == b'\t')
            && (column == 0 || self.block_indent > 0 && column >= self.block_indent)
    }

    fn at_list_marker(&self) -> bool {
        matches!(self.cur(), Some(SyntaxKind::Minus) | Some(SyntaxKind::Plus))
            && self.peek(1) == Some(SyntaxKind::Whitespace)
    }

    fn at_divider_at(&self, pos: usize) -> bool {
        if !self.at_block_start(pos) {
            return false;
        }
        if !(0..3).all(|i| self.kind_at(pos + i) == Some(SyntaxKind::Minus)) {
            return false;
        }
        let mut end = pos + 3;
        while self.kind_at(end) == Some(SyntaxKind::Whitespace) {
            end += 1;
        }
        matches!(self.kind_at(end), None | Some(SyntaxKind::Newline))
    }

    fn line_ends_block(&self) -> bool {
        match self.cur() {
            None => true,
            Some(SyntaxKind::Newline) => self.line_ends_block_at(self.pos),
            _ => unreachable!(),
        }
    }

    /// Whether a block ends after the newline at `pos`: a blank line follows
    /// (whitespace-only counts), or the next line starts a new block
    /// (heading marker, fence, list marker, divider, annotation). Document
    /// markers start at column zero; list item blocks use their body indentation.
    fn line_ends_block_at(&self, pos: usize) -> bool {
        let (next, indent) = self.line_content_at(pos + 1);
        if matches!(self.kind_at(next), None | Some(SyntaxKind::Newline)) {
            return true;
        }
        if indent < self.block_indent {
            return true;
        }
        if self.at_table_at(next) {
            return true;
        }
        if !self.at_block_start(next) {
            return false;
        }
        match self.kind_at(next) {
            Some(SyntaxKind::Eq) => self.kind_at(next + 1) == Some(SyntaxKind::Whitespace),
            Some(SyntaxKind::Backtick) => self.lexed.len(next) >= 3,
            Some(SyntaxKind::Minus) | Some(SyntaxKind::Plus) => {
                self.kind_at(next + 1) == Some(SyntaxKind::Whitespace) || self.at_divider_at(next)
            }
            Some(SyntaxKind::At) => true,
            _ => false,
        }
    }

    fn at_heading_marker(&self, ws_skip: usize) -> bool {
        self.peek(ws_skip) == Some(SyntaxKind::Eq)
            && self.peek(ws_skip + 1) == Some(SyntaxKind::Whitespace)
    }

    fn at_fence(&self, ws_skip: usize) -> bool {
        self.peek(ws_skip) == Some(SyntaxKind::Backtick) && self.lexed.len(self.pos + ws_skip) >= 3
    }

    fn at_blank_line(&self) -> bool {
        let mut i = self.pos;
        while self.kind_at(i) == Some(SyntaxKind::Whitespace) {
            i += 1;
        }
        matches!(self.kind_at(i), None | Some(SyntaxKind::Newline))
    }

    fn eat_blank_line(&mut self) {
        while self.cur() == Some(SyntaxKind::Whitespace) {
            self.eat();
        }
    }

    fn kind_at(&self, i: usize) -> Option<SyntaxKind> {
        if self.limit.is_some_and(|limit| i >= limit) {
            None
        } else {
            self.lexed.kind(i)
        }
    }

    fn cur(&self) -> Option<SyntaxKind> {
        self.kind_at(self.pos)
    }

    fn peek(&self, offset: usize) -> Option<SyntaxKind> {
        self.kind_at(self.pos + offset)
    }

    /// The single funnel for consuming a token into the tree; also reports
    /// unterminated block comments (a lexical, trivia-level breakage, hence
    /// no `Error` wrapper).
    fn eat(&mut self) {
        let kind = self.kind_at(self.pos).unwrap();
        let text = self.lexed.text(self.pos);
        let unclosed = if kind == SyntaxKind::BlockComment
            && !crate::lexer::block_comment_closed(text)
        {
            Some("unclosed block comment")
        } else if self.in_module && kind == SyntaxKind::Str && !crate::lexer::string_closed(text) {
            Some("unclosed string")
        } else {
            None
        };
        if let Some(message) = unclosed {
            self.diagnostics.push(Diagnostic {
                span: TextRange::new(self.lexed.offset(self.pos), self.lexed.offset(self.pos + 1)),
                message: message.to_string(),
            });
        }
        self.builder.token(kind.into(), text);
        self.pos += 1;
    }
}
