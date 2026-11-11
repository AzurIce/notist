//! Lazy, source-backed token tape.
//!
//! The parser owns the lexical mode: a token exists only once the cursor reaches
//! it, and it is produced by a scanner that never consults the grammar. Mode
//! switches happen at a committed token boundary, so lookahead is always
//! homogeneous and is discarded on a switch. Independent speculative scans open
//! their own tape over the same source.
use std::cell::RefCell;

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

/// Token storage with interior mutability so predicates can look ahead through
/// a shared borrow. Text is always borrowed from the original source.
pub struct Lexed<'a> {
    src: &'a str,
    tape: RefCell<Tape<'a>>,
}

struct Tape<'a> {
    mode: Mode,
    kinds: Vec<SyntaxKind>,
    /// `starts[i]` is the offset of token `i`; the last entry is the next
    /// unscanned offset.
    starts: Vec<TextSize>,
    next: usize,
    _marker: std::marker::PhantomData<&'a str>,
}

impl<'a> Lexed<'a> {
    /// A tape positioned at `start`, which must be a char boundary.
    pub fn new(src: &'a str, start: usize, mode: Mode) -> Self {
        Self {
            src,
            tape: RefCell::new(Tape {
                mode,
                kinds: Vec::new(),
                starts: vec![TextSize::new(start as u32)],
                next: start,
                _marker: std::marker::PhantomData,
            }),
        }
    }

    pub fn source(&self) -> &'a str {
        self.src
    }

    fn scan(tape: &mut Tape<'a>, src: &'a str, i: usize) {
        while tape.kinds.len() <= i && tape.next < src.len() {
            let offset = tape.next;
            let (kind, len) = match tape.mode {
                Mode::Markup => crate::markup::lexer::token(src, offset),
                Mode::Literal => crate::code::lexer::literal_token(src, offset),
                Mode::Module => crate::code::lexer::module_token(src, offset),
            };
            debug_assert!(len > 0 && src.is_char_boundary(offset + len));
            tape.kinds.push(kind);
            tape.next = offset + len;
            tape.starts.push(TextSize::new(tape.next as u32));
        }
    }

    pub fn kind(&self, i: usize) -> Option<SyntaxKind> {
        let mut tape = self.tape.borrow_mut();
        Self::scan(&mut tape, self.src, i);
        tape.kinds.get(i).copied()
    }

    /// Byte offset of token `i`; the source length past the end.
    pub fn offset(&self, i: usize) -> TextSize {
        let mut tape = self.tape.borrow_mut();
        Self::scan(&mut tape, self.src, i);
        tape.starts
            .get(i)
            .copied()
            .unwrap_or_else(|| TextSize::new(self.src.len() as u32))
    }

    pub fn text(&self, i: usize) -> &'a str {
        let mut tape = self.tape.borrow_mut();
        Self::scan(&mut tape, self.src, i);
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
        self.tape.borrow().mode
    }

    /// Drop every token from index `pos` on and continue in `mode` from `byte`,
    /// which must be the start of token `pos`.
    pub fn seek(&self, pos: usize, byte: TextSize, mode: Mode) {
        let mut tape = self.tape.borrow_mut();
        debug_assert_eq!(tape.starts.get(pos), Some(&byte));
        tape.kinds.truncate(pos);
        tape.starts.truncate(pos + 1);
        tape.next = usize::from(byte);
        tape.mode = mode;
    }

    /// Continue at `end`: token `pos` has been consumed by another grammar and
    /// the next token begins exactly at `end`. Unlike [`Self::seek`], the token
    /// that started at the cursor is replaced by the resumed range.
    pub fn resume(&self, pos: usize, end: TextSize, mode: Mode) {
        let mut tape = self.tape.borrow_mut();
        tape.kinds.truncate(pos);
        tape.starts.truncate(pos + 1);
        tape.starts[pos] = end;
        tape.next = usize::from(end);
        tape.mode = mode;
    }
}
