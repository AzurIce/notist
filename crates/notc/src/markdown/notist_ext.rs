use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

use rushdown::ast::{Arena, KindData, NodeKind, NodeRef, NodeType, PrettyPrint};
use rushdown::parser::{BlockParser, Context, InlineParser, State};
use rushdown::text::{BasicReader, BlockReader, Reader};

/// Opaque block-level Notist directive or invocation captured verbatim by `rushdown`
/// and lowered into frontend-agnostic `notc::ast` nodes.
#[derive(Debug)]
pub(crate) struct NotistBlock {
    pub kind: NotistBlockKind,
    pub text: RefCell<String>,
    pub start: usize,
    pub end: RefCell<usize>,
    pub complete: RefCell<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotistBlockKind {
    DocAttr,
    BlockAttr,
    Import,
    Call,
    AttributedCall,
}

impl NodeKind for NotistBlock {
    fn typ(&self) -> NodeType {
        NodeType::LeafBlock
    }
    fn kind_name(&self) -> &'static str {
        "NotistBlock"
    }
}

impl PrettyPrint for NotistBlock {
    fn pretty_print(&self, w: &mut dyn fmt::Write, _: &str, _: usize) -> fmt::Result {
        write!(w, "{:?}({:?})", self.kind, self.text.borrow())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NotistBlockParser {
    pub allow_bare_ident: bool,
    pub allow_top_level_directives: bool,
}

impl BlockParser for NotistBlockParser {
    fn trigger(&self) -> &[u8] {
        b"@#"
    }

    fn open(
        &self,
        arena: &mut Arena,
        _: NodeRef,
        reader: &mut BasicReader,
        ctx: &mut Context,
    ) -> Option<(NodeRef, State)> {
        let (line, segment) = reader.peek_line()?;
        let offset = ctx.block_offset()?;
        let raw = &line[offset..];
        let trimmed = raw.trim_end_matches(['\r', '\n']);
        let bytes = trimmed.as_bytes();
        let start = segment.start() + offset;

        // 1. Document attribute line: `@!(...)`
        if self.allow_top_level_directives && bytes.starts_with(b"@!(") {
            let close = find_balanced_delim(bytes, 2, b'(', b')')?;
            if !trimmed[close + 1..].trim().is_empty() {
                return None;
            }
            reader.advance_to_eol();
            return Some((
                make_block(arena, NotistBlockKind::DocAttr, trimmed, start, true),
                State::NO_CHILDREN,
            ));
        }

        // 2. Block attribute line: `@(...)` or `@(...)#call[...]`
        if bytes.starts_with(b"@(") {
            let close = find_balanced_delim(bytes, 1, b'(', b')')?;
            let after = trimmed[close + 1..].trim_start();
            if after.is_empty() {
                reader.advance_to_eol();
                return Some((
                    make_block(arena, NotistBlockKind::BlockAttr, trimmed, start, true),
                    State::NO_CHILDREN,
                ));
            }
            let hash_rel = close + 1 + (trimmed[close + 1..].len() - after.len());
            if bytes.get(hash_rel) == Some(&b'#')
                && is_hash_invocation(bytes, hash_rel, self.allow_bare_ident)
            {
                match scan_call_end(bytes, hash_rel, self.allow_bare_ident) {
                    CallScan::Complete(end) if trimmed[end..].trim().is_empty() => {
                        reader.advance_to_eol();
                        return Some((
                            make_block(
                                arena,
                                NotistBlockKind::AttributedCall,
                                trimmed,
                                start,
                                true,
                            ),
                            State::NO_CHILDREN,
                        ));
                    }
                    CallScan::Incomplete => {
                        reader.advance_to_eol();
                        return Some((
                            make_block(
                                arena,
                                NotistBlockKind::AttributedCall,
                                raw,
                                start,
                                false,
                            ),
                            State::NO_CHILDREN,
                        ));
                    }
                    _ => return None,
                }
            }
            return None;
        }

        // 3. Top-level `#import ...` directive
        if self.allow_top_level_directives && bytes.starts_with(b"#import") {
            let next = bytes.get(7).copied();
            if next.is_none() || matches!(next, Some(b' ' | b'\t' | b'"')) {
                reader.advance_to_eol();
                return Some((
                    make_block(arena, NotistBlockKind::Import, trimmed, start, true),
                    State::NO_CHILDREN,
                ));
            }
        }

        // 4. Standalone `#call(...)` / `#ns.call[...]` / `#(expr)` (or `#ident` in code literals)
        if bytes.first() == Some(&b'#') && is_hash_invocation(bytes, 0, self.allow_bare_ident) {
            match scan_call_end(bytes, 0, self.allow_bare_ident) {
                CallScan::Complete(end) if trimmed[end..].trim().is_empty() => {
                    reader.advance_to_eol();
                    return Some((
                        make_block(arena, NotistBlockKind::Call, trimmed, start, true),
                        State::NO_CHILDREN,
                    ));
                }
                CallScan::Incomplete => {
                    reader.advance_to_eol();
                    return Some((
                        make_block(arena, NotistBlockKind::Call, raw, start, false),
                        State::NO_CHILDREN,
                    ));
                }
                _ => return None,
            }
        }

        None
    }

    fn cont(
        &self,
        arena: &mut Arena,
        node: NodeRef,
        reader: &mut BasicReader,
        _: &mut Context,
    ) -> Option<State> {
        let KindData::Extension(data) = arena[node].kind_data() else {
            return None;
        };
        let block = data.as_any().downcast_ref::<NotistBlock>()?;
        if *block.complete.borrow() {
            return None;
        }
        let (line, segment) = reader.peek_line()?;
        let mut text = block.text.borrow_mut();
        text.push_str(&line);
        *block.end.borrow_mut() = segment.start() + line.trim_end_matches(['\r', '\n']).len();
        reader.advance_to_eol();

        let bytes = text.as_bytes();
        let hash_pos = if block.kind == NotistBlockKind::AttributedCall {
            let close = find_balanced_delim(bytes, 1, b'(', b')').unwrap_or(0);
            let mut pos = close + 1;
            while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t') {
                pos += 1;
            }
            pos
        } else {
            0
        };
        if matches!(
            scan_call_end(bytes, hash_pos, self.allow_bare_ident),
            CallScan::Complete(_)
        ) {
            *block.complete.borrow_mut() = true;
        }
        Some(State::NO_CHILDREN)
    }

    fn can_interrupt_paragraph(&self) -> bool {
        true
    }
}

fn make_block(
    arena: &mut Arena,
    kind: NotistBlockKind,
    text: &str,
    start: usize,
    complete: bool,
) -> NodeRef {
    let end = start + text.trim_end_matches(['\r', '\n']).len();
    arena.new_node(KindData::Extension(Box::new(NotistBlock {
        kind,
        text: RefCell::new(text.to_owned()),
        start,
        end: RefCell::new(end),
        complete: RefCell::new(complete),
    })))
}

/// Inline Notist attribute annotation `@(...)` or `#call(...)` / `#(expr)` / `#ident` interpolation.
#[derive(Debug)]
pub(crate) enum NotistInline {
    Attr {
        text: String,
        start: usize,
    },
    Call {
        text: String,
        start: usize,
        end: usize,
    },
}

impl NodeKind for NotistInline {
    fn typ(&self) -> NodeType {
        NodeType::Inline
    }
    fn kind_name(&self) -> &'static str {
        "NotistInline"
    }
}

