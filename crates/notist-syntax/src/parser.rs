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

/// Chars whose special meaning `\` cancels. `\<newline>` is a hard break
/// instead; `\` followed by anything else stays literal.
/// The closing convention of the enclosing inline construct. `Flanked` is
/// for emphasis delimiters (whitespace flanking); `Single`/`Pair` close on a
/// literal token, no flanking involved.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Active {
    Flanked(SyntaxKind),
    Single(SyntaxKind),
    Pair(SyntaxKind),
}

fn is_escapable(kind: SyntaxKind) -> bool {    matches!(
        kind,
        SyntaxKind::Eq
            | SyntaxKind::Star
            | SyntaxKind::Underscore
            | SyntaxKind::Backslash
            | SyntaxKind::LBracket
            | SyntaxKind::RBracket
            | SyntaxKind::LParen
            | SyntaxKind::RParen
            | SyntaxKind::Pipe
            | SyntaxKind::Hash
            | SyntaxKind::Dollar
            | SyntaxKind::Backtick
    )
}

pub fn parse(src: &str) -> Parse {
    Parser::new(src).run()
}

/// Recursive-descent over a pre-lexed token vector, emitting a lossless
/// green tree. `pos` only moves forward: every decision is made by bounded
/// peeking, never by reparsing.
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
                SyntaxKind::Minus | SyntaxKind::Plus if self.at_list_marker() => self.list_at(0),
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

    /// Entry: at `=` followed by whitespace, at a line start. The marker and
    /// its following whitespace stay outside the `Inline` child.
    fn heading(&mut self) {
        self.builder.start_node(SyntaxKind::Heading.into());
        self.eat();
        self.eat();
        self.inline(|_: &Self| true);
        self.builder.finish_node();
    }

    /// Entry: at a fence (`>= 3` backticks) at a line start. Consumes through
    /// the closing fence (equal or longer run) or EOF; the closing fence's
    /// trailing newline belongs to `Document`.
    ///
    /// An unclosed block swallows to EOF and is reported; the `Error` wrapper
    /// is attached retroactively via the checkpoint taken at entry, so a
    /// broken block is structurally marked, not just diagnosed.
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
        self.inline(Self::line_ends_block);
        self.builder.finish_node();
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
        self.builder.start_node(SyntaxKind::Inline.into());
        loop {
            match self.cur() {
                None => break,
                Some(SyntaxKind::Newline) => {
                    if stop(self) {
                        break;
                    }
                    self.eat();
                }
                Some(kind) if self.active_closes(active, kind) => break,
                Some(SyntaxKind::Backslash) => self.escape_or_break(),
                Some(SyntaxKind::Backtick) => self.raw_inline(),
                Some(SyntaxKind::Star) => self.delimited(SyntaxKind::Star, SyntaxKind::Strong, stop),
                Some(SyntaxKind::Underscore) => {
                    self.delimited(SyntaxKind::Underscore, SyntaxKind::Emph, stop)
                }
                Some(SyntaxKind::Dollar) => self.math_inline(),
                Some(SyntaxKind::LBracket) => self.link(stop),
                Some(SyntaxKind::Hash) => self.code_embed(),
                Some(_) => self.eat(),
            }
        }
        self.builder.finish_node();
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

    /// A paired-delimiter construct (`*…*`, `_…_`). Entry: at the delimiter.
    ///
    /// The construct only comes into being if the opener can open and a legal
    /// closer exists with non-empty content; otherwise the delimiter is plain
    /// text and no node is built (failed pairing never starts).
    fn delimited(
        &mut self,
        delim: SyntaxKind,
        node: SyntaxKind,
        stop: &impl Fn(&Self) -> bool,
    ) {
        if !self.can_open_at(self.pos) {
            self.eat();
            return;
        }
        match self.find_close(self.pos + 1, delim) {
            Some(close) if close > self.pos + 1 => {
                self.builder.start_node(node.into());
                self.eat();
                self.inline_delimited(stop, Some(Active::Flanked(delim)));
                self.eat();
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
            let (kind, _) = *self.tokens.get(i)?;
            if kind == SyntaxKind::Newline && self.line_ends_block_at(i) {
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
        let Some((next, _)) = self.tokens.get(i + 1) else {
            return false;
        };
        !matches!(next, SyntaxKind::Whitespace | SyntaxKind::Newline)
    }

    /// Whitespace flanking, close side: the previous token must exist and not
    /// be whitespace.
    fn can_close_at(&self, i: usize) -> bool {
        i > 0 && !matches!(
            self.tokens[i - 1].0,
            SyntaxKind::Whitespace | SyntaxKind::Newline
        )
    }

    /// Inline raw: a backtick run closed by an equal-length run on the same
    /// line. Longer/shorter runs inside are content. Unclosed means it never
    /// was raw: the opening run stays literal text, no diagnostic.
    /// Inline math: `$…$`, same flanking rules as paired delimiters, but the
    /// content is an opaque payload (the math grammar is not parsed). Empty
    /// or unclosed means the dollars are plain text.
    fn math_inline(&mut self) {
        if !self.can_open_at(self.pos) {
            self.eat();
            return;
        }
        let mut i = self.pos + 1;
        let close = loop {
            match self.tokens.get(i) {
                None => break None,
                Some((SyntaxKind::Newline, _)) if self.line_ends_block_at(i) => break None,
                Some((SyntaxKind::Dollar, _)) if self.can_close_at(i) => break Some(i),
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

    fn raw_inline(&mut self) {
        let len = self.tokens[self.pos].1.len();
        let mut i = self.pos + 1;
        let closed = loop {
            match self.tokens.get(i) {
                None => break false,
                Some((SyntaxKind::Newline, _)) => break false,
                Some((SyntaxKind::Backtick, text)) if text.len() == len => break true,
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
                Some(SyntaxKind::Backtick) if self.tokens[self.pos].1.len() == len => {
                    self.eat();
                    break;
                }
                Some(_) => self.eat(),
            }
        }
        self.builder.finish_node();
    }

    /// Entry: at `\`. `\`+newline is a hard break, `\`+special char is an
    /// escape, anything else leaves the backslash as literal text.
    /// Links, both spellings: `[[target]]` / `[[target|text]]` and
    /// `[text](target)`. Entry: at `[`. Any shape mismatch (no `]]`, no `]`,
    /// no `(...)`, or a block boundary in between) leaves the bracket as
    /// literal text.
    fn link(&mut self, stop: &impl Fn(&Self) -> bool) {
        if self.peek(1) == Some(SyntaxKind::LBracket) {
            self.wikilink(stop);
        } else {
            self.md_link(stop);
        }
    }

    fn wikilink(&mut self, stop: &impl Fn(&Self) -> bool) {
        let mut i = self.pos + 2;
        let mut pipe = None;
        let close = loop {
            match self.tokens.get(i) {
                None => break None,
                Some((SyntaxKind::Newline, _)) => break None,
                Some((SyntaxKind::Pipe, _)) if pipe.is_none() => {
                    pipe = Some(i);
                    i += 1;
                }
                Some((SyntaxKind::RBracket, _))
                    if self.tokens.get(i + 1).map(|t| t.0) == Some(SyntaxKind::RBracket) =>
                {
                    break Some(i)
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

    fn md_link(&mut self, stop: &impl Fn(&Self) -> bool) {
        let mut i = self.pos + 1;
        let close = loop {
            match self.tokens.get(i) {
                None => break None,
                Some((SyntaxKind::Newline, _)) if self.line_ends_block_at(i) => break None,
                Some((SyntaxKind::RBracket, _)) => break Some(i),
                Some(_) => i += 1,
            }
        };
        let Some(close) = close else {
            self.eat();
            return;
        };
        if self.tokens.get(close + 1).map(|t| t.0) != Some(SyntaxKind::LParen) {
            self.eat();
            return;
        }
        let mut j = close + 2;
        let close_paren = loop {
            match self.tokens.get(j) {
                None => break None,
                Some((SyntaxKind::Newline, _)) => break None,
                Some((SyntaxKind::RParen, _)) => break Some(j),
                Some(_) => j += 1,
            }
        };
        let Some(close_paren) = close_paren else {
            self.eat();
            return;
        };
        self.builder.start_node(SyntaxKind::Link.into());
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

    /// Code-mode placeholder: `#` + optional identifier + any number of
    /// immediately adjacent balanced `()`/`[]` groups. Never evaluated at
    /// this stage. A lone `#`, or a `#` whose group fails to balance within
    /// the block, degrades to a literal character.
    fn code_embed(&mut self) {
        let mut end = self.pos + 1;
        if matches!(self.tokens.get(end).map(|t| t.0), Some(SyntaxKind::Text)) {
            end += 1;
        }
        loop {
            let kind = self.tokens.get(end).map(|t| t.0);
            let (open, close) = match kind {
                Some(SyntaxKind::LParen) => (SyntaxKind::LParen, SyntaxKind::RParen),
                Some(SyntaxKind::LBracket) => (SyntaxKind::LBracket, SyntaxKind::RBracket),
                _ => break,
            };
            match self.balanced(end, open, close) {
                Some(e) => end = e + 1,
                None => {
                    self.eat();
                    return;
                }
            }
        }
        if end == self.pos + 1 {
            self.eat();
            return;
        }
        self.builder.start_node(SyntaxKind::CodeEmbed.into());
        while self.pos < end {
            self.eat();
        }
        self.builder.finish_node();
    }

    /// Index of the token closing the group opened at `from`, bounded by the
    /// enclosing block.
    fn balanced(&self, mut i: usize, open: SyntaxKind, close: SyntaxKind) -> Option<usize> {
        let mut depth = 0usize;
        loop {
            let (kind, _) = *self.tokens.get(i)?;
            if kind == SyntaxKind::Newline && self.line_ends_block_at(i) {
                return None;
            }
            if kind == open {
                depth += 1;
            }
            if kind == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            i += 1;
        }
    }

    fn escape_or_break(&mut self) {
        match self.peek(1) {
            Some(SyntaxKind::Newline) => {
                self.builder.start_node(SyntaxKind::HardBreak.into());
                self.eat();
                self.eat();
                self.builder.finish_node();
            }
            Some(kind) if is_escapable(kind) => {
                self.builder.start_node(SyntaxKind::Escape.into());
                self.eat();
                self.eat();
                self.builder.finish_node();
            }
            _ => self.eat(),
        }
    }

    /// Entry: at a `- ` / `+ ` marker, column 0. A list is a run of sibling
    /// items at one indent level; items deeper than `indent` nest, a blank
    /// line or dedent ends the list.
    fn list_at(&mut self, indent: usize) {
        self.builder.start_node(SyntaxKind::List.into());
        loop {
            self.list_item(indent);
            if self.cur() != Some(SyntaxKind::Newline) {
                break;
            }
            match self.next_line_marker_indent() {
                Some(next) if next == indent => self.eat(),
                _ => break,
            }
        }
        self.builder.finish_node();
    }

    /// Entry: at the (possibly indented) marker of one item. Consumes the
    /// marker line's inline content, then any nested list.
    fn list_item(&mut self, indent: usize) {
        self.builder.start_node(SyntaxKind::ListItem.into());
        while self.cur() == Some(SyntaxKind::Whitespace) {
            self.eat();
        }
        self.eat();
        self.eat();
        self.inline(|p: &Self| p.line_ends_list_item(p.pos, indent));
        if self.cur() == Some(SyntaxKind::Newline) {
            if let Some(next) = self.next_line_marker_indent() {
                if next > indent {
                    self.eat();
                    self.list_at(next);
                }
            }
        }
        self.builder.finish_node();
    }

    /// Indent width of the next line's marker, if the next line is a list
    /// item. Entry: current token is a newline.
    fn next_line_marker_indent(&self) -> Option<usize> {
        if self.cur() != Some(SyntaxKind::Newline) {
            return None;
        }
        let mut i = self.pos + 1;
        let mut indent = 0;
        while let Some((SyntaxKind::Whitespace, text)) = self.tokens.get(i) {
            indent += text.len();
            i += 1;
        }
        match self.tokens.get(i).map(|t| t.0) {
            Some(SyntaxKind::Minus) | Some(SyntaxKind::Plus)
                if self.tokens.get(i + 1).map(|t| t.0) == Some(SyntaxKind::Whitespace) =>
            {
                Some(indent)
            }
            _ => None,
        }
    }

    /// Stop condition for an item's inline content: the item ends at a blank
    /// line, at any further marker (sibling or nested — the item/list layer
    /// sorts out which), or at a continuation line indented no deeper than
    /// the marker.
    fn line_ends_list_item(&self, pos: usize, indent: usize) -> bool {
        let mut i = pos + 1;
        let mut next_indent = 0;
        while let Some((SyntaxKind::Whitespace, text)) = self.tokens.get(i) {
            next_indent += text.len();
            i += 1;
        }
        match self.tokens.get(i).map(|t| t.0) {
            None | Some(SyntaxKind::Newline) => true,
            Some(SyntaxKind::Minus) | Some(SyntaxKind::Plus)
                if self.tokens.get(i + 1).map(|t| t.0) == Some(SyntaxKind::Whitespace) =>
            {
                true
            }
            _ => next_indent <= indent,
        }
    }

    fn at_list_marker(&self) -> bool {
        matches!(self.cur(), Some(SyntaxKind::Minus) | Some(SyntaxKind::Plus))
            && self.peek(1) == Some(SyntaxKind::Whitespace)
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
    /// (heading marker, fence). Block markers are column-0 only; a marker
    /// preceded by whitespace is paragraph text.
    fn line_ends_block_at(&self, pos: usize) -> bool {
        let mut i = pos + 1;
        while self.tokens.get(i).map(|t| t.0) == Some(SyntaxKind::Whitespace) {
            i += 1;
        }
        if matches!(self.tokens.get(i).map(|t| t.0), None | Some(SyntaxKind::Newline)) {
            return true;
        }
        let next = self.tokens.get(pos + 1);
        match next.map(|t| t.0) {
            Some(SyntaxKind::Eq) => {
                self.tokens.get(pos + 2).map(|t| t.0) == Some(SyntaxKind::Whitespace)
            }
            Some(SyntaxKind::Backtick) => next.is_some_and(|t| t.1.len() >= 3),
            Some(SyntaxKind::Minus) | Some(SyntaxKind::Plus) => {
                self.tokens.get(pos + 2).map(|t| t.0) == Some(SyntaxKind::Whitespace)
            }
            _ => false,
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

    /// Token index to absolute byte offset. O(n); only used on diagnostic
    /// paths.
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

    /// The single funnel for consuming a token into the tree; also reports
    /// unterminated block comments (a lexical, trivia-level breakage, hence
    /// no `Error` wrapper).
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
