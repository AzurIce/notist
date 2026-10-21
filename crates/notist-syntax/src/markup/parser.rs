use crate::content::{ContentFlavor, ParsedContent};
use crate::tree::SyntaxKind;
use crate::{Diagnostic, Parse};
use rowan::{NodeOrToken, TextRange, TextSize};

mod state;
mod table;
use state::State;

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

/// Shared lookahead for parsing calls and finding table cell boundaries.
struct CallFrame {
    named: bool,
    has_args: bool,
    body_start: usize,
    body: Option<Balanced>,
}

impl CallFrame {
    /// Only complete calls protect their contents from table separators.
    fn end(&self) -> Option<usize> {
        match self.body {
            Some(Balanced::Closed(close)) if self.named || !self.has_args => Some(close + 1),
            None if self.named && self.has_args => Some(self.body_start),
            _ => None,
        }
    }
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
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            state: State::new(src),
            block_indent: 0,
            block_start: None,
            in_body: false,
        }
    }
    fn at_block_annotation(&self) -> bool {
        let mut i = self.state.pos + 1;
        if self.kind_at(i) == Some(SyntaxKind::Bang) {
            i += 1;
        }
        if self.kind_at(i) != Some(SyntaxKind::LParen) {
            return true; // malformed payload: let the block path diagnose it
        }
        match self.balanced(i, SyntaxKind::LParen, SyntaxKind::RParen, false) {
            Balanced::Closed(end) => {
                let mut j = end + 1;
                while self.kind_at(j) == Some(SyntaxKind::Whitespace) {
                    j += 1;
                }
                matches!(
                    self.kind_at(j),
                    None | Some(SyntaxKind::Newline | SyntaxKind::LineComment)
                )
            }
            _ => true,
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
                SyntaxKind::At if self.at_block_annotation() => self.annotation(true),
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
                    self.annotation(false)
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
            Active::Flanked(delim) => kind == delim && self.can_close_at(self.state.pos),
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
        if !self.can_open_at(self.state.pos) {
            self.eat();
            return;
        }
        match self.find_close(self.state.pos + 1, delim) {
            Some(close) if close > self.state.pos + 1 => {
                self.state.builder.start_node(node.into());
                self.eat();
                self.inline_delimited(stop, Some(Active::Flanked(delim)));
                // the walk may have stopped at a body's `]` instead of the closer
                if self.cur() == Some(delim) {
                    self.eat();
                }
                self.state.builder.finish_node();
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

    /// `$x$` is inline math; `$ x $` is block math. Matching flanks determine
    /// the flavor. Block payloads own their newlines, including blank lines;
    /// inline payloads stay within the enclosing block. Empty or unclosed
    /// forms remain literal text.
    fn math(&mut self) {
        let Some(close) = self.math_close_at(self.state.pos) else {
            self.eat();
            return;
        };
        self.state.builder.start_node(SyntaxKind::Math.into());
        while self.state.pos <= close {
            self.eat();
        }
        self.state.builder.finish_node();
    }

    fn math_close_at(&self, start: usize) -> Option<usize> {
        let padded = |i| {
            matches!(
                self.kind_at(i),
                Some(SyntaxKind::Whitespace | SyntaxKind::Newline)
            )
        };
        let block = padded(start + 1);
        let mut end = start + 1;
        while let Some(kind) = self.kind_at(end) {
            if !block && kind == SyntaxKind::Newline && self.line_ends_block_at(end) {
                break;
            }
            if kind == SyntaxKind::Dollar && padded(end - 1) == block {
                let begin = usize::from(self.state.lexed.offset(start + 1));
                let finish = usize::from(self.state.lexed.offset(end));
                return (!self.state.source[begin..finish].trim().is_empty()).then_some(end);
            }
            end += 1;
        }
        None
    }

    /// Inline raw: a backtick run closed by an equal-length run on the same
    /// line. Longer/shorter runs inside are content. Unclosed means it never
    /// was raw: the opening run stays literal text, no diagnostic.
    fn raw_inline(&mut self) {
        let Some(close) = self.raw_inline_close_at(self.state.pos) else {
            self.eat();
            return;
        };
        self.state.builder.start_node(SyntaxKind::RawInline.into());
        while self.state.pos <= close {
            self.eat();
        }
        self.state.builder.finish_node();
    }

    fn raw_inline_close_at(&self, start: usize) -> Option<usize> {
        let mut end = start + 1;
        while let Some(kind) = self.kind_at(end) {
            if kind == SyntaxKind::Newline {
                break;
            }
            if kind == SyntaxKind::Backtick
                && self.state.lexed.len(end) == self.state.lexed.len(start)
            {
                return Some(end);
            }
            end += 1;
        }
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
        let mut i = self.state.pos + 2;
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
        self.state.builder.start_node(SyntaxKind::WikiLink.into());
        self.eat();
        self.eat();
        let target_end = pipe.unwrap_or(close);
        while self.state.pos < target_end {
            self.eat();
        }
        if pipe.is_some() {
            self.eat();
            self.inline_delimited(stop, Some(Active::Pair(SyntaxKind::RBracket)));
        }
        self.eat();
        self.eat();
        self.state.builder.finish_node();
    }

    fn md_link(&mut self, stop: &impl Fn(&Self) -> bool, embed: bool) {
        let open = self.state.pos + usize::from(embed);
        let mut i = open + 1;
        let mut depth = 1usize;
        let close = loop {
            match self.kind_at(i) {
                None => break None,
                Some(SyntaxKind::Newline) if self.line_ends_block_at(i) => break None,
                Some(SyntaxKind::Backslash) if self.kind_at(i + 1) == Some(SyntaxKind::Newline) => {
                    break None;
                }
                Some(SyntaxKind::Backtick | SyntaxKind::Dollar) => {
                    // Brackets inside opaque inline payloads are not label delimiters.
                    i = self.opaque_close_at(i).map_or(i + 1, |end| end + 1);
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
                    for ch in self.state.lexed.text(j).chars() {
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
        self.state.builder.start_node(
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
        while self.state.pos < close_paren {
            self.eat();
        }
        self.eat();
        self.state.builder.finish_node();
    }

    /// `#name(args)[body]` — an atomic constructor call; `#[body]` is the
    /// anonymous form, producing a transparent group node. Anything else
    /// after `#` (bare `#name`, `#(..)`) is literal text.
    fn code_call(&mut self) {
        if crate::code::is_markup_import(
            &self.state.source[usize::from(self.state.lexed.offset(self.state.pos))..],
        ) {
            self.import_decl();
            return;
        }
        let Some(frame) = self.code_call_at(self.state.pos) else {
            self.eat();
            return;
        };
        let body_close = match frame.body {
            Some(Balanced::Closed(close)) => Some(close),
            Some(Balanced::Unclosed(eof)) => {
                let point = self.state.lexed.offset(eof);
                self.state.diagnostics.push(Diagnostic {
                    span: TextRange::new(point, point),
                    message: "unclosed code call body".to_string(),
                });
                None
            }
            Some(Balanced::BlockEnd) => unreachable!("body scans cross blocks"),
            None => None,
        };
        let has_body = body_close.is_some();
        let is_call = frame.named || (!frame.named && !frame.has_args && has_body);
        if !is_call {
            self.eat();
            return;
        }
        self.state.builder.start_node(SyntaxKind::CodeCall.into());
        let start = self.state.lexed.offset(self.state.pos);
        let end = self.state.lexed.offset(frame.body_start);
        let fragment = crate::code::parse_call_prefix(
            &self.state.source[usize::from(start)..usize::from(end)],
            &crate::markup::MarkupContentParser,
        );
        self.import_fragment(fragment, true, None);
        if let Some(close) = body_close {
            let block = self.is_block_body(frame.body_start, close);
            if !block {
                self.diagnose_inline_parbreak(frame.body_start, close);
            }
            self.bracket_body(block);
        }
        self.state.builder.finish_node();
    }

    fn import_decl(&mut self) {
        let start = usize::from(self.state.lexed.offset(self.state.pos));
        let end = self.state.source[start..]
            .find(['\r', '\n'])
            .map_or(self.state.source.len(), |n| start + n);
        let mut parsed = crate::code::parse_import_prefix(&self.state.source[start..end]);
        let target = TextSize::new(start as u32) + parsed.consumed();
        let mut boundary = self.state.pos;
        while self.state.lexed.offset(boundary) < target {
            boundary += 1;
        }
        let recovery_end = usize::from(self.state.lexed.offset(boundary));
        if self.state.lexed.offset(boundary) != target {
            parsed = crate::code::parse_import(&self.state.source[start..recovery_end]);
        }
        let consumed = usize::from(parsed.consumed());
        let before = self.state.source[..start]
            .rsplit(['\r', '\n'])
            .next()
            .unwrap_or_default();
        let after = self
            .state
            .source
            .get(start + consumed..end)
            .unwrap_or_default()
            .trim();
        if recovery_end > end
            || !before.trim().is_empty()
            || !after.is_empty() && !after.starts_with("//")
        {
            self.state.diagnostics.push(Diagnostic {
                span: TextRange::new((start as u32).into(), ((start + consumed) as u32).into()),
                message: "import declarations must occupy their own line".into(),
            });
        }
        self.import_fragment(parsed, false, None);
    }

    fn code_call_at(&self, start: usize) -> Option<CallFrame> {
        let named = self.kind_at(start + 1) == Some(SyntaxKind::Ident);
        let mut args_at = if named {
            self.path_end(start + 1)?
        } else {
            start + 1
        };
        if self.kind_at(args_at) == Some(SyntaxKind::Less) {
            args_at = match self.balanced(args_at, SyntaxKind::Less, SyntaxKind::Greater, false) {
                Balanced::Closed(close) => close + 1,
                _ => return None,
            };
        }
        let has_args = self.kind_at(args_at) == Some(SyntaxKind::LParen);
        let body_start = if has_args {
            match self.balanced(args_at, SyntaxKind::LParen, SyntaxKind::RParen, false) {
                Balanced::Closed(close) => close + 1,
                _ => return None,
            }
        } else {
            args_at
        };
        let body = (self.kind_at(body_start) == Some(SyntaxKind::LBracket))
            .then(|| self.balanced(body_start, SyntaxKind::LBracket, SyntaxKind::RBracket, true));
        Some(CallFrame {
            named,
            has_args,
            body_start,
            body,
        })
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
        let mut i = open + 1;
        while i < close {
            // Nested calls own their content slots and diagnose their own
            // inline bodies. Their block bodies may contain blank lines.
            if self.kind_at(i) == Some(SyntaxKind::Hash)
                && let Some(end) = self.code_call_at(i).and_then(|frame| frame.end())
            {
                i = end;
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
                Some(SyntaxKind::Minus) if self.at_divider_at(self.state.pos) => self.divider(),
                Some(SyntaxKind::Minus) | Some(SyntaxKind::Plus) if self.at_list_marker() => {
                    self.list_at(self.column_at(self.state.pos))
                }
                _ if self.at_table_at(self.state.pos) => self.table(),
                Some(SyntaxKind::Whitespace) if self.at_blank_line() => self.eat_blank_line(),
                Some(SyntaxKind::Whitespace) if self.block_indent > 0 => self.eat(),
                Some(SyntaxKind::LineComment) | Some(SyntaxKind::BlockComment) => self.eat(),
                Some(SyntaxKind::At) if self.at_block_annotation() => self.annotation(true),
                Some(SyntaxKind::Backslash) if self.peek(1) == Some(SyntaxKind::Newline) => {
                    self.parbreak()
                }
                _ => self.inline(Self::line_ends_block),
            }
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
        let mut grouped = 0usize;
        loop {
            let Some(kind) = self.kind_at(i) else {
                return Balanced::Unclosed(i);
            };
            if !cross_blocks && kind == SyntaxKind::Newline && self.line_ends_block_at(i) {
                return Balanced::BlockEnd;
            }
            if cross_blocks && let Some(end) = self.opaque_close_at(i) {
                i = end + 1;
                continue;
            }
            if open == SyntaxKind::Less {
                match kind {
                    SyntaxKind::LParen | SyntaxKind::LBrace => grouped += 1,
                    SyntaxKind::RParen | SyntaxKind::RBrace => grouped = grouped.saturating_sub(1),
                    _ => {}
                }
            }
            if kind == open && grouped == 0 {
                depth += 1;
            }
            if kind == close && grouped == 0 {
                depth -= 1;
                if depth == 0 {
                    return Balanced::Closed(i);
                }
            }
            i += 1;
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
                SyntaxKind::At if self.at_block_annotation() => self.annotation(true),
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

    // Native lookahead only; actual path construction belongs to Code syntax.
    fn path_end(&self, from: usize) -> Option<usize> {
        if self.kind_at(from) != Some(SyntaxKind::Ident) {
            return None;
        }
        let mut end = from + 1;
        while self.kind_at(end) == Some(SyntaxKind::Dot) {
            if self.kind_at(end + 1) != Some(SyntaxKind::Ident) {
                return None;
            }
            end += 2;
        }
        Some(end)
    }

    fn annotation(&mut self, block: bool) {
        let start = self.state.lexed.offset(self.state.pos);
        let limit = self.state.limit.map_or(self.state.source.len(), |i| {
            usize::from(self.state.lexed.offset(i))
        });
        let parsed = crate::code::parse_annotation_prefix(
            &self.state.source[usize::from(start)..limit],
            &crate::markup::MarkupContentParser,
        );
        self.import_fragment(parsed, false, block.then_some(SyntaxKind::BlockAnnotation));
    }

    fn import_fragment(&mut self, parsed: Parse, header: bool, root_kind: Option<SyntaxKind>) {
        fn copy(
            builder: &mut rowan::GreenNodeBuilder<'_>,
            node: crate::tree::SyntaxNode,
            kind: Option<SyntaxKind>,
        ) {
            builder.start_node(kind.unwrap_or(node.kind()).into());
            for element in node.children_with_tokens() {
                match element {
                    NodeOrToken::Node(node) => copy(builder, node, None),
                    NodeOrToken::Token(token) => builder.token(token.kind().into(), token.text()),
                }
            }
            builder.finish_node();
        }
        let start = self.state.lexed.offset(self.state.pos);
        let end = start + parsed.consumed();
        assert!(end > start, "embedded Code parser must make progress");
        let syntax = parsed.syntax();
        if header {
            for element in syntax.children_with_tokens() {
                match element {
                    NodeOrToken::Node(node) => copy(&mut self.state.builder, node, None),
                    NodeOrToken::Token(token) => {
                        self.state.builder.token(token.kind().into(), token.text())
                    }
                }
            }
        } else {
            copy(&mut self.state.builder, syntax, root_kind);
        }
        self.state
            .diagnostics
            .extend(parsed.diagnostics.into_iter().map(|mut d| {
                d.span += start;
                d
            }));
        self.state
            .contents
            .extend(parsed.contents.into_iter().map(|mut content| {
                content.range += start;
                content.body += start;
                for diagnostic in &mut content.diagnostics {
                    diagnostic.span += start;
                }
                content
            }));
        while self.state.lexed.offset(self.state.pos) < end {
            self.state.pos += 1;
        }
        assert_eq!(self.state.lexed.offset(self.state.pos), end);
    }
}

/// Parse native content without embedding its tree into the Code CST.
pub(crate) fn parse_content(source: &str, open: TextSize) -> ParsedContent {
    let tail = &source[usize::from(open)..];
    assert!(tail.starts_with('['));
    let mut parser = Parser::new(tail);
    let (end, closed) = match parser.balanced(0, SyntaxKind::LBracket, SyntaxKind::RBracket, true) {
        Balanced::Closed(close) => (close, true),
        Balanced::Unclosed(eof) => (eof, false),
        Balanced::BlockEnd => unreachable!(),
    };
    let body_end = parser.state.lexed.offset(end);
    let block = if closed {
        parser.is_block_body(0, end)
    } else {
        matches!(
            parser.kind_at(1),
            Some(SyntaxKind::Whitespace | SyntaxKind::Newline)
        )
    };
    if !block {
        parser.diagnose_inline_parbreak(0, end);
    }
    parser.state.pos = 1;
    parser.state.limit = Some(end);
    parser.in_body = true;
    parser.state.builder.start_node(SyntaxKind::Document.into());
    if block {
        parser.block_body();
    } else {
        parser.inline_delimited(&|_: &Parser<'_>| false, None);
    }
    while parser.cur().is_some() {
        parser.eat();
    }
    parser.state.builder.finish_node();
    let mut parsed = parser.state.finish();
    let frame = TextSize::new(1);
    for diagnostic in &mut parsed.diagnostics {
        diagnostic.span -= frame;
    }
    for content in &mut parsed.contents {
        content.range -= frame;
        content.body -= frame;
        for diagnostic in &mut content.diagnostics {
            diagnostic.span -= frame;
        }
    }
    let mut diagnostics = parsed
        .diagnostics
        .iter()
        .cloned()
        .map(|mut d| {
            d.span += open + frame;
            d
        })
        .collect::<Vec<_>>();
    if !closed {
        diagnostics.push(Diagnostic {
            span: TextRange::empty(open + body_end),
            message: "unclosed content literal".into(),
        });
    }
    ParsedContent {
        range: TextRange::new(open, open + body_end + TextSize::new(u32::from(closed))),
        body: TextRange::new(open + TextSize::new(1), open + body_end),
        flavor: if block {
            ContentFlavor::Block
        } else {
            ContentFlavor::Inline
        },
        closed,
        syntax: Box::new(parsed),
        diagnostics,
    }
}