impl PrettyPrint for NotistInline {
    fn pretty_print(&self, w: &mut dyn fmt::Write, _: &str, _: usize) -> fmt::Result {
        write!(w, "{self:?}")
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NotistInlineParser {
    pub source: Arc<str>,
    pub allow_bare_ident: bool,
}

impl InlineParser for NotistInlineParser {
    fn trigger(&self) -> &[u8] {
        b"@#"
    }

    fn parse(
        &self,
        arena: &mut Arena,
        _: NodeRef,
        reader: &mut BlockReader,
        _: &mut Context,
    ) -> Option<NodeRef> {
        let (line, segment) = reader.peek_line()?;
        let bytes = line.as_bytes();
        let start = segment.start();

        // Inline attribute annotation: `@(key: value, ...)`
        if bytes.starts_with(b"@(") {
            let close = find_balanced_delim(bytes, 1, b'(', b')')?;
            let consumed = close + 1;
            let raw = line[..consumed].to_owned();
            reader.advance(consumed);
            return Some(arena.new_node(KindData::Extension(Box::new(
                NotistInline::Attr { text: raw, start },
            ))));
        }

        // Inline `#call(...)` / `#ns.call[...]` / `#(expr)` (or `#ident` in code literals)
        if bytes.first() == Some(&b'#') {
            if !self.allow_bare_ident
                && start > 0
                && is_url_or_ident_byte(self.source.as_bytes()[start - 1])
            {
                return None;
            }
            if !is_hash_invocation(bytes, 0, self.allow_bare_ident) {
                return None;
            }
            let CallScan::Complete(end_rel) = scan_call_end(bytes, 0, self.allow_bare_ident) else {
                return None;
            };
            let raw = line[..end_rel].to_owned();
            reader.advance(end_rel);
            return Some(arena.new_node(KindData::Extension(Box::new(
                NotistInline::Call {
                    text: raw,
                    start,
                    end: start + end_rel,
                },
            ))));
        }

        None
    }
}

pub(crate) fn is_url_or_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(b, b'/' | b'_' | b'-' | b'.' | b':' | b'?' | b'&' | b'=')
}

/// Determine whether `#...` at `hash_pos` begins a Notist interpolation/invocation.
/// - `#(expr)` is always accepted.
/// - When `allow_bare_ident` is `true` (inside `.not` `[...]` content literals),
///   any `#ident...` is accepted (including bare `#x` or `#a.b`).
/// - When `allow_bare_ident` is `false` (top-level `.md` documents), `#ident(.ident)*`
///   must be followed immediately by `(`, `[`, or `<...>` + `(`/`[` so prose `#tags` stay text.
pub(crate) fn is_hash_invocation(bytes: &[u8], hash_pos: usize, allow_bare_ident: bool) -> bool {
    let len = bytes.len();
    let mut j = hash_pos + 1;
    if j >= len {
        return false;
    }
    if bytes[j] == b'(' {
        return true;
    }
    if !(bytes[j].is_ascii_alphabetic() || bytes[j] == b'_') {
        return false;
    }
    if allow_bare_ident {
        return true;
    }
    while j < len && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
        j += 1;
    }
    while j < len && bytes[j] == b'.' {
        j += 1;
        if j >= len || !(bytes[j].is_ascii_alphabetic() || bytes[j] == b'_') {
            return false;
        }
        while j < len && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
    }
    if j < len && bytes[j] == b'<'
        && let Some(close) = find_balanced_delim(bytes, j, b'<', b'>')
    {
        j = close + 1;
    }
    j < len && matches!(bytes[j], b'(' | b'[')
}

