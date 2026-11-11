//! Lazy, source-backed token tape.
//!
//! The parser owns the lexical mode: a token exists only once the cursor reaches
//! it, and it is produced by a scanner that never consults the grammar. Mode
//! switches happen at a committed token boundary, so lookahead is always
//! homogeneous and is discarded on a switch. Independent speculative scans open
//! their own tape over the same source.
//!
//! Accessors take `&self` so grammar predicates can look ahead freely. The tape
//! grows lazily behind a cell; every accessor returns a copy or a slice of the
//! immutable source, never a reference into the growable vector, so no borrow
//! can span a grow.
use std::cell::UnsafeCell;

use crate::tree::SyntaxKind;
use rowan::TextSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Native Markup body text.
    Markup,
    /// Shared Code literal tokens (call arguments, annotation payloads).
    Literal,
    /// Code declaration tokens (keywords reserved).
    Module,
}

/// Token storage. Text is always borrowed from the original source.
pub struct Lexed<'a> {
    src: &'a str,
    tape: UnsafeCell<Tape>,
}

struct Tape {
    mode: Mode,
    kinds: Vec<SyntaxKind>,
    /// `starts[i]` is the offset of token `i`; the last entry is the next
    /// unscanned offset.
    starts: Vec<TextSize>,
    /// Tab-expanded column of `starts[i]`, counted from the line start.
    columns: Vec<u32>,
    /// Whether the source between the previous line break and `starts[i]`
    /// holds only spaces and tabs.
    at_line_start: Vec<bool>,
    next: usize,
}

impl<'a> Lexed<'a> {
    /// A tape over the whole source, positioned at `start` (a char boundary).
    /// Reserves room for the expected token count.
    pub fn new(src: &'a str, start: usize, mode: Mode) -> Self {
        Self::sized(src, start, mode, true)
    }

    /// A tape for a bounded scan: tokens are appended as they are reached, so
    /// the source may extend past the region this tape is used for.
    pub fn bounded(src: &'a str, start: usize, mode: Mode) -> Self {
        Self::sized(src, start, mode, false)
    }

    fn sized(src: &'a str, start: usize, mode: Mode, reserve: bool) -> Self {
        // One slot per two remaining bytes is the typical density: reserving it
        // avoids repeated growth on a whole-document tape, while a bounded scan
        // only touches its own region and should not pay for the rest.
        let capacity = if reserve { (src.len() - start) / 2 + 1 } else { 16 };
        Self {
            src,
            tape: UnsafeCell::new(Tape {
                mode,
                // Most tokens cover two or more bytes; reserving one slot per
                // two remaining bytes avoids repeated reallocation without a
                // large over-allocation on prose-heavy sources.
                kinds: Vec::with_capacity(capacity),
                starts: {
                    let mut starts = Vec::with_capacity(capacity + 1);
                    starts.push(TextSize::new(start as u32));
                    starts
                },
                columns: {
                    // Positions before the tape's start are not represented, so
                    // the first token is treated as beginning a line. Callers
                    // that need a true column start the tape at a line start.
                    let mut columns = Vec::with_capacity(capacity + 1);
                    columns.push(0);
                    columns
                },
                at_line_start: {
                    let mut at_line_start = Vec::with_capacity(capacity + 1);
                    at_line_start.push(true);
                    at_line_start
                },
                next: start,
            }),
        }
    }

