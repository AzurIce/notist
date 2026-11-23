//! Markup parser for `[...]` content literals embedded inside `.notc`.
//!
//! Uses Typst-style flanking whitespace to determine flavor purely lexically:
//! - `[hello]` -> `InlineLiteral`
//! - `[ hello ]` or multi-line `[\n...\n]` -> `BlockLiteral`

use crate::ast::{
    AttrEntry, AttrLiteral, BlockPiece, Decl, Expr, ExprKind, InlinePiece, Module, Span,
    TableAlign,
};
use crate::frontend::build_document_module;
use crate::lexer::ParseDiagnostic;
use crate::parser::{parse_embedded_expr, parse_embedded_import, parse_embedded_postfix};

/// Parse a standalone `.not` document file into a `notc::Module`:
/// - Leading `@!(key: value, ...)` document-level attributes attach to the root `Block`.
/// - Leading `#import ...` lines become `Decl::Import(...)` (with optional trailing `;`).
/// - Remaining top-level Markup becomes synthetic `fn __doc__() -> Block`.
pub(crate) fn parse_document_source(
    source: &str,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Module {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut declarations: Vec<Decl> = Vec::new();
    let mut doc_attrs: Vec<AttrEntry> = Vec::new();

    // Parse leading preamble: blank lines, `@!(...)` document attributes, and `#import ...`.
    while i < len {
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

        // Document attribute block: `@!(key: value, ...)`
        if bytes[i] == b'@'
            && i + 2 < len
            && bytes[i + 1] == b'!'
            && bytes[i + 2] == b'('
            && let Some((attrs, next_i)) = parse_attr_list_at(source, i, i + 2, 0, diagnostics)
        {
            let mut rest = next_i;
            while rest < len && matches!(bytes[rest], b' ' | b'\t' | b'\r') {
                rest += 1;
            }
            if rest >= len || bytes[rest] == b'\n' {
                doc_attrs.extend(attrs);
                i = if rest < len { rest + 1 } else { rest };
                continue;
            }
        }

        // Top-level `#import ...` directive
        if bytes[i] == b'#' && source[i + 1..].starts_with("import") {
            let after_kw = i + 7;
            if after_kw >= len
                || matches!(bytes[after_kw], b' ' | b'\t' | b'"' | b'\r' | b'\n')
            {
                let line_end = find_line_end(bytes, i);
                let stmt_slice = &source[i + 1..line_end];
                if let Some(import) = parse_embedded_import(stmt_slice, i + 1, diagnostics) {
                    declarations.push(Decl::Import(import));
                }
                i = next_line(bytes, line_end);
                continue;
            }
        }

        // First non-preamble line starts the document body.
        i = line_start;
        break;
    }

    let body_start = i;
    let blocks = parse_blocks(&source[body_start..], body_start, 0, diagnostics);
    let body_span = Span::new(body_start as u32, len as u32);
    build_document_module(
        declarations,
        doc_attrs,
        blocks,
        body_span,
        len as u32,
        diagnostics,
    )
}

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
        let blocks = attach_block_attrs(parse_blocks(inner, base, depth, diagnostics), diagnostics);
        Expr::new(ExprKind::BlockLiteral(blocks), literal_span)
    } else {
        let inlines =
            attach_inline_attrs(parse_inlines(inner, base, depth, diagnostics), diagnostics);
        Expr::new(ExprKind::InlineLiteral(inlines), literal_span)
    }
}

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
                // (e.g. `@(collapsed: true) [A](a.not)`), keeping the space in output.
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

        // 1. Block attribute line: `@(key: value, ...)` (either standalone or immediately before `#expr`)
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
                blocks.push(BlockPiece::Attr {
                    attrs,
                    span: mk_span(base, i, next_i),
                });
                i = if rest < len { rest + 1 } else { rest };
                continue;
            }
            if bytes[rest] == b'#'
                && let Some((expr, after_expr)) =
                    try_parse_hash_expr(text, rest, base, depth, diagnostics)
            {
                let mut after = after_expr;
                while after < len && matches!(bytes[after], b' ' | b'\t' | b'\r') {
                    after += 1;
                }
                if after >= len || bytes[after] == b'\n' {
                    blocks.push(BlockPiece::Attr {
                        attrs,
                        span: mk_span(base, i, next_i),
                    });
                    blocks.push(BlockPiece::Expr {
                        span: mk_span(base, rest, after_expr),
                        expr: Box::new(expr),
                        attrs: Vec::new(),
                    });
                    i = if after < len { after + 1 } else { after };
                    continue;
                }
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
                blocks.push(BlockPiece::Heading {
                    level: eqs as u8,
                    title,
                    attrs: Vec::new(),
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
            let raw_code = &text[body_start..body_end];
            let code = if fence_indent.is_empty() {
                raw_code.replace("\r\n", "\n")
            } else {
                let mut out = String::with_capacity(raw_code.len());
                for segment in raw_code.split_inclusive('\n') {
                    let (body_part, nl) = if let Some(s) = segment.strip_suffix("\r\n") {
                        (s, "\n")
                    } else if let Some(s) = segment.strip_suffix('\n') {
                        (s, "\n")
                    } else {
                        (segment, "")
                    };
                    out.push_str(body_part.strip_prefix(fence_indent).unwrap_or(body_part));
                    out.push_str(nl);
                }
                out
            };
            blocks.push(BlockPiece::CodeBlock {
                lang,
                text: code,
                attrs: Vec::new(),
                span: mk_span(base, cb_start, closed_end),
            });
            i = cursor;
            continue;
        }

        // 4. Horizontal divider: `---` (3+ hyphens on its own line)
        if is_divider_line(text, bytes, i) {
            let l_end = find_line_end(bytes, i);
            blocks.push(BlockPiece::Divider {
                attrs: Vec::new(),
                span: mk_span(base, i, l_end),
            });
            i = next_line(bytes, l_end);
            continue;
        }

        // 5. Blockquote: consecutive lines starting with `>`
        if bytes[i] == b'>' && (i + 1 >= len || matches!(bytes[i + 1], b' ' | b'\t' | b'\r' | b'\n'))
        {
            let q_start = i;
            let mut cursor = line_start;
            let mut quote_end = i;
            let mut inner_lines: Vec<&str> = Vec::new();
            while cursor < len {
                let mut trimmed = cursor;
                while trimmed < len && matches!(bytes[trimmed], b' ' | b'\t' | b'\r') {
                    trimmed += 1;
                }
                if trimmed >= len
                    || bytes[trimmed] != b'>'
                    || (trimmed + 1 < len
                        && !matches!(bytes[trimmed + 1], b' ' | b'\t' | b'\r' | b'\n'))
                {
                    break;
                }
                let l_end = find_line_end(bytes, trimmed);
                let after_gt = if trimmed + 1 < l_end && matches!(bytes[trimmed + 1], b' ' | b'\t') {
                    trimmed + 2
                } else {
                    trimmed + 1
                };
                inner_lines.push(&text[after_gt..l_end]);
                quote_end = l_end;
                cursor = next_line(bytes, l_end);
            }
            let joined = inner_lines.join("\n");
            let body = parse_blocks(&joined, base + q_start, depth + 1, diagnostics);
            blocks.push(BlockPiece::Quote {
                body,
                attrs: Vec::new(),
                span: mk_span(base, q_start, quote_end),
            });
            i = cursor;
            continue;
        }

        // 6. Unordered (`- ` / `* ` / `+ `) or ordered (`1. ` / `1) `) list (with indentation nesting)
        if list_marker_at(bytes, i).is_some() {
            let base_indent = indent_width(&text[line_start..i]);
            let (list_block, next_cursor) =
                parse_list_at(text, bytes, line_start, base_indent, base, depth, diagnostics);
            blocks.push(list_block);
            i = next_cursor;
            continue;
        }

        // 7. Pipe table (`| a | b |\n| --- | --- |\n| 1 | 2 |`)
        if bytes[i] == b'|'
            && let Some((table_block, next_cursor)) =
                try_parse_table_at(text, bytes, line_start, base, depth, diagnostics)
        {
            blocks.push(table_block);
            i = next_cursor;
            continue;
        }

        // 8. Block math (`$$` fenced or `$ ... $` flanked / multi-line `$`)
        if bytes[i] == b'$'
            && let Some((math_block, next_cursor)) = try_parse_block_math_at(text, bytes, i, base)
        {
            blocks.push(math_block);
            i = next_cursor;
            continue;
        }

        // 9. Standalone `#expr` block item.
        if bytes[i] == b'#'
            && let Some((expr, after_expr)) =
                try_parse_hash_expr(text, i, base, depth, diagnostics)
        {
            let mut rest = after_expr;
            while rest < len && matches!(bytes[rest], b' ' | b'\t' | b'\r') {
                rest += 1;
            }
            if rest >= len || bytes[rest] == b'\n' {
                blocks.push(BlockPiece::Expr {
                    span: mk_span(base, i, after_expr),
                    expr: Box::new(expr),
                    attrs: Vec::new(),
                });
                i = if rest < len { rest + 1 } else { rest };
                continue;
            }
        }

        // 10. Paragraph: accumulate consecutive non-blank inline lines.
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
        blocks.push(BlockPiece::Paragraph {
            inlines,
            attrs: Vec::new(),
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
    if is_divider_line(text, bytes, i) {
        return true;
    }
    if bytes[i] == b'>' && (i + 1 >= len || matches!(bytes[i + 1], b' ' | b'\t' | b'\r' | b'\n')) {
        return true;
    }
    if list_marker_at(bytes, i).is_some() {
        return true;
    }
    if bytes[i] == b'|' && is_table_header_at(text, bytes, i) {
        return true;
    }
    if bytes[i] == b'$' && is_block_math_starter(text, bytes, i) {
        return true;
    }
    false
}

fn indent_width(ws: &str) -> usize {
    ws.bytes()
        .map(|b| if b == b'\t' { 4 } else { 1 })
        .sum()
}

#[allow(clippy::too_many_arguments)]
fn strip_indent(line: &str, max_cols: usize) -> &str {
    let mut cols = 0usize;
    let mut byte_idx = 0usize;
    for (i, ch) in line.char_indices() {
        if cols >= max_cols {
            break;
        }
        match ch {
            ' ' => {
                cols += 1;
                byte_idx = i + 1;
            }
            '\t' => {
                cols += 4;
                byte_idx = i + 1;
            }
            _ => break,
        }
    }
    &line[byte_idx..]
}

fn parse_list_at(
    text: &str,
    bytes: &[u8],
    start_line: usize,
    base_indent: usize,
    base: usize,
    depth: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> (BlockPiece, usize) {
    let len = bytes.len();
    let mut cursor = start_line;
    let mut first_trimmed = start_line;
    while first_trimmed < len && matches!(bytes[first_trimmed], b' ' | b'\t' | b'\r') {
        first_trimmed += 1;
    }
    let (ordered, start_num, _) = list_marker_at(bytes, first_trimmed).expect("checked list marker");
    let list_start = first_trimmed;
    let mut list_end = first_trimmed;
    let mut items: Vec<Vec<BlockPiece>> = Vec::new();

    while cursor < len {
        let mut scan = cursor;
        while scan < len && matches!(bytes[scan], b' ' | b'\t' | b'\r') {
            scan += 1;
        }
        if scan >= len || bytes[scan] == b'\n' {
            break;
        }
        let line_indent = indent_width(&text[cursor..scan]);
        if line_indent != base_indent {
            break;
        }
        let Some((item_ordered, _, content_start)) = list_marker_at(bytes, scan) else {
            break;
        };
        if item_ordered != ordered {
            break;
        }

        let content_indent = indent_width(&text[cursor..content_start]);
        let first_end = find_line_end(bytes, content_start);
        let first_line = &text[content_start..first_end];
        let mut item_end = first_end;
        let mut next = next_line(bytes, first_end);

        // Collect indented continuation lines (nested blocks, multi-line annotations, nested lists).
        let mut cont_lines: Vec<&str> = Vec::new();
        while next < len {
            let mut look = next;
            while look < len && matches!(bytes[look], b' ' | b'\t' | b'\r') {
                look += 1;
            }
            if look >= len {
                break;
            }
            if bytes[look] == b'\n' {
                // Keep blank line only if a more-indented line follows it.
                let mut probe = next_line(bytes, look);
                while probe < len {
                    let mut p = probe;
                    while p < len && matches!(bytes[p], b' ' | b'\t' | b'\r') {
                        p += 1;
                    }
                    if p >= len || bytes[p] != b'\n' {
                        probe = p;
                        break;
                    }
                    probe = next_line(bytes, p);
                }
                if probe < len {
                    let line_begin = text[..probe].rfind('\n').map_or(0, |idx| idx + 1);
                    if indent_width(&text[line_begin..probe]) > base_indent {
                        cont_lines.push("");
                        next = next_line(bytes, look);
                        continue;
                    }
                }
                break;
            }
            let next_indent = indent_width(&text[next..look]);
            if next_indent <= base_indent {
                break;
            }
            let cont_end = find_line_end(bytes, next);
            cont_lines.push(strip_indent(&text[next..cont_end], content_indent));
            item_end = cont_end;
            next = next_line(bytes, cont_end);
        }

        let item_blocks = if cont_lines.is_empty() {
            parse_blocks(first_line, base + content_start, depth + 1, diagnostics)
        } else {
            let mut joined = String::from(first_line);
            for line in cont_lines {
                joined.push('\n');
                joined.push_str(line);
            }
            parse_blocks(&joined, base + content_start, depth + 1, diagnostics)
        };
        items.push(item_blocks);
        list_end = item_end;
        cursor = next;
    }

    (
        BlockPiece::List {
            ordered,
            start: start_num,
            items,
            attrs: Vec::new(),
            span: mk_span(base, list_start, list_end),
        },
        cursor,
    )
}

fn is_table_header_at(text: &str, bytes: &[u8], i: usize) -> bool {
    let l1_end = find_line_end(bytes, i);
    if !text[i..l1_end].contains('|') {
        return false;
    }
    let l2_start = next_line(bytes, l1_end);
    if l2_start >= bytes.len() {
        return false;
    }
    let l2_end = find_line_end(bytes, l2_start);
    is_table_separator_line(text[l2_start..l2_end].trim())
}

fn is_table_separator_line(line: &str) -> bool {
    if !line.contains('-') || !line.contains('|') {
        return false;
    }
    let trimmed = line.trim_matches('|');
    let cells: Vec<&str> = trimmed.split('|').collect();
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let c = cell.trim();
            let c = c.strip_prefix(':').unwrap_or(c);
            let c = c.strip_suffix(':').unwrap_or(c);
            c.len() >= 2 && c.bytes().all(|b| b == b'-')
        })
}

fn split_table_cells(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && matches!(bytes[start], b' ' | b'\t') {
        start += 1;
    }
    while end > start && matches!(bytes[end - 1], b' ' | b'\t' | b'\r') {
        end -= 1;
    }
    if start < end && bytes[start] == b'|' {
        start += 1;
    }
    if end > start && bytes[end - 1] == b'|' {
        end -= 1;
    }
    let mut out = Vec::new();
    let mut cell_start = start;
    let mut i = start;
    let mut in_code = false;
    while i < end {
        match bytes[i] {
            b'\\' if i + 1 < end => i += 2,
            b'`' => {
                in_code = !in_code;
                i += 1;
            }
            b'|' if !in_code => {
                out.push(trim_span(bytes, cell_start, i));
                cell_start = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    out.push(trim_span(bytes, cell_start, end));
    out
}

fn trim_span(bytes: &[u8], mut s: usize, mut e: usize) -> (usize, usize) {
    while s < e && matches!(bytes[s], b' ' | b'\t') {
        s += 1;
    }
    while e > s && matches!(bytes[e - 1], b' ' | b'\t') {
        e -= 1;
    }
    (s, e)
}

fn try_parse_table_at(
    text: &str,
    bytes: &[u8],
    line_start: usize,
    base: usize,
    depth: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<(BlockPiece, usize)> {
    let mut first = line_start;
    while first < bytes.len() && matches!(bytes[first], b' ' | b'\t' | b'\r') {
        first += 1;
    }
    if !is_table_header_at(text, bytes, first) {
        return None;
    }
    let h_end = find_line_end(bytes, line_start);
    let header_line = &text[line_start..h_end];
    let headers: Vec<Vec<InlinePiece>> = split_table_cells(header_line)
        .into_iter()
        .map(|(cs, ce)| parse_inlines(&header_line[cs..ce], base + line_start + cs, depth, diagnostics))
        .collect();

    let sep_start = next_line(bytes, h_end);
    let sep_end = find_line_end(bytes, sep_start);
    let sep_line = &text[sep_start..sep_end];
    let alignments: Vec<TableAlign> = split_table_cells(sep_line)
        .into_iter()
        .map(|(cs, ce)| {
            let cell = sep_line[cs..ce].trim();
            let left = cell.starts_with(':');
            let right = cell.ends_with(':');
            match (left, right) {
                (true, true) => TableAlign::Center,
                (true, false) => TableAlign::Left,
                (false, true) => TableAlign::Right,
                (false, false) => TableAlign::None,
            }
        })
        .collect();
    let mut cursor = next_line(bytes, sep_end);
    let mut table_end = sep_end;
    let mut rows: Vec<Vec<Vec<InlinePiece>>> = Vec::new();

    while cursor < bytes.len() {
        let mut trimmed = cursor;
        while trimmed < bytes.len() && matches!(bytes[trimmed], b' ' | b'\t' | b'\r') {
            trimmed += 1;
        }
        if trimmed >= bytes.len() || bytes[trimmed] == b'\n' || bytes[trimmed] != b'|' {
            break;
        }
        let r_end = find_line_end(bytes, cursor);
        let row_line = &text[cursor..r_end];
        let row_cells: Vec<Vec<InlinePiece>> = split_table_cells(row_line)
            .into_iter()
            .map(|(cs, ce)| parse_inlines(&row_line[cs..ce], base + cursor + cs, depth, diagnostics))
            .collect();
        rows.push(row_cells);
        table_end = r_end;
        cursor = next_line(bytes, r_end);
    }

    Some((
        BlockPiece::Table {
            headers,
            alignments,
            rows,
            attrs: Vec::new(),
            span: mk_span(base, first, table_end),
        },
        cursor,
    ))
}

fn is_block_math_starter(text: &str, bytes: &[u8], i: usize) -> bool {
    let l_end = find_line_end(bytes, i);
    let line = text[i..l_end].trim_end();
    line == "$$"
        || line == "$"
        || (line.starts_with("$ ") && line.ends_with(" $") && line.len() >= 4)
        || (line.starts_with("$$") && line.ends_with("$$") && line.len() >= 4)
}

fn try_parse_block_math_at(
    text: &str,
    bytes: &[u8],
    i: usize,
    base: usize,
) -> Option<(BlockPiece, usize)> {
    let len = bytes.len();
    let l_end = find_line_end(bytes, i);
    let line = text[i..l_end].trim_end();

    // Single-line `$ expr $` or `$$expr$$`
    if line.starts_with("$$") && line.ends_with("$$") && line.len() >= 4 {
        let content = line[2..line.len() - 2].trim().to_owned();
        return Some((
            BlockPiece::Math {
                text: content,
                attrs: Vec::new(),
                span: mk_span(base, i, l_end),
            },
            next_line(bytes, l_end),
        ));
    }
    if line.starts_with("$ ") && line.ends_with(" $") && line.len() >= 4 {
        let content = line[2..line.len() - 2].trim().to_owned();
        return Some((
            BlockPiece::Math {
                text: content,
                attrs: Vec::new(),
                span: mk_span(base, i, l_end),
            },
            next_line(bytes, l_end),
        ));
    }

    // Multi-line `$$` ... `$$` or `$` ... `$`
    if line == "$$" || line == "$" {
        let fence = line;
        let mut cursor = next_line(bytes, l_end);
        let body_start = cursor;
        let mut body_end = len;
        let mut closed_end = len;
        while cursor < len {
            let c_end = find_line_end(bytes, cursor);
            if text[cursor..c_end].trim() == fence {
                body_end = cursor;
                closed_end = c_end;
                cursor = next_line(bytes, c_end);
                break;
            }
            cursor = next_line(bytes, c_end);
        }
        let math_text = text[body_start..body_end].trim().to_owned();
        return Some((
            BlockPiece::Math {
                text: math_text,
                attrs: Vec::new(),
                span: mk_span(base, i, closed_end),
            },
            cursor,
        ));
    }

    None
}

fn is_divider_line(text: &str, bytes: &[u8], i: usize) -> bool {
    if i >= bytes.len() || bytes[i] != b'-' {
        return false;
    }
    let l_end = find_line_end(bytes, i);
    let trimmed = text[i..l_end].trim_end();
    trimmed.len() >= 3 && trimmed.bytes().all(|b| b == b'-')
}

/// If `bytes[i..]` starts with an unordered (`- `, `* `) or ordered (`1. `, `1) `)
/// list marker, returns `(ordered, start_number, content_start)`.
fn list_marker_at(bytes: &[u8], i: usize) -> Option<(bool, u64, usize)> {
    let len = bytes.len();
    if i >= len {
        return None;
    }
    if matches!(bytes[i], b'-' | b'*' | b'+') && i + 1 < len && matches!(bytes[i + 1], b' ' | b'\t')
    {
        let mut s = i + 1;
        while s < len && matches!(bytes[s], b' ' | b'\t') {
            s += 1;
        }
        return Some((false, 1, s));
    }
    if bytes[i].is_ascii_digit() {
        let mut j = i;
        while j < len && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j < len
            && matches!(bytes[j], b'.' | b')')
            && j + 1 < len
            && matches!(bytes[j + 1], b' ' | b'\t')
        {
            let start_num = std::str::from_utf8(&bytes[i..j])
                .ok()
                .and_then(|digits| digits.parse::<u64>().ok())
                .unwrap_or(1);
            let mut s = j + 1;
            while s < len && matches!(bytes[s], b' ' | b'\t') {
                s += 1;
            }
            return Some((true, start_num, s));
        }
    }
    None
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
                    pieces.push(InlinePiece::Attr {
                        attrs,
                        span: mk_span(base, i, next_i),
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
            b'_' if i == 0 || !bytes[i - 1].is_ascii_alphanumeric() => {
                if let Some(close) = find_closing_underscore(bytes, i + 1) {
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
            b'~' if i + 1 < len && bytes[i + 1] == b'~' => {
                if let Some(close) = find_closing_double_tilde(bytes, i + 2) {
                    flush_text(&mut pieces, &mut text_buf, text_start, i);
                    let inner =
                        parse_inlines(&text[i + 2..close], base + i + 2, depth, diagnostics);
                    pieces.push(InlinePiece::Strike(inner, mk_span(base, i, close + 2)));
                    i = close + 2;
                    continue;
                }
                if text_buf.is_empty() {
                    text_start = i;
                }
                text_buf.push('~');
                i += 1;
            }
            b'$' => {
                if i + 1 < len
                    && !matches!(bytes[i + 1], b' ' | b'\t' | b'\r' | b'\n' | b'$')
                    && let Some(close) = find_closing_delim(bytes, i + 1, b'$')
                    && !matches!(bytes[close - 1], b' ' | b'\t')
                {
                    flush_text(&mut pieces, &mut text_buf, text_start, i);
                    pieces.push(InlinePiece::Math(
                        text[i + 1..close].to_owned(),
                        mk_span(base, i, close + 1),
                    ));
                    i = close + 1;
                    continue;
                }
                if text_buf.is_empty() {
                    text_start = i;
                }
                text_buf.push('$');
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
            b'!' if i + 1 < len && bytes[i + 1] == b'[' => {
                if let Some((alt_end, target_start, target_end)) = parse_link_bounds(bytes, i + 1) {
                    flush_text(&mut pieces, &mut text_buf, text_start, i);
                    let alt = text[i + 2..alt_end].to_owned();
                    let target = text[target_start..target_end].to_owned();
                    pieces.push(InlinePiece::Image {
                        target,
                        alt,
                        span: mk_span(base, i, target_end + 1),
                    });
                    i = target_end + 1;
                    continue;
                }
                if text_buf.is_empty() {
                    text_start = i;
                }
                text_buf.push('!');
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

fn find_closing_double_tilde(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'~' && bytes[i + 1] == b'~' && i > start {
            return Some(i);
        }
        i += 1;
    }
    None
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

fn find_closing_underscore(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'_'
            && i > start
            && !bytes.get(i + 1).is_some_and(u8::is_ascii_alphanumeric)
        {
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
    if at_pos + 1 >= bytes.len() || bytes[at_pos] != b'@' || bytes[at_pos + 1] != b'(' {
        return None;
    }
    parse_attr_list_at(text, at_pos, at_pos + 1, base, diagnostics)
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