pub(crate) enum CallScan {
    Complete(usize),
    Incomplete,
    Invalid,
}

/// Scan a `#(expr)` or `#ident(.ident)*` postfix chain (including chained `(...)`,
/// `[...]`, `<...>`, and `.method(...)`) to determine whether delimiters are complete.
pub(crate) fn scan_call_end(bytes: &[u8], hash_pos: usize, allow_bare_ident: bool) -> CallScan {
    let len = bytes.len();
    let mut i = hash_pos + 1;
    if i >= len {
        return CallScan::Invalid;
    }
    if bytes[i] == b'(' {
        return match find_balanced_delim(bytes, i, b'(', b')') {
            Some(close) => CallScan::Complete(close + 1),
            None => CallScan::Incomplete,
        };
    }
    if !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        return CallScan::Invalid;
    }
    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    let mut had_call = allow_bare_ident;
    loop {
        if i < len && bytes[i] == b'.' {
            let next = i + 1;
            if next < len && (bytes[next].is_ascii_alphabetic() || bytes[next] == b'_') {
                i = next + 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                continue;
            }
            break;
        }
        if i < len && bytes[i] == b'<' {
            if let Some(close) = find_balanced_delim(bytes, i, b'<', b'>')
                && bytes.get(close + 1).is_some_and(|b| matches!(b, b'(' | b'['))
            {
                i = close + 1;
                continue;
            }
            break;
        }
        if i < len && bytes[i] == b'(' {
            had_call = true;
            match find_balanced_delim(bytes, i, b'(', b')') {
                Some(close) => i = close + 1,
                None => return CallScan::Incomplete,
            }
            continue;
        }
        if i < len && bytes[i] == b'[' {
            had_call = true;
            match find_balanced_delim(bytes, i, b'[', b']') {
                Some(close) => i = close + 1,
                None => return CallScan::Incomplete,
            }
            continue;
        }
        break;
    }
    if had_call {
        CallScan::Complete(i)
    } else {
        CallScan::Invalid
    }
}

pub(crate) fn find_balanced_delim(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    if bytes.get(start) != Some(&open) {
        return None;
    }
    let mut i = start + 1;
    let mut depth = 1usize;
    let mut in_string = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'\\' if open == b'[' && i + 1 < bytes.len() => {
                i += 2;
            }
            b'"' if open == b'(' => {
                in_string = true;
                i += 1;
            }
            b if b == open => {
                depth += 1;
                i += 1;
            }
            b if b == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}
