//! Markup parser for `[...]` content literals embedded inside `.notc`.
//!
//! Uses Typst-style flanking whitespace to determine flavor purely lexically:
//! - `[hello]` -> `InlineLiteral`
//! - `[ hello ]` or multi-line `[\n...\n]` -> `BlockLiteral`

use crate::ast::{AttrEntry, AttrLiteral, BlockPiece, Expr, ExprKind, InlinePiece, Span};
use crate::lexer::ParseDiagnostic;
use crate::parser::{parse_embedded_expr, parse_embedded_postfix};

pub(crate) fn parse_content_literal(
    raw: &str,
    literal_span: Span,
    depth: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Expr {
    let inner = &raw[1..raw.len() - 1];
    let base = literal_span.start as usize + 1;

    let is_block = inner.bytes().next().is_some_and(is_ws)
        && inner.bytes().last().is_some_and(is_ws);

    if is_block {
        let blocks = parse_blocks(inner, base, depth, diagnostics);
        Expr::new(ExprKind::BlockLiteral(blocks), literal_span)
    } else {
        let inlines = parse_inlines(inner, base, depth, diagnostics);
        Expr::new(ExprKind::InlineLiteral(inlines), literal_span)
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn mk_span(base: usize, start: usize, end: usize) -> Span {
    Span::new((base + start) as u32, (base + end) as u32)
}

fn parse_blocks(
    text: &str,
    base: usize,
    depth: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Vec<BlockPiece> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut blocks = Vec::new();
    let mut pending_attrs: Vec<AttrEntry> = Vec::new();

    while i < len {
        // Skip blank lines and leading whitespace before next block.
        let line_start = i;
        while i < len && matches!(bytes[i], b' ' | b'\t' | b'\r') {
            i += 1;
        }
        if i >= len {
            break;
        }
        if bytes[i] == b'\n' {
            i += 1;
            continue;
        }

        // 1. Block attribute line: `@(key: value, ...)`
        if bytes[i] == b'@'
            && i + 1 < len
            && bytes[i + 1] == b'('
            && let Some((attrs, next_i)) = parse_attr_list(text, i, base, diagnostics)
        {
            let mut rest = next_i;
            while rest < len && matches!(bytes[rest], b' ' | b'\t' | b'\r') {
                rest += 1;
            }
            if rest >= len || bytes[rest] == b'\n' {
                pending_attrs.extend(attrs);
                i = if rest < len { rest + 1 } else { rest };
                continue;
            }
        }

        // 2. Heading line: `=`..`======` followed by space or tab
        if bytes[i] == b'=' {
            let mut eqs = 0usize;
            while i + eqs < len && bytes[i + eqs] == b'=' {
                eqs += 1;
            }
            if (1..=6).contains(&eqs)
                && i + eqs < len
                && matches!(bytes[i + eqs], b' ' | b'\t')
            {
                let h_start = i;
                let mut title_start = i + eqs;
                while title_start < len && matches!(bytes[title_start], b' ' | b'\t') {
                    title_start += 1;
                }
                let line_end = find_line_end(bytes, title_start);
                let title_raw = text[title_start..line_end].trim_end();
                let title = parse_inlines(title_raw, base + title_start, depth, diagnostics);
                let attrs = std::mem::take(&mut pending_attrs);
                blocks.push(BlockPiece::Heading {
                    level: eqs as u8,
                    title,
                    attrs,
                    span: mk_span(base, h_start, line_end),
                });
                i = next_line(bytes, line_end);
                continue;
            }
        }

        // 3. Fenced code block: ```lang\n...\n```
        if i + 2 < len && bytes[i] == b'`' && bytes[i + 1] == b'`' && bytes[i + 2] == b'`' {
            let cb_start = i;
            let fence_indent = &text[line_start..i];
            let info_start = i + 3;
            let info_end = find_line_end(bytes, info_start);
            let lang = text[info_start..info_end].trim().to_owned();
            let mut cursor = next_line(bytes, info_end);
            let body_start = cursor;
            let mut body_end = len;
            let mut closed_end = len;
            while cursor < len {
                let l_start = cursor;
                let mut trimmed = cursor;
                while trimmed < len && matches!(bytes[trimmed], b' ' | b'\t') {
                    trimmed += 1;
                }
                let l_end = find_line_end(bytes, trimmed);
                if text[trimmed..l_end].trim() == "```" {
                    body_end = l_start;
                    closed_end = l_end;
                    cursor = next_line(bytes, l_end);
                    break;
                }
                cursor = next_line(bytes, l_end);
            }
            let raw_code = text[body_start..body_end]
                .strip_suffix("\r\n")
                .or_else(|| text[body_start..body_end].strip_suffix('\n'))
                .unwrap_or(&text[body_start..body_end]);
            let code = if fence_indent.is_empty() {
                raw_code.to_owned()
            } else {
                raw_code
                    .split('\n')
                    .map(|line| line.strip_prefix(fence_indent).unwrap_or(line))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let attrs = std::mem::take(&mut pending_attrs);
            blocks.push(BlockPiece::CodeBlock {
                lang,
                text: code,
                attrs,
                span: mk_span(base, cb_start, closed_end),
            });
            i = cursor;
            continue;
        }

        // 4. Standalone `#expr` block item (may span multiple lines when calling `[ ... ]`).
        if bytes[i] == b'#'
            && let Some((expr, after_expr)) =
                try_parse_hash_expr(text, i, base, depth, diagnostics)
        {
            let mut rest = after_expr;
            while rest < len && matches!(bytes[rest], b' ' | b'\t' | b'\r') {
                rest += 1;
            }
            if rest >= len || bytes[rest] == b'\n' {
                let attrs = std::mem::take(&mut pending_attrs);
                blocks.push(BlockPiece::Expr {
                    span: mk_span(base, i, after_expr),
                    expr: Box::new(expr),
                    attrs,
                });
                i = if rest < len { rest + 1 } else { rest };
                continue;
            }
        }

        // 5. Paragraph: accumulate consecutive non-blank inline lines.
        let para_start = i;
        let mut lines: Vec<(usize, usize)> = Vec::new();
        let mut cursor = line_start;
        while cursor < len {
            let mut trimmed = cursor;
            while trimmed < len && matches!(bytes[trimmed], b' ' | b'\t' | b'\r') {
                trimmed += 1;
            }
            if trimmed >= len || bytes[trimmed] == b'\n' {
                break;
            }
            // Stop paragraph before a new block starter (after at least 1 line).
            if !lines.is_empty() && is_block_starter(text, bytes, trimmed) {
                break;
            }
            let l_end = find_line_end(bytes, trimmed);
            let mut trimmed_end = l_end;
            while trimmed_end > trimmed
                && matches!(bytes[trimmed_end - 1], b' ' | b'\t' | b'\r')
            {
                trimmed_end -= 1;
            }
            lines.push((trimmed, trimmed_end));
            cursor = next_line(bytes, l_end);
        }

        let para_end = lines.last().map_or(para_start, |&(_, e)| e);
        let mut inlines = Vec::new();
        for (idx, (ls, le)) in lines.into_iter().enumerate() {
            if idx > 0 {
                inlines.push(InlinePiece::Text(" ".into(), mk_span(base, ls, ls)));
            }
            inlines.extend(parse_inlines(&text[ls..le], base + ls, depth, diagnostics));
        }
        let attrs = std::mem::take(&mut pending_attrs);
        blocks.push(BlockPiece::Paragraph {
            inlines,
            attrs,
            span: mk_span(base, para_start, para_end),
        });
        i = cursor;
    }

    blocks
}

fn is_block_starter(text: &str, bytes: &[u8], i: usize) -> bool {
    let len = bytes.len();
    if bytes[i] == b'=' {
        let mut eqs = 0;
        while i + eqs < len && bytes[i + eqs] == b'=' {
            eqs += 1;
        }
        if (1..=6).contains(&eqs) && i + eqs < len && matches!(bytes[i + eqs], b' ' | b'\t') {
            return true;
        }
    }
    if i + 2 < len && bytes[i] == b'`' && bytes[i + 1] == b'`' && bytes[i + 2] == b'`' {
        return true;
    }
    if bytes[i] == b'@' && i + 1 < len && bytes[i + 1] == b'(' {
        let l_end = find_line_end(bytes, i);
        if text[i..l_end].trim_end().ends_with(')') {
            return true;
        }
    }
    false
}

fn find_line_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    if i > start && bytes[i - 1] == b'\r' {
        i - 1
    } else {
        i
    }
}

fn next_line(bytes: &[u8], line_end: usize) -> usize {
    let mut i = line_end;
    if i < bytes.len() && bytes[i] == b'\r' {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'\n' {
        i += 1;
    }
    i
}

fn parse_inlines(
    text: &str,
    base: usize,
    depth: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Vec<InlinePiece> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut pieces = Vec::new();
    let mut text_buf = String::new();
    let mut text_start = 0usize;

    let flush_text = |pieces: &mut Vec<InlinePiece>, buf: &mut String, start: usize, end: usize| {
        if !buf.is_empty() {
            pieces.push(InlinePiece::Text(
                std::mem::take(buf),
                mk_span(base, start, end),
            ));
        }
    };

    while i < len {
        let b = bytes[i];
        match b {
            b'\\' if i + 1 < len => {
                if text_buf.is_empty() {
                    text_start = i;
                }
                let next_ch = text[i + 1..].chars().next().expect("valid utf-8");
                text_buf.push(next_ch);
                i += 1 + next_ch.len_utf8();
            }
            b'@' if i + 1 < len && bytes[i + 1] == b'(' => {
                if let Some((attrs, next_i)) = parse_attr_list(text, i, base, diagnostics) {
                    flush_text(&mut pieces, &mut text_buf, text_start, i);
                    let rest_pieces = parse_single_inline_target(
                        text,
                        next_i,
                        base,
                        depth,
                        diagnostics,
                    );
                    if let Some((target, after_target)) = rest_pieces {
                        pieces.push(InlinePiece::Attributed {
                            attrs,
                            target: Box::new(target),
                            span: mk_span(base, i, after_target),
                        });
                        i = after_target;
                        continue;
                    }
                    // Trailing `@(...)` with nothing to attach to: emit diagnostic and skip.
                    diagnostics.push(ParseDiagnostic {
                        span: mk_span(base, i, next_i),
                        message: "attribute annotation has no following content element".into(),
                    });
                    i = next_i;
                    continue;
                }
                if text_buf.is_empty() {
                    text_start = i;
                }
                text_buf.push('@');
                i += 1;
            }
            b'#' => {
                if let Some((expr, next_i)) =
                    try_parse_hash_expr(text, i, base, depth, diagnostics)
                {
                    flush_text(&mut pieces, &mut text_buf, text_start, i);
                    pieces.push(InlinePiece::Expr(Box::new(expr), mk_span(base, i, next_i)));
                    i = next_i;
                    continue;
                }
                if text_buf.is_empty() {
                    text_start = i;
                }
                text_buf.push('#');
                i += 1;
            }
            b'*' => {
                if let Some(close) = find_closing_delim(bytes, i + 1, b'*') {
                    flush_text(&mut pieces, &mut text_buf, text_start, i);
                    let inner = parse_inlines(&text[i + 1..close], base + i + 1, depth, diagnostics);
                    pieces.push(InlinePiece::Strong(inner, mk_span(base, i, close + 1)));
                    i = close + 1;
                    continue;
                }
                if text_buf.is_empty() {
                    text_start = i;
                }
                text_buf.push('*');
                i += 1;
            }
            b'_' => {
                if let Some(close) = find_closing_delim(bytes, i + 1, b'_') {
                    flush_text(&mut pieces, &mut text_buf, text_start, i);
                    let inner = parse_inlines(&text[i + 1..close], base + i + 1, depth, diagnostics);
                    pieces.push(InlinePiece::Emph(inner, mk_span(base, i, close + 1)));
                    i = close + 1;
                    continue;
                }
                if text_buf.is_empty() {
                    text_start = i;
                }
                text_buf.push('_');
                i += 1;
            }
            b'`' => {
                if let Some(close) = find_closing_backtick(bytes, i + 1) {
                    flush_text(&mut pieces, &mut text_buf, text_start, i);
                    pieces.push(InlinePiece::Raw(
                        text[i + 1..close].to_owned(),
                        mk_span(base, i, close + 1),
                    ));
                    i = close + 1;
                    continue;
                }
                if text_buf.is_empty() {
                    text_start = i;
                }
                text_buf.push('`');
                i += 1;
            }
            b'[' => {
                if let Some((label_end, target_start, target_end)) = parse_link_bounds(bytes, i) {
                    flush_text(&mut pieces, &mut text_buf, text_start, i);
                    let label =
                        parse_inlines(&text[i + 1..label_end], base + i + 1, depth, diagnostics);
                    let target = text[target_start..target_end].to_owned();
                    pieces.push(InlinePiece::Link {
                        target,
                        label,
                        span: mk_span(base, i, target_end + 1),
                    });
                    i = target_end + 1;
                    continue;
                }
                if text_buf.is_empty() {
                    text_start = i;
                }
                text_buf.push('[');
                i += 1;
            }
            _ => {
                if text_buf.is_empty() {
                    text_start = i;
                }
                let ch = text[i..].chars().next().expect("valid utf-8");
                text_buf.push(ch);
                i += ch.len_utf8();
            }
        }
    }

    flush_text(&mut pieces, &mut text_buf, text_start, len);
    pieces
}

fn parse_single_inline_target(
    text: &str,
    start: usize,
    base: usize,
    depth: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<(InlinePiece, usize)> {
    let bytes = text.as_bytes();
    if start >= bytes.len() {
        return None;
    }
    match bytes[start] {
        b'#' => {
            let (expr, next) = try_parse_hash_expr(text, start, base, depth, diagnostics)?;
            Some((
                InlinePiece::Expr(Box::new(expr), mk_span(base, start, next)),
                next,
            ))
        }
        b'*' => {
            let close = find_closing_delim(bytes, start + 1, b'*')?;
            let inner =
                parse_inlines(&text[start + 1..close], base + start + 1, depth, diagnostics);
            Some((
                InlinePiece::Strong(inner, mk_span(base, start, close + 1)),
                close + 1,
            ))
        }
        b'_' => {
            let close = find_closing_delim(bytes, start + 1, b'_')?;
            let inner =
                parse_inlines(&text[start + 1..close], base + start + 1, depth, diagnostics);
            Some((
                InlinePiece::Emph(inner, mk_span(base, start, close + 1)),
                close + 1,
            ))
        }
        b'`' => {
            let close = find_closing_backtick(bytes, start + 1)?;
            Some((
                InlinePiece::Raw(
                    text[start + 1..close].to_owned(),
                    mk_span(base, start, close + 1),
                ),
                close + 1,
            ))
        }
        b'[' => {
            let (label_end, target_start, target_end) = parse_link_bounds(bytes, start)?;
            let label = parse_inlines(
                &text[start + 1..label_end],
                base + start + 1,
                depth,
                diagnostics,
            );
            Some((
                InlinePiece::Link {
                    target: text[target_start..target_end].to_owned(),
                    label,
                    span: mk_span(base, start, target_end + 1),
                },
                target_end + 1,
            ))
        }
        _ => None,
    }
}

fn find_closing_delim(bytes: &[u8], start: usize, delim: u8) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == delim && i > start {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_closing_backtick(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'`' && i > start {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_link_bounds(bytes: &[u8], start: usize) -> Option<(usize, usize, usize)> {
    let mut i = start + 1;
    let mut depth = 1usize;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            b'[' => {
                depth += 1;
                i += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    if depth != 0 || i + 1 >= bytes.len() || bytes[i + 1] != b'(' {
        return None;
    }
    let label_end = i;
    let target_start = i + 2;
    let mut j = target_start;
    while j < bytes.len() && bytes[j] != b')' && bytes[j] != b'\n' {
        j += 1;
    }
    if j < bytes.len() && bytes[j] == b')' {
        Some((label_end, target_start, j))
    } else {
        None
    }
}

fn try_parse_hash_expr(
    text: &str,
    hash_pos: usize,
    base: usize,
    depth: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<(Expr, usize)> {
    let bytes = text.as_bytes();
    let after_hash = hash_pos + 1;
    if after_hash >= bytes.len() {
        return None;
    }
    if bytes[after_hash] == b'(' {
        let (expr, consumed) =
            parse_embedded_expr(&text[after_hash..], base + after_hash, depth, diagnostics)?;
        Some((expr, after_hash + consumed))
    } else if bytes[after_hash].is_ascii_alphabetic() || bytes[after_hash] == b'_' {
        let (expr, consumed) =
            parse_embedded_postfix(&text[after_hash..], base + after_hash, depth, diagnostics)?;
        Some((expr, after_hash + consumed))
    } else {
        None
    }
}

fn parse_attr_list(
    text: &str,
    at_pos: usize,
    base: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<(Vec<AttrEntry>, usize)> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    if at_pos + 1 >= len || bytes[at_pos] != b'@' || bytes[at_pos + 1] != b'(' {
        return None;
    }
    let mut i = at_pos + 2;
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

        let val_start = i;
        let value = if i < len && bytes[i] == b'"' {
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
            AttrLiteral::String(s)
        } else if text[i..].starts_with("true")
            && !bytes.get(i + 4).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            i += 4;
            AttrLiteral::Bool(true)
        } else if text[i..].starts_with("false")
            && !bytes.get(i + 5).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            i += 5;
            AttrLiteral::Bool(false)
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
                let Ok(v) = num.parse::<f64>() else {
                    return None;
                };
                AttrLiteral::Float(v)
            } else {
                let Ok(v) = num.parse::<i64>() else {
                    return None;
                };
                AttrLiteral::Int(v)
            }
        } else {
            diagnostics.push(ParseDiagnostic {
                span: mk_span(base, val_start, (val_start + 1).min(len)),
                message: "expected literal attribute value".into(),
            });
            return None;
        };

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
