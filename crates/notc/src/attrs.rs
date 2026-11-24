//! Frontend-agnostic attribute annotation parser (`@(...)`, `@!(...)`) and
//! unified AST-level attribute attachment pass over `BlockPiece` / `InlinePiece`.

use crate::ast::{AttrEntry, AttrLiteral, BlockPiece, InlinePiece, Span};
use crate::lexer::ParseDiagnostic;

/// Frontend-agnostic AST pass that attaches `BlockPiece::Attr` and `InlinePiece::Attr`
/// placeholder nodes to the immediately following AST node in the same sequence.
pub(crate) fn attach_block_attrs(
    blocks: Vec<BlockPiece>,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Vec<BlockPiece> {
    let mut out = Vec::with_capacity(blocks.len());
    let mut pending_attrs: Vec<AttrEntry> = Vec::new();
    let mut pending_span: Option<Span> = None;

    for block in blocks {
        match block {
            BlockPiece::Attr { attrs, span } => {
                pending_attrs.extend(attrs);
                pending_span = Some(match pending_span {
                    Some(prev) => prev.merge(span),
                    None => span,
                });
            }
            mut piece => {
                // First recursively resolve nested sequences inside this block.
                piece = match piece {
                    BlockPiece::Paragraph {
                        inlines,
                        attrs,
                        span,
                    } => BlockPiece::Paragraph {
                        inlines: attach_inline_attrs(inlines, diagnostics),
                        attrs,
                        span,
                    },
                    BlockPiece::Heading {
                        level,
                        title,
                        attrs,
                        span,
                    } => BlockPiece::Heading {
                        level,
                        title: attach_inline_attrs(title, diagnostics),
                        attrs,
                        span,
                    },
                    BlockPiece::List {
                        ordered,
                        start,
                        items,
                        attrs,
                        span,
                    } => BlockPiece::List {
                        ordered,
                        start,
                        items: items
                            .into_iter()
                            .map(|item| attach_block_attrs(item, diagnostics))
                            .collect(),
                        attrs,
                        span,
                    },
                    BlockPiece::Quote { body, attrs, span } => BlockPiece::Quote {
                        body: attach_block_attrs(body, diagnostics),
                        attrs,
                        span,
                    },
                    BlockPiece::Table {
                        headers,
                        alignments,
                        rows,
                        attrs,
                        span,
                    } => BlockPiece::Table {
                        headers: headers
                            .into_iter()
                            .map(|cell| attach_inline_attrs(cell, diagnostics))
                            .collect(),
                        alignments,
                        rows: rows
                            .into_iter()
                            .map(|row| {
                                row.into_iter()
                                    .map(|cell| attach_inline_attrs(cell, diagnostics))
                                    .collect()
                            })
                            .collect(),
                        attrs,
                        span,
                    },
                    other => other,
                };

                if !pending_attrs.is_empty() {
                    let mut combined = std::mem::take(&mut pending_attrs);
                    pending_span = None;
                    match &mut piece {
                        BlockPiece::Paragraph { attrs, .. }
                        | BlockPiece::Heading { attrs, .. }
                        | BlockPiece::CodeBlock { attrs, .. }
                        | BlockPiece::List { attrs, .. }
                        | BlockPiece::Quote { attrs, .. }
                        | BlockPiece::Divider { attrs, .. }
                        | BlockPiece::Table { attrs, .. }
                        | BlockPiece::Math { attrs, .. }
                        | BlockPiece::Expr { attrs, .. } => {
                            combined.append(attrs);
                            *attrs = combined;
                        }
                        BlockPiece::Attr { .. } => unreachable!(),
                    }
                }
                out.push(piece);
            }
        }
    }

    if let Some(span) = pending_span {
        diagnostics.push(ParseDiagnostic {
            span,
            message: "attribute annotation has no following content element".into(),
        });
    }

    out
}

pub(crate) fn attach_inline_attrs(
    inlines: Vec<InlinePiece>,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Vec<InlinePiece> {
    // Coalesce adjacent plain `Text` nodes (e.g. across soft line breaks) before
    // attaching inline `@(...)` annotations so `@(k: v)foo\nbar` attaches cleanly
    // to the whole continuous text node.
    let mut merged: Vec<InlinePiece> = Vec::with_capacity(inlines.len());
    for piece in inlines {
        if let InlinePiece::Text(ref next_text, next_span) = piece
            && let Some(InlinePiece::Text(prev_text, prev_span)) = merged.last_mut()
        {
            prev_text.push_str(next_text);
            *prev_span = prev_span.merge(next_span);
            continue;
        }
        merged.push(piece);
    }

    let mut out = Vec::with_capacity(merged.len());
    let mut pending_attrs: Vec<AttrEntry> = Vec::new();
    let mut pending_span: Option<Span> = None;

    let mut iter = merged.into_iter().peekable();
    while let Some(piece) = iter.next() {
        match piece {
            InlinePiece::Attr { attrs, span } => {
                pending_attrs.extend(attrs);
                pending_span = Some(match pending_span {
                    Some(prev) => prev.merge(span),
                    None => span,
                });
            }
            mut node => {
                // Skip pure-whitespace text separator between `@(...)` and a following element
                // (e.g. `@(collapsed: true) [A](a.md)`), keeping the space in output.
                if !pending_attrs.is_empty()
                    && let InlinePiece::Text(ref s, _) = node
                    && s.chars().all(char::is_whitespace)
                    && iter.peek().is_some()
                {
                    out.push(node);
                    continue;
                }

                // Recursively resolve nested inline sequences.
                node = match node {
                    InlinePiece::Strong(inner, span) => {
                        InlinePiece::Strong(attach_inline_attrs(inner, diagnostics), span)
                    }
                    InlinePiece::Emph(inner, span) => {
                        InlinePiece::Emph(attach_inline_attrs(inner, diagnostics), span)
                    }
                    InlinePiece::Strike(inner, span) => {
                        InlinePiece::Strike(attach_inline_attrs(inner, diagnostics), span)
                    }
                    InlinePiece::Link {
                        target,
                        label,
                        span,
                    } => InlinePiece::Link {
                        target,
                        label: attach_inline_attrs(label, diagnostics),
                        span,
                    },
                    InlinePiece::Attributed {
                        attrs,
                        target,
                        span,
                    } => {
                        let resolved = attach_inline_attrs(vec![*target], diagnostics);
                        let target = resolved.into_iter().next().unwrap_or_else(|| {
                            InlinePiece::Text(String::new(), span)
                        });
                        InlinePiece::Attributed {
                            attrs,
                            target: Box::new(target),
                            span,
                        }
                    }
                    other => other,
                };

                if !pending_attrs.is_empty() {
                    let attr_span = pending_span.take().unwrap_or(Span::DUMMY);
                    let mut combined = std::mem::take(&mut pending_attrs);
                    let node_span = inline_piece_span(&node);
                    node = match node {
                        InlinePiece::Attributed {
                            mut attrs,
                            target,
                            span,
                        } => {
                            combined.append(&mut attrs);
                            InlinePiece::Attributed {
                                attrs: combined,
                                target,
                                span: attr_span.merge(span),
                            }
                        }
                        other => InlinePiece::Attributed {
                            attrs: combined,
                            target: Box::new(other),
                            span: attr_span.merge(node_span),
                        },
                    };
                }
                out.push(node);
            }
        }
    }

    if let Some(span) = pending_span {
        diagnostics.push(ParseDiagnostic {
            span,
            message: "attribute annotation has no following content element".into(),
        });
    }

    out
}

fn inline_piece_span(piece: &InlinePiece) -> Span {
    match piece {
        InlinePiece::Attr { span, .. }
        | InlinePiece::Text(_, span)
        | InlinePiece::Strong(_, span)
        | InlinePiece::Emph(_, span)
        | InlinePiece::Strike(_, span)
        | InlinePiece::Raw(_, span)
        | InlinePiece::Link { span, .. }
        | InlinePiece::Image { span, .. }
        | InlinePiece::Math(_, span)
        | InlinePiece::Expr(_, span)
        | InlinePiece::Attributed { span, .. } => *span,
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn mk_span(base: usize, start: usize, end: usize) -> Span {
    Span::new((base + start) as u32, (base + end) as u32)
}

pub(crate) fn parse_attr_list_at(
    text: &str,
    at_pos: usize,
    open_paren_pos: usize,
    base: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<(Vec<AttrEntry>, usize)> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    if open_paren_pos >= len || bytes[open_paren_pos] != b'(' {
        return None;
    }
    let mut i = open_paren_pos + 1;
    let mut entries = Vec::new();

    loop {
        while i < len && is_ws(bytes[i]) {
            i += 1;
        }
        if i >= len {
            diagnostics.push(ParseDiagnostic {
                span: mk_span(base, at_pos, i),
                message: "unterminated attribute annotation".into(),
            });
            return Some((entries, i));
        }
        if bytes[i] == b')' {
            return Some((entries, i + 1));
        }

        let key_start = i;
        if !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
            diagnostics.push(ParseDiagnostic {
                span: mk_span(base, i, i + 1),
                message: "expected attribute key identifier".into(),
            });
            return None;
        }
        while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
        {
            i += 1;
        }
        let key = text[key_start..i].to_owned();

        while i < len && is_ws(bytes[i]) {
            i += 1;
        }
        if i >= len || bytes[i] != b':' {
            diagnostics.push(ParseDiagnostic {
                span: mk_span(base, key_start, i),
                message: format!("expected `:` after attribute key `{key}`"),
            });
            return None;
        }
        i += 1;
        while i < len && is_ws(bytes[i]) {
            i += 1;
        }

        let (value, next_i) = parse_attr_literal(text, i, base, diagnostics)?;
        i = next_i;

        entries.push(AttrEntry {
            key,
            value,
            span: mk_span(base, key_start, i),
        });

        while i < len && is_ws(bytes[i]) {
            i += 1;
        }
        if i < len && bytes[i] == b',' {
            i += 1;
        } else if i < len && bytes[i] == b')' {
            return Some((entries, i + 1));
        } else {
            diagnostics.push(ParseDiagnostic {
                span: mk_span(base, i, (i + 1).min(len)),
                message: "expected `,` or `)` in attribute annotation".into(),
            });
            return None;
        }
    }
}

fn parse_attr_literal(
    text: &str,
    start: usize,
    base: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<(AttrLiteral, usize)> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = start;
    while i < len && is_ws(bytes[i]) {
        i += 1;
    }
    let val_start = i;
    if i < len && bytes[i] == b'(' {
        i += 1;
        while i < len && is_ws(bytes[i]) {
            i += 1;
        }
        if i < len && bytes[i] == b')' {
            return Some((AttrLiteral::List(Vec::new()), i + 1));
        }
        let (first, next_i) = parse_attr_literal(text, i, base, diagnostics)?;
        i = next_i;
        while i < len && is_ws(bytes[i]) {
            i += 1;
        }
        if i < len && bytes[i] == b',' {
            i += 1;
            let mut items = vec![first];
            loop {
                while i < len && is_ws(bytes[i]) {
                    i += 1;
                }
                if i < len && bytes[i] == b')' {
                    return Some((AttrLiteral::List(items), i + 1));
                }
                if i >= len {
                    diagnostics.push(ParseDiagnostic {
                        span: mk_span(base, val_start, i),
                        message: "unterminated attribute list".into(),
                    });
                    return None;
                }
                let (item, item_end) = parse_attr_literal(text, i, base, diagnostics)?;
                items.push(item);
                i = item_end;
                while i < len && is_ws(bytes[i]) {
                    i += 1;
                }
                if i < len && bytes[i] == b',' {
                    i += 1;
                } else if i < len && bytes[i] == b')' {
                    return Some((AttrLiteral::List(items), i + 1));
                } else {
                    diagnostics.push(ParseDiagnostic {
                        span: mk_span(base, i, (i + 1).min(len)),
                        message: "expected `,` or `)` in attribute list".into(),
                    });
                    return None;
                }
            }
        } else if i < len && bytes[i] == b')' {
            Some((first, i + 1))
        } else {
            diagnostics.push(ParseDiagnostic {
                span: mk_span(base, val_start, i.min(len)),
                message: "expected `,` or `)` in attribute list".into(),
            });
            None
        }
    } else if i < len && bytes[i] == b'"' {
        i += 1;
        let mut s = String::new();
        let mut closed = false;
        while i < len {
            if bytes[i] == b'\\' && i + 1 < len {
                i += 1;
                match bytes[i] {
                    b'"' => s.push('"'),
                    b'\\' => s.push('\\'),
                    b'n' => s.push('\n'),
                    b't' => s.push('\t'),
                    b'r' => s.push('\r'),
                    other => s.push(other as char),
                }
                i += 1;
            } else if bytes[i] == b'"' {
                i += 1;
                closed = true;
                break;
            } else {
                let ch = text[i..].chars().next().expect("valid utf-8");
                s.push(ch);
                i += ch.len_utf8();
            }
        }
        if !closed {
            diagnostics.push(ParseDiagnostic {
                span: mk_span(base, val_start, i),
                message: "unterminated attribute string".into(),
            });
            return None;
        }
        Some((AttrLiteral::String(s), i))
    } else if text[i..].starts_with("true")
        && !bytes
            .get(i + 4)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        Some((AttrLiteral::Bool(true), i + 4))
    } else if text[i..].starts_with("false")
        && !bytes
            .get(i + 5)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        Some((AttrLiteral::Bool(false), i + 5))
    } else if i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'-') {
        if bytes[i] == b'-' {
            i += 1;
        }
        let mut is_float = false;
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i + 1 < len && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
            is_float = true;
            i += 1;
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
        let num = &text[val_start..i];
        if is_float {
            let v = num.parse::<f64>().ok()?;
            Some((AttrLiteral::Float(v), i))
        } else {
            let v = num.parse::<i64>().ok()?;
            Some((AttrLiteral::Int(v), i))
        }
    } else {
        diagnostics.push(ParseDiagnostic {
            span: mk_span(base, val_start, (val_start + 1).min(len)),
            message: "expected literal attribute value".into(),
        });
        None
    }
}
