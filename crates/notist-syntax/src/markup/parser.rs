use std::cell::Cell;

use crate::content::{ContentFlavor, ParsedContent};
use crate::tree::SyntaxKind;
use crate::{Diagnostic, Parse};
use rowan::{TextRange, TextSize};

mod table;
use crate::code::parser::{Fragment, parse_fragment};
use crate::markup::MarkupContentParser;
use crate::parser::State;

/// An opener whose node is attached only once its closer appears. Rowan wraps
/// already-emitted tokens, so an unclosed opener costs nothing: its tokens stay
/// literal because no node is ever started.
#[derive(Clone, Copy)]
struct Open {
    delim: SyntaxKind,
    node: SyntaxKind,
    checkpoint: rowan::Checkpoint,
    /// Token index of the opener; a closer must leave non-empty content.
    at: usize,
}

/// A forward scan that reached the end of its region without a closer.
///
/// A scan searches for the closer token itself, not for a balanced shape, so
/// every narrower range inside `(start, end)` fails too. Recording the range
/// turns repeated failures on one line into a single scan.
#[derive(Clone, Copy)]
struct NoCloser {
    /// Where the scan stopped: a newline ending the block, or the region end.
    end: usize,
    /// The container limit the scan ran under.
    limit: Option<usize>,
}

#[derive(Default)]
struct Scans {
    math: [Cell<Option<NoCloser>>; 2],
    raw: Cell<Option<(usize, NoCloser)>>,
}

/// Parse a native Markup document without IO.
pub fn parse_document(src: &str) -> Parse {
    Parser::new(src).run()
}