    pub fn source(&self) -> &'a str {
        self.src
    }

    #[inline]
    fn tape(&self) -> &mut Tape {
        // SAFETY: every accessor reads or appends to the tape and returns either
        // a copy or a slice of `self.src`, which the tape never reallocates.
        // No `&mut Tape` survives an accessor call, so no two borrows overlap.
        unsafe { &mut *self.tape.get() }
    }

    #[inline]
    fn scan(&self, i: usize) {
        let tape = self.tape();
        while tape.kinds.len() <= i && tape.next < self.src.len() {
            let offset = tape.next;
            let (kind, len) = match tape.mode {
                Mode::Markup => crate::markup::lexer::token(self.src, offset),
                Mode::Literal => crate::code::lexer::literal_token(self.src, offset),
                Mode::Module => crate::code::lexer::module_token(self.src, offset),
            };
            debug_assert!(len > 0 && self.src.is_char_boundary(offset + len));
            let text = &self.src[offset..offset + len];
            tape.kinds.push(kind);
            tape.next = offset + len;
            tape.starts.push(TextSize::new(tape.next as u32));
            let (column, at_line_start) = advance(
                tape.columns[tape.columns.len() - 1],
                tape.at_line_start[tape.at_line_start.len() - 1],
                text,
            );
            tape.columns.push(column);
            tape.at_line_start.push(at_line_start);
        }
    }

    /// Tab-expanded column of token `i` within its line.
    pub fn column(&self, i: usize) -> usize {
        self.scan(i);
        self.tape().columns.get(i).copied().unwrap_or(0) as usize
    }

    /// Whether everything between the last line break and token `i` is
    /// indentation.
    pub fn at_line_start(&self, i: usize) -> bool {
        self.scan(i);
        self.tape().at_line_start.get(i).copied().unwrap_or(true)
    }

    /// Kind and text of token `i`.
    #[inline]
    pub fn peek(&self, i: usize) -> Option<(SyntaxKind, &'a str)> {
        self.scan(i);
        let tape = self.tape();
        let kind = *tape.kinds.get(i)?;
        let start = tape.starts[i];
        let end = tape
            .starts
            .get(i + 1)
            .copied()
            .unwrap_or_else(|| TextSize::new(self.src.len() as u32));
        Some((kind, &self.src[usize::from(start)..usize::from(end)]))
    }

    pub fn kind(&self, i: usize) -> Option<SyntaxKind> {
        self.scan(i);
        self.tape().kinds.get(i).copied()
    }

    /// Byte offset of token `i`; the source length past the end.
    pub fn offset(&self, i: usize) -> TextSize {
        self.scan(i);
        self.tape()
            .starts
            .get(i)
            .copied()
            .unwrap_or_else(|| TextSize::new(self.src.len() as u32))
    }

    pub fn text(&self, i: usize) -> &'a str {
        self.scan(i);
        let tape = self.tape();
        let start = tape.starts[i];
        let end = tape
            .starts
            .get(i + 1)
            .copied()
            .unwrap_or_else(|| TextSize::new(self.src.len() as u32));
        &self.src[usize::from(start)..usize::from(end)]
    }

    pub fn len(&self, i: usize) -> usize {
        self.text(i).len()
    }

    /// The tape's current lexical mode.
    pub fn mode(&self) -> Mode {
        self.tape().mode
    }

    /// Drop every token from index `pos` on and continue in `mode` from `byte`,
    /// which must be the start of token `pos`.
    pub fn seek(&self, pos: usize, byte: TextSize, mode: Mode) {
        let tape = self.tape();
        debug_assert_eq!(tape.starts.get(pos), Some(&byte));
        tape.kinds.truncate(pos);
        tape.starts.truncate(pos + 1);
        tape.columns.truncate(pos + 1);
        tape.at_line_start.truncate(pos + 1);
        tape.next = usize::from(byte);
        tape.mode = mode;
    }

    /// Continue at `end`: token `pos` has been consumed by another grammar and
    /// the next token begins exactly at `end`.
    pub fn resume(&self, pos: usize, end: TextSize, mode: Mode) {
        let tape = self.tape();
        let from = usize::from(tape.starts[pos]);
        let to = usize::from(end);
        tape.kinds.truncate(pos);
        tape.starts.truncate(pos + 1);
        tape.columns.truncate(pos + 1);
        tape.at_line_start.truncate(pos + 1);
        tape.starts[pos] = end;
        let (column, at_line_start) = advance(
            tape.columns[pos],
            tape.at_line_start[pos],
            &self.src[from.min(to)..to],
        );
        tape.columns[pos] = column;
        tape.at_line_start[pos] = at_line_start;
        tape.next = to;
        tape.mode = mode;
    }
}

/// The column and line-start status after `text`, given the status before it.
/// A line break restarts both; tabs advance to the next four-column boundary.
fn advance(column: u32, at_line_start: bool, text: &str) -> (u32, bool) {
    let mut column = column;
    let mut at_line_start = at_line_start;
    for ch in text.chars() {
        match ch {
            '\n' | '\r' => {
                column = 0;
                at_line_start = true;
            }
            '\t' => {
                column = (column / 4 + 1) * 4;
                at_line_start = false;
            }
            ' ' => column += 1,
            _ => {
                column += 1;
                at_line_start = false;
            }
        }
    }
    (column, at_line_start)
}