struct Parser<'a> {
    state: State<'a>,
    block_indent: usize,
    block_start: Option<usize>,
    in_body: bool,
    /// Emphasis openers, innermost last. Cleared when an inline run ends.
    opens: Vec<Open>,
    /// Index of the first opener visible to the current inline run. A closer
    /// outside the run's scope cannot pair across a finished node.
    floor: usize,
    scans: Scans,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self::at(src, 0)
    }

    /// A parser whose first token starts at byte `start` of `src`.
    fn at(src: &'a str, start: usize) -> Self {
        Self {
            state: State::new(src, start, crate::lexed::Mode::Markup),
            block_indent: 0,
            block_start: None,
            in_body: false,
            opens: Vec::new(),
            floor: 0,
            scans: Scans::default(),
        }
    }
    /// Entry point: wraps the whole file in a `Document`. Only called at
    /// block boundaries; every arm there is at a line start.
    ///
    /// Block-level trivia (blank lines, standalone comment lines) belongs to
    /// `Document` directly, so a block's span is exactly its content.
    fn run(mut self) -> Parse {
        self.state.builder.start_node(SyntaxKind::Document.into());
        while let Some(kind) = self.cur() {
            match kind {
                SyntaxKind::Newline => self.eat(),
                SyntaxKind::Eq if self.at_heading_marker(0) => self.heading(),
                SyntaxKind::Backtick if self.at_fence(0) => self.raw_block(),
                SyntaxKind::Minus if self.at_divider_at(self.state.pos) => self.divider(),
                SyntaxKind::Minus | SyntaxKind::Plus if self.at_list_marker() => self.list_at(0),
                _ if self.at_table_at(self.state.pos) => self.table(),
                SyntaxKind::Whitespace if self.at_blank_line() => self.eat_blank_line(),
                SyntaxKind::LineComment | SyntaxKind::BlockComment => self.eat(),
                SyntaxKind::At => self.annotation(),
                SyntaxKind::Backslash if self.peek(1) == Some(SyntaxKind::Newline) => {
                    self.parbreak()
                }
                _ => self.inline(Self::line_ends_block),
            }
        }
        self.state.builder.finish_node();
        self.state.finish()
    }

    /// Entry: at `=` followed by whitespace, at a line start. The marker and
    /// its following whitespace are eaten before the inline content.
    fn heading(&mut self) {
        self.state.builder.start_node(SyntaxKind::Heading.into());
        self.eat();
        self.eat();
        self.inline(|_: &Self| true);
        self.state.builder.finish_node();
    }

    /// `\`+newline at block level: an explicit paragraph break, equivalent
    /// to a blank line.
    fn parbreak(&mut self) {
        self.state.builder.start_node(SyntaxKind::ParBreak.into());
        self.eat();
        self.eat();
        self.state.builder.finish_node();
    }

    /// Exactly `---` at a block start, with only optional trailing whitespace.
    /// The marker's span excludes trailing whitespace and the newline.
    fn divider(&mut self) {
        self.state.builder.start_node(SyntaxKind::Divider.into());
        for _ in 0..3 {
            self.eat();
        }
        self.state.builder.finish_node();
    }

    /// Entry: at a fence (`>= 3` backticks) at a line start. Consumes through
    /// the closing fence (equal or longer run) or container end; the closing
    /// fence's trailing newline belongs to the enclosing block sequence.
    ///
    /// An unclosed block swallows to container end and is reported; the `Error` wrapper
    /// is attached retroactively via the checkpoint taken at entry, so a
    /// broken block is structurally marked, not just diagnosed.
    fn raw_block(&mut self) {
        let checkpoint = self.state.builder.checkpoint();
        self.state.builder.start_node(SyntaxKind::Raw.into());
        let fence_len = self.state.lexed.len(self.state.pos);
        let fence_start = self.state.pos;
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
                && self.line_content_at(self.state.pos).1 == self.block_indent
                && self.peek(1) == Some(SyntaxKind::Backtick)
                && self.state.lexed.len(self.state.pos + 1) >= fence_len
            {
                self.eat();
            }
            match self.cur() {
                None => break,
                Some(SyntaxKind::Backtick) if self.state.lexed.len(self.state.pos) >= fence_len => {
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
        self.state.builder.finish_node();
        if !closed {
            self.state.diagnostics.push(Diagnostic {
                span: TextRange::new(
                    self.state.lexed.offset(fence_start),
                    self.state.lexed.offset(self.state.pos),
                ),
                message: "unclosed raw block".to_string(),
            });
            self.state
                .builder
                .start_node_at(checkpoint, SyntaxKind::Error.into());
            self.state.builder.finish_node();
        }
    }

    fn inline(&mut self, stop: impl Fn(&Self) -> bool) {
        self.inline_run(&stop);
    }

    /// The one inline loop; all inline constructs are dispatched here.
    ///
    /// `stop` is called only when the current token is a newline and decides
    /// whether the inline run ends there. `active` is the delimiter of the
    /// enclosing construct (e.g. `*` while inside a strong): the loop breaks
    /// before it if it may legally close, letting the caller consume it.
    fn inline_run(&mut self, stop: &impl Fn(&Self) -> bool) {
        // Openers are scoped to this run: a delimiter outside it cannot close
        // a node this run already finished.
        let base = self.opens.len();
        let floor = std::mem::replace(&mut self.floor, base);
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
                Some(SyntaxKind::Backslash) if self.peek(1) == Some(SyntaxKind::Newline) => break,
                Some(SyntaxKind::Backtick) => self.raw_inline(),
                Some(SyntaxKind::Star) => self.delimiter(SyntaxKind::Star, SyntaxKind::Strong),
                Some(SyntaxKind::Underscore) => {
                    self.delimiter(SyntaxKind::Underscore, SyntaxKind::Emph)
                }
                Some(SyntaxKind::Tilde) => self.delimiter(SyntaxKind::Tilde, SyntaxKind::Strike),
                Some(SyntaxKind::Dollar) => self.math(),
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
        self.floor = floor;
        self.opens.truncate(base);
    }

    /// A paired-delimiter construct (`*..*`, `_.._`, `~..~`). Entry: at the delimiter.
    ///
    /// An opener only records a checkpoint. The first later delimiter of the same
    /// kind that may legally close wraps everything since that checkpoint, which
    /// is the nearest matched closer. Delimiters that never pair stay literal
    /// text; nothing is scanned ahead and nothing is rewound.
    fn delimiter(&mut self, delim: SyntaxKind, node: SyntaxKind) {
        let pos = self.state.pos;
        let can_close = self.can_close_at(pos);
        if can_close {
            let index = self
                .opens
                .iter()
                .enumerate()
                .skip(self.floor)
                .filter(|(_, open)| open.delim == delim && open.at + 1 < pos)
                .map(|(index, _)| index)
                .next_back();
            if let Some(index) = index {
                self.eat();
                // Inner openers lose their closer when the outer pair closes.
                let open = self.opens[index];
                self.opens.truncate(index);
                self.state
                    .builder
                    .start_node_at(open.checkpoint, open.node.into());
                self.state.builder.finish_node();
                return;
            }
        }
        if !self.can_open_at(pos) {
            self.eat();
            return;
        }
        self.opens.push(Open {
            delim,
            node,
            checkpoint: self.state.builder.checkpoint(),
            at: pos,
        });
        self.eat();
    }

    /// An inline body: a run bounded by the body's `]`, the container end, or a
    /// blank line. A blank line that ends the body is diagnosed once.
    fn inline_in_body(&mut self) {
        // A body run owns its openers: a delimiter outside it must never wrap
        // tokens this run already emitted.
        let base = self.opens.len();
        let floor = std::mem::replace(&mut self.floor, base);
        loop {
            match self.cur() {
                None | Some(SyntaxKind::RBracket) => break,
                Some(SyntaxKind::Newline) => {
                    // A blank line ends the body.
                    if self.line_ends_block_at(self.state.pos) {
                        let point = self.state.offset(self.state.pos);
                        self.state.diagnostics.push(Diagnostic {
                            span: TextRange::empty(point),
                            message: "inline content cannot contain a blank line".into(),
                        });
                        break;
                    }
                    self.eat();
                }
                Some(SyntaxKind::Backslash) if self.peek(1) == Some(SyntaxKind::Newline) => break,
                Some(_) => self.inline_one(),
            }
        }
        self.floor = floor;
        self.opens.truncate(base);
    }

    /// One inline construct, dispatched without deciding where the run ends.
    fn inline_at(&mut self, stop: &impl Fn(&Self) -> bool) {
        match self.cur() {
            Some(SyntaxKind::Backtick) => self.raw_inline(),
            Some(SyntaxKind::Star) => self.delimiter(SyntaxKind::Star, SyntaxKind::Strong),
            Some(SyntaxKind::Underscore) => self.delimiter(SyntaxKind::Underscore, SyntaxKind::Emph),
            Some(SyntaxKind::Tilde) => self.delimiter(SyntaxKind::Tilde, SyntaxKind::Strike),
            Some(SyntaxKind::Dollar) => self.math(),
            Some(SyntaxKind::Bang) if self.peek(1) == Some(SyntaxKind::LBracket) => {
                self.md_link(stop, true)
            }
            Some(SyntaxKind::LBracket) => self.link(stop),
            Some(SyntaxKind::Hash) => self.code_call(),
            Some(SyntaxKind::At)
                if self.peek(1) == Some(SyntaxKind::LParen)
                    || (self.peek(1) == Some(SyntaxKind::Bang)
                        && self.peek(2) == Some(SyntaxKind::LParen)) =>
            {
                self.annotation()
            }
            Some(_) => self.eat(),
            None => {}
        }
    }

    /// Dispatch one inline construct or literal token, in a body.
    fn inline_one(&mut self) {
        self.inline_at(&|_: &Self| false);
    }

    /// Whether the token at `i` is whitespace or a newline.
    fn padded(&self, i: usize) -> bool {
        matches!(
            self.kind_at(i),
            Some(SyntaxKind::Whitespace | SyntaxKind::Newline)
        )
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

    /// `$x$` is inline math; `$ x $` is block math. Matching flanks determine
    /// the flavor. Block payloads own their newlines, including blank lines;
    /// inline payloads stay within the enclosing block. Empty or unclosed
    /// forms remain literal text.
    fn math(&mut self) {
        let pos = self.state.pos;
        match self.math_close_at(pos) {
            Some(close) if self.state.lexed.offset(close) > self.state.lexed.offset(pos + 1) => {
                self.state.builder.start_node(SyntaxKind::Math.into());
                while self.state.pos <= close {
                    self.eat();
                }
                self.state.builder.finish_node();
            }
            _ => self.eat(),
        }
    }

    /// The closer of the math opener at `start`, or `None`. A failed search is
    /// recorded for the region it covered, so later openers inside it fail
    /// without touching the token stream again.
    fn math_close_at(&self, start: usize) -> Option<usize> {
        let block = self.padded(start + 1);
        let slot = usize::from(block);
        if let Some(known) = self.scans.math[slot].get()
            && known.limit == self.state.limit
            && start < known.end
        {
            return None;
        }
        let padded = |i| {
            matches!(
                self.kind_at(i),
                Some(SyntaxKind::Whitespace | SyntaxKind::Newline)
            )
        };
        let mut end = start + 1;
        let stop = loop {
            match self.kind_at(end) {
                None => break end,
                Some(SyntaxKind::Newline) if !block && self.line_ends_block_at(end) => break end,
                Some(SyntaxKind::Dollar) if padded(end - 1) == block => {
                    let begin = usize::from(self.state.lexed.offset(start + 1));
                    let finish = usize::from(self.state.lexed.offset(end));
                    if !self.state.source[begin..finish].trim().is_empty() {
                        return Some(end);
                    }
                }
                Some(_) => {}
            }
            end += 1;
        };
        self.scans.math[slot].set(Some(NoCloser {
            end: stop,
            limit: self.state.limit,
        }));
        None
    }

    /// Inline raw: a backtick run closed by an equal-length run on the same
    /// line. Longer/shorter runs inside are content. Unclosed means it never
    /// was raw: the opening run stays literal text, no diagnostic.
    fn raw_inline(&mut self) {
        let pos = self.state.pos;
        match self.raw_inline_close_at(pos) {
            Some(close) => {
                self.state.builder.start_node(SyntaxKind::RawInline.into());
                while self.state.pos <= close {
                    self.eat();
                }
                self.state.builder.finish_node();
            }
            None => self.eat(),
        }
    }

    /// The closer of the backtick run at `start`: an equal-length run on the
    /// same line. Failures are remembered per run length for their region.
    fn raw_inline_close_at(&self, start: usize) -> Option<usize> {
        let run = self.state.lexed.len(start);
        if let Some((known_run, known)) = self.scans.raw.get()
            && known_run == run
            && known.limit == self.state.limit
            && start < known.end
        {
            return None;
        }
        let mut end = start + 1;
        let stop = loop {
            match self.kind_at(end) {
                None => break end,
                Some(SyntaxKind::Newline) => break end,
                Some(SyntaxKind::Backtick) if self.state.lexed.len(end) == run => {
                    return Some(end);
                }
                Some(_) => {}
            }
            end += 1;
        };
        self.scans.raw.set(Some((
            run,
            NoCloser {
                end: stop,
                limit: self.state.limit,
            },
        )));
        None
    }

    /// Opaque payload boundaries are shared by their parser and enclosing
    /// scanners, so internal delimiters cannot split a cell, label or body.
    fn opaque_close_at(&self, start: usize) -> Option<usize> {
        match self.kind_at(start) {
            Some(SyntaxKind::Backtick) => self.raw_inline_close_at(start),
            Some(SyntaxKind::Dollar) => self.math_close_at(start),
            _ => None,
        }
    }

    /// Links, both spellings. Entry: at `[` (or `!` for an embed).
    ///
    /// The label is parsed in place, so nothing is scanned twice. The node is
    /// attached only after the destination is known; a shape mismatch leaves
    /// the already-emitted tokens as plain content, exactly as if the bracket
    /// had been literal text.
    fn link(&mut self, stop: &impl Fn(&Self) -> bool) {
        if self.peek(1) == Some(SyntaxKind::LBracket) {
            self.wikilink();
        } else {
            self.md_link(stop, false);
        }
    }

    fn wikilink(&mut self) {
        let checkpoint = self.state.builder.checkpoint();
        self.eat();
        self.eat();
        // The target is raw text up to the first pipe or closing pair.
        while let Some(kind) = self.cur() {
            match kind {
                SyntaxKind::Newline => return,
                SyntaxKind::RBracket if self.peek(1) == Some(SyntaxKind::RBracket) => break,
                _ => self.eat(),
            }
        }
        if self.cur() != Some(SyntaxKind::RBracket) {
            return;
        }
        self.eat();
        self.eat();
        self.state
            .builder
            .start_node_at(checkpoint, SyntaxKind::WikiLink.into());
        self.state.builder.finish_node();
    }

    /// `[label](destination)` / `![label](destination)`. The label run ends at
    /// the first `]` that closes the outer bracket; a nested `[` keeps it open.
    fn md_link(&mut self, stop: &impl Fn(&Self) -> bool, embed: bool) {
        let checkpoint = self.state.builder.checkpoint();
        if embed {
            self.eat();
        }
        self.eat();
        let mut depth = 1usize;
        let base = self.opens.len();
        let floor = std::mem::replace(&mut self.floor, base);
        let closed = loop {
            match self.cur() {
                None => {
                    self.floor = floor;
                    self.opens.truncate(base);
                    return;
                }
                Some(SyntaxKind::Newline) if self.line_ends_block_at(self.state.pos) => {
                    self.floor = floor;
                    self.opens.truncate(base);
                    return;
                }
                Some(SyntaxKind::RBracket) if depth == 1 => {
                    self.eat();
                    break true;
                }
                Some(SyntaxKind::RBracket) => {
                    depth -= 1;
                    self.eat();
                }
                Some(SyntaxKind::LBracket) => {
                    depth += 1;
                    self.eat();
                }
                Some(SyntaxKind::Backslash) if self.peek(1) == Some(SyntaxKind::Newline) => {
                    self.floor = floor;
                    self.opens.truncate(base);
                    return;
                }
                Some(_) => self.inline_at(stop),
            }
        };
        let _ = closed;
        self.floor = floor;
        self.opens.truncate(base);
        if self.cur() != Some(SyntaxKind::LParen) {
            return;
        }
        // The destination is raw text: parens nest, quotes shield them, and a
        // backslash escapes the next character. A line break ends it.
        self.eat();
        let mut parens = 1usize;
        let mut quote: Option<char> = None;
        let mut escaped = false;
        let mut previous_space = false;
        loop {
            match self.cur() {
                None | Some(SyntaxKind::Newline) => return,
                Some(SyntaxKind::RParen) if parens == 1 && quote.is_none() => {
                    self.eat();
                    break;
                }
                Some(_) => {
                    for ch in self.state.lexed.text(self.state.pos).chars() {
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
                    self.eat();
                }
            }
        }
        self.state.builder.start_node_at(
            checkpoint,
            if embed {
                SyntaxKind::Embed
            } else {
                SyntaxKind::Link
            }
            .into(),
        );
        self.state.builder.finish_node();
    }

    /// `#name(args)[body]` — an atomic constructor call; `#[body]` is the
    /// anonymous form, producing a transparent group node. Anything else
    /// after `#` (bare `#(..)`, a broken path) is literal text.
    ///
    /// The prefix commits: once `#` is followed by a name or `[`, the call is
    /// parsed in place on this cursor, and its body belongs to the same node.
    fn code_call(&mut self) {
        let start = usize::from(self.state.offset(self.state.pos));
        let rest = &self.state.source[start..];
        if crate::code::is_markup_for(rest) {
            self.state.set_mode(crate::lexed::Mode::Module);
            parse_fragment(&mut self.state, &MarkupContentParser, Fragment::For);
            self.state.set_mode(crate::lexed::Mode::Markup);
            return;
        }
        if crate::code::is_markup_import(rest) {
            self.import_decl();
            return;
        }
        if call_shape(rest).is_none() {
            self.eat();
            return;
        }
        self.state.set_mode(crate::lexed::Mode::Literal);
        self.state.in_markup_args = true;
        parse_fragment(&mut self.state, &MarkupContentParser, Fragment::CallHeader);
        self.state.in_markup_args = false;
        self.state.set_mode(crate::lexed::Mode::Markup);
        // The body, when present, is part of the same call node.
        if self.cur() == Some(SyntaxKind::LBracket) {
            self.bracket_body();
        }
        self.state.builder.finish_node();
    }

    fn import_decl(&mut self) {
        let start = usize::from(self.state.offset(self.state.pos));
        self.state.set_mode(crate::lexed::Mode::Module);
        parse_fragment(&mut self.state, &MarkupContentParser, Fragment::Import);
        self.state.set_mode(crate::lexed::Mode::Markup);
        let end = usize::from(self.state.offset(self.state.pos));
        let line_end = self.state.source[start..]
            .find(['\r', '\n'])
            .map_or(self.state.source.len(), |n| start + n);
        let before = self.state.source[..start]
            .rsplit(['\r', '\n'])
            .next()
            .unwrap_or_default();
        let after = self
            .state
            .source
            .get(end..line_end.max(end))
            .unwrap_or_default()
            .trim();
        if end > line_end || !before.trim().is_empty() || !after.is_empty() && !after.starts_with("//")
        {
            self.state.diagnostics.push(Diagnostic {
                span: TextRange::new((start as u32).into(), (end as u32).into()),
                message: "import declarations must occupy their own line".into(),
            });
        }
    }

    /// The bracketed content at cur == `[`. Its flavor comes from the opening
    /// side only; an inline body ends at a blank line (diagnosed) and a block
    /// body at the closing bracket or the container end.
    fn bracket_body(&mut self) {
        let outer = self.state.lexed.mode();
        self.eat();
        self.state.set_mode(crate::lexed::Mode::Markup);
        let block = self.padded(self.state.pos);
        let was = std::mem::replace(&mut self.in_body, true);
        if block {
            self.block_body();
        } else {
            // An inline body ends at a blank line: the paragraph break closes
            // it, so following blocks keep their own structure.
            self.inline_in_body();
        }
        self.in_body = was;
        self.state.set_mode(outer);
        if self.cur() == Some(SyntaxKind::RBracket) {
            self.eat();
        } else {
            let point = self.state.offset(self.state.pos);
            self.state.diagnostics.push(Diagnostic {
                span: TextRange::new(point, point),
                message: "unclosed code call body".into(),
            });
        }
    }

    fn diagnose_inline_parbreak(&mut self, open: usize, close: usize) {
        let mut i = open + 1;
        while i < close && self.kind_at(i).is_some() {
            // Nested calls own their content slots and diagnose their own
            // inline bodies. Their block bodies may contain blank lines.
            // A nested call's own inline body diagnoses itself when parsed.
            if self.kind_at(i) == Some(SyntaxKind::Hash) {
                i += 1;
                continue;
            }
            if let Some(end) = self.opaque_close_at(i) {
                i = end + 1;
                continue;
            }
            if self.kind_at(i) == Some(SyntaxKind::Newline) && self.line_ends_block_at(i) {
                let point = self.state.lexed.offset(i);
                self.state.diagnostics.push(Diagnostic {
                    span: TextRange::new(point, point),
                    message: "inline content cannot contain a blank line".to_string(),
                });
                return;
            }
            i += 1;
        }
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
                Some(SyntaxKind::Minus) if self.at_divider_at(self.state.pos) => self.divider(),
                Some(SyntaxKind::Minus) | Some(SyntaxKind::Plus) if self.at_list_marker() => {
                    self.list_at(self.column_at(self.state.pos))
                }
                _ if self.at_table_at(self.state.pos) => self.table(),
                Some(SyntaxKind::Whitespace) if self.at_blank_line() => self.eat_blank_line(),
                Some(SyntaxKind::Whitespace) if self.block_indent > 0 => self.eat(),
                Some(SyntaxKind::LineComment) | Some(SyntaxKind::BlockComment) => self.eat(),
                Some(SyntaxKind::At) => self.annotation(),
                Some(SyntaxKind::Backslash) if self.peek(1) == Some(SyntaxKind::Newline) => {
                    self.parbreak()
                }
                _ => self.inline(Self::line_ends_block),
            }
        }
    }

    /// A structural sequence of same-type items. Blank lines between items
    /// belong to the list; an item's indented block sequence owns its body.
    fn list_at(&mut self, indent: usize) {
        let marker = self.cur();
        self.state.builder.start_node(SyntaxKind::List.into());
        loop {
            self.list_item(indent);
            if self.cur() != Some(SyntaxKind::Newline) {
                break;
            }
            let mut next = self.state.pos + 1;
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
            while self.state.pos < next {
                self.eat();
            }
        }
        self.state.builder.finish_node();
    }

    fn list_item(&mut self, indent: usize) {
        self.state.builder.start_node(SyntaxKind::ListItem.into());
        self.eat(); // marker
        let body_indent =
            self.state
                .lexed
                .text(self.state.pos)
                .chars()
                .fold(indent + 1, |column, ch| {
                    if ch == '\t' {
                        (column / 4 + 1) * 4
                    } else {
                        column + 1
                    }
                });
        self.eat(); // marker whitespace
        let start = self.state.pos;
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
        let old_limit = self.state.limit.replace(end);
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
                SyntaxKind::Minus if self.at_divider_at(self.state.pos) => self.divider(),
                SyntaxKind::Minus | SyntaxKind::Plus if self.at_list_marker() => {
                    self.list_at(self.column_at(self.state.pos))
                }
                _ if self.at_table_at(self.state.pos) => self.table(),
                SyntaxKind::At => self.annotation(),
                SyntaxKind::Backslash if self.peek(1) == Some(SyntaxKind::Newline) => {
                    self.parbreak()
                }
                _ => self.inline(Self::line_ends_block),
            }
        }
        self.block_start = old_start;
        self.block_indent = old_indent;
        self.state.limit = old_limit;
        self.state.builder.finish_node();
    }

    fn line_content_at(&self, mut pos: usize) -> (usize, usize) {
        let mut width = 0;
        while self.kind_at(pos) == Some(SyntaxKind::Whitespace) {
            width = self
                .state
                .lexed
                .text(pos)
                .chars()
                .fold(width, |column, ch| {
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
        let offset = usize::from(self.state.lexed.offset(pos));
        let start = self.state.source[..offset]
            .rfind(['\n', '\r'])
            .map_or(0, |i| i + 1);
        self.state.source[start..offset]
            .chars()
            .fold(0, |column, ch| {
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
        let offset = usize::from(self.state.lexed.offset(pos));
        let column = self.column_at(pos);
        let start = self.state.source[..offset]
            .rfind(['\n', '\r'])
            .map_or(0, |i| i + 1);
        let prefix = &self.state.source[start..offset];
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
            Some(SyntaxKind::Newline) => self.line_ends_block_at(self.state.pos),
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
            Some(SyntaxKind::Backtick) => self.state.lexed.len(next) >= 3,
            Some(SyntaxKind::Minus) | Some(SyntaxKind::Plus) => {
                self.kind_at(next + 1) == Some(SyntaxKind::Whitespace) || self.at_divider_at(next)
            }
            Some(SyntaxKind::At) => true,
            Some(SyntaxKind::Hash) => crate::code::is_markup_import(
                &self.state.source[usize::from(self.state.lexed.offset(next))..],
            ),
            _ => false,
        }
    }

    fn at_heading_marker(&self, ws_skip: usize) -> bool {
        self.peek(ws_skip) == Some(SyntaxKind::Eq)
            && self.peek(ws_skip + 1) == Some(SyntaxKind::Whitespace)
    }

    fn at_fence(&self, ws_skip: usize) -> bool {
        self.peek(ws_skip) == Some(SyntaxKind::Backtick)
            && self.state.lexed.len(self.state.pos + ws_skip) >= 3
    }

    fn at_blank_line(&self) -> bool {
        let mut i = self.state.pos;
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
}

impl Parser<'_> {
    fn kind_at(&self, i: usize) -> Option<SyntaxKind> {
        self.state.kind_at(i)
    }
    fn cur(&self) -> Option<SyntaxKind> {
        self.kind_at(self.state.pos)
    }
    fn peek(&self, offset: usize) -> Option<SyntaxKind> {
        self.kind_at(self.state.pos + offset)
    }
    fn eat(&mut self) {
        self.state.eat();
    }

    /// `@(dict)` and `@!(dict)`. The payload is parsed in place; the node kind
    /// depends on where the line ends, so the node is attached retroactively.
    fn annotation(&mut self) {
        let checkpoint = self.state.builder.checkpoint();
        self.state.set_mode(crate::lexed::Mode::Literal);
        parse_fragment(
            &mut self.state,
            &MarkupContentParser,
            Fragment::AnnotationBody,
        );
        self.state.set_mode(crate::lexed::Mode::Markup);
        let mut i = self.state.pos;
        while self.kind_at(i) == Some(SyntaxKind::Whitespace) {
            i += 1;
        }
        let block = matches!(
            self.kind_at(i),
            None | Some(SyntaxKind::Newline | SyntaxKind::LineComment)
        );
        self.state.builder.start_node_at(
            checkpoint,
            if block {
                SyntaxKind::BlockAnnotation
            } else {
                SyntaxKind::Annotation
            }
            .into(),
        );
        self.state.builder.finish_node();
    }
}

impl Parser<'_> {
    /// Token index just past a complete call starting at `start`, if the prefix
    /// forms one. Used by row planning, which must not split a cell inside a
    /// call's arguments or body.
    pub(super) fn call_end(&self, start: usize) -> Option<usize> {
        if self.kind_at(start) != Some(SyntaxKind::Hash) {
            return None;
        }
        let offset = usize::from(self.state.offset(start));
        let named = call_shape(&self.state.source[offset..])?;
        let rest = &self.state.source[offset..];
        let after_path = if named {
            path_end_bytes(rest.as_bytes(), 1)?
        } else {
            1
        };
        let has_args = rest.as_bytes().get(after_path) == Some(&b'(');
        let mut i = start + 1;
        if has_args {
            while let Some(kind) = self.kind_at(i) {
                match kind {
                    SyntaxKind::LParen => break,
                    SyntaxKind::Newline if self.line_ends_block_at(i) => return None,
                    _ => i += 1,
                }
            }
            let mut depth = 0usize;
            while let Some(kind) = self.kind_at(i) {
                match kind {
                    SyntaxKind::LParen => depth += 1,
                    SyntaxKind::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    SyntaxKind::Newline if self.line_ends_block_at(i) => return None,
                    _ => {}
                }
                i += 1;
            }
            if depth != 0 {
                return None;
            }
        }
        // Compile-time arguments sit between the path and the body.
        while matches!(self.kind_at(i), Some(SyntaxKind::Less)) {
            let mut depth = 0usize;
            while let Some(kind) = self.kind_at(i) {
                match kind {
                    SyntaxKind::Less => depth += 1,
                    SyntaxKind::Greater => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    SyntaxKind::Newline => return None,
                    _ => {}
                }
                i += 1;
            }
        }
        if self.kind_at(i) != Some(SyntaxKind::LBracket) {
            return None;
        }
        let mut depth = 0usize;
        while let Some(kind) = self.kind_at(i) {
            match kind {
                SyntaxKind::LBracket => depth += 1,
                SyntaxKind::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None
    }
}

/// Whether `#` at the start of `rest` begins a call. A path name always does;
/// a bare `#` only when a body bracket follows. Everything else is prose.
fn call_shape(rest: &str) -> Option<bool> {
    let bytes = rest.as_bytes();
    let named = matches!(bytes.get(1), Some(b) if b.is_ascii_alphabetic() || *b == b'_' || *b >= 0x80);
    if named {
        path_end_bytes(bytes, 1)?;
        return Some(true);
    }
    (bytes.get(1) == Some(&b'[')).then_some(false)
}

/// End of an identifier run starting at `i`. Identifiers use the literal
/// lexer's rule: alphabetic (or non-ASCII) first, then alphanumeric, `_`, `-`.
fn ident_end_bytes(bytes: &[u8], i: usize) -> usize {
    let mut j = i;
    while let Some(b) = bytes.get(j) {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-') || *b >= 0x80 {
            j += 1;
        } else {
            break;
        }
    }
    j
}

fn path_end_bytes(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = ident_end_bytes(bytes, i);
    while bytes.get(j) == Some(&b'.') {
        if ident_end_bytes(bytes, j + 1) == j + 1 {
            return None;
        }
        j = ident_end_bytes(bytes, j + 1);
    }
    Some(j)
}

/// The body of a `[...]` content literal opening at `open`.
struct Extent {
    /// Token index of the closing `]`, when one exists.
    close: Option<usize>,
    /// Byte end of the literal, including the closing bracket when present.
    end: usize,
}

/// Delimit the content literal opening at `open` with a line-bounded scan over
/// the same token rules as the grammar. Raw inline, math and fences are opaque:
/// a bracket inside them does not close the literal. Callers then parse only
/// the literal's own text, so a literal never costs more than that text.
fn content_extent(src: &str, open: usize) -> Extent {
    let tape = crate::lexed::Lexed::new(src, open, crate::lexed::Mode::Markup);
    let mut depth = 0usize;
    let mut i = 0usize;
    while let Some(kind) = tape.kind(i) {
        match kind {
            SyntaxKind::LBracket => depth += 1,
            SyntaxKind::RBracket => {
                depth -= 1;
                if depth == 0 {
                    return Extent {
                        close: Some(i),
                        end: usize::from(tape.offset(i)) + 1,
                    };
                }
            }
            SyntaxKind::Backtick => {
                let run = tape.len(i);
                let block = run >= 3
                    && src[..usize::from(tape.offset(i))]
                        .rsplit('\n')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .is_empty();
                let mut j = i + 1;
                let mut found = false;
                while let Some(kind) = tape.kind(j) {
                    if kind == SyntaxKind::Newline && !block {
                        break;
                    }
                    if kind == SyntaxKind::Backtick && (tape.len(j) == run || block && tape.len(j) >= run)
                    {
                        i = j;
                        found = true;
                        break;
                    }
                    j += 1;
                }
                if block && !found {
                    // An unclosed fence swallows the rest of the input.
                    return Extent {
                        close: None,
                        end: src.len(),
                    };
                }
            }
            SyntaxKind::Dollar => {
                let padded = |k: usize| {
                    matches!(
                        tape.kind(k),
                        Some(SyntaxKind::Whitespace | SyntaxKind::Newline)
                    )
                };
                let block = padded(i + 1);
                let mut j = i + 1;
                while let Some(kind) = tape.kind(j) {
                    if !block && kind == SyntaxKind::Newline {
                        break;
                    }
                    if kind == SyntaxKind::Dollar && padded(j - 1) == block {
                        i = j;
                        break;
                    }
                    j += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    Extent {
        close: None,
        end: src.len(),
    }
}

/// Parse native content without embedding its tree into the Code CST.
///
/// The literal is delimited before parsing, and the resulting tree keeps
/// absolute source coordinates: its tokens already carry the source offsets of
/// the enclosing file.
pub(crate) fn parse_content(source: &str, open: TextSize) -> ParsedContent {
    let open_at = usize::from(open);
    debug_assert!(source[open_at..].starts_with('['));
    let extent = content_extent(source, open_at);
    let body_end = extent.end - usize::from(extent.close.is_some());
    let block = matches!(
        source.as_bytes().get(open_at + 1),
        None | Some(b' ' | b'\t' | b'\r' | b'\n')
    );
    let mut parser = Parser::at(source, open_at);
    parser.state.pos = 1;
    parser.state.limit = extent.close;
    parser.in_body = true;
    parser.state.builder.start_node(SyntaxKind::Document.into());
    if block {
        parser.block_body();
    } else {
        parser.diagnose_inline_parbreak(0, extent.close.unwrap_or(usize::MAX));
        parser.inline_run(&|_: &Parser<'_>| false);
    }
    while parser.cur().is_some() {
        parser.eat();
    }
    parser.state.builder.finish_node();
    let mut parsed = parser.state.finish();
    let mut diagnostics = std::mem::take(&mut parsed.diagnostics);
    if extent.close.is_none() {
        diagnostics.push(Diagnostic {
            span: TextRange::empty((extent.end as u32).into()),
            message: "unclosed content literal".into(),
        });
    }
    ParsedContent {
        range: TextRange::new(open, (extent.end as u32).into()),
        body: TextRange::new(open + TextSize::new(1), (body_end as u32).into()),
        flavor: if block {
            ContentFlavor::Block
        } else {
            ContentFlavor::Inline
        },
        closed: extent.close.is_some(),
        syntax: Box::new(parsed),
        diagnostics,
    }
}
