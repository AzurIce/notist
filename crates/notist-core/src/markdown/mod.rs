//! Built-in CommonMark/GFM [`DocumentFrontend`] backed by `rushdown`.
//!
//! `rushdown` types are strictly encapsulated inside this module: both `.md`
//! documents and `[...]` content literals inside `.not` code lower directly
//! into `notist-core`'s frontend-agnostic AST IR (`BlockPiece` / `InlinePiece` / `Module`).

mod math;
mod notist_ext;
mod wikilink;

use std::sync::Arc;

use rushdown::ast::{Arena, KindData, Node, NodeRef, TableCellAlignment, TextQualifier};
use rushdown::parser::{
    AnyBlockParser, InlineParser, NoParserOptions, Options, Parser, ParserExtension,
    gfm_strikethrough, gfm_table, parser_extension,
};
use rushdown::text::BasicReader;
use rushdown::util::{resolve_entity_references, resolve_numeric_references};

use crate::ast::{
    AttrEntry, BlockPiece, Decl, Expr, ExprKind, InlinePiece, Program, Span, TableAlign,
};
use crate::attrs::{attach_block_attrs, attach_inline_attrs};
use crate::frontend::{DocumentFrontend, build_document_module, embedded};
use crate::lexer::ParseDiagnostic;
use crate::markdown::math::{MathBlock, MathBlockParser, MathInline, MathInlineParser};
use crate::markdown::notist_ext::{
    NotistBlock, NotistBlockKind, NotistBlockParser, NotistInline, NotistInlineParser,
    find_balanced_delim,
};
use crate::markdown::wikilink::{WikiLink, WikiLinkParser};
use crate::parser::{ParseResult, parse_embedded_expr, parse_embedded_postfix};

/// Built-in CommonMark + GFM + Notist-extension document frontend for `.md` / `.markdown` files.
#[derive(Debug, Default, Clone, Copy)]
pub struct MarkdownFrontend;

impl DocumentFrontend for MarkdownFrontend {
    fn extensions(&self) -> &[&str] {
        &["md", "markdown"]
    }

    fn parse_document(&self, source: &str) -> ParseResult {
        parse_markdown_document(source)
    }
}

/// Parse a Markdown document into a standard `notist_core::ParseResult` exporting `fn __doc__() -> Block`.
pub fn parse_markdown_document(source: &str) -> ParseResult {
    let source_arc: Arc<str> = Arc::from(source);
    let (arena, root) = parse_with_rushdown(source_arc, false, true, false);

    let mut lowerer = Lowerer {
        src: source,
        arena: &arena,
        root,
        span_map: SpanMap::Identity,
        depth: 0,
        declarations: Vec::new(),
        doc_attrs: Vec::new(),
        diagnostics: Vec::new(),
    };
    let blocks = lowerer.lower_blocks(root);
    let full_len = source.len() as u32;
    let module = build_document_module(
        lowerer.declarations,
        lowerer.doc_attrs,
        blocks,
        Span::new(0, full_len),
        full_len,
        &mut lowerer.diagnostics,
    );
    ParseResult {
        program: Program { root: module },
        diagnostics: lowerer.diagnostics,
    }
}

/// Parse a `[...]` content literal embedded inside `.not` code using the same
/// CommonMark/GFM + Notist extension parser.
///
/// Uses lexical flanking whitespace to distinguish flavor:
/// - `[]` / `[hello **world**]` -> `ExprKind::InlineLiteral`
/// - `[ ]` / `[ hello ]` / multiline `[\n...\n]` -> `ExprKind::BlockLiteral`
pub(crate) fn parse_content_literal(
    raw: &str,
    literal_span: Span,
    depth: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Expr {
    let inner = &raw[1..raw.len() - 1];
    let base = literal_span.start as usize + 1;

    let is_ws = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n');
    let is_block = inner.bytes().next().is_some_and(is_ws)
        && inner.bytes().last().is_some_and(is_ws);

    if is_block {
        if inner.trim().is_empty() {
            return Expr::new(ExprKind::BlockLiteral(Vec::new()), literal_span);
        }
        let (dedented, line_map) = dedent_block_literal(inner, base);
        let source_arc: Arc<str> = Arc::from(dedented.as_str());
        let (arena, root) = parse_with_rushdown(source_arc, true, false, false);
        let mut lowerer = Lowerer {
            src: &dedented,
            arena: &arena,
            root,
            span_map: SpanMap::DedentedLines(line_map),
            depth,
            declarations: Vec::new(),
            doc_attrs: Vec::new(),
            diagnostics: Vec::new(),
        };
        let blocks = lowerer.lower_blocks(root);
        diagnostics.extend(lowerer.diagnostics);
        let blocks = attach_block_attrs(blocks, diagnostics);
        Expr::new(ExprKind::BlockLiteral(blocks), literal_span)
    } else {
        if inner.is_empty() {
            return Expr::new(ExprKind::InlineLiteral(Vec::new()), literal_span);
        }
        // Wrap with 2-byte sentinels `"x "` and `" x"` so CommonMark block starters
        // (`- `, `# `, `> `, `1. `) never trigger and intentional leading/trailing spaces
        // in inline literals like `[Hello ] + strong[world]` are preserved verbatim.
        const PREFIX: &str = "x ";
        const SUFFIX: &str = " x";
        let prefixed = format!("{PREFIX}{inner}{SUFFIX}");
        let source_arc: Arc<str> = Arc::from(prefixed.as_str());
        let (arena, root) = parse_with_rushdown(source_arc, true, false, true);
        let mut lowerer = Lowerer {
            src: &prefixed,
            arena: &arena,
            root,
            span_map: SpanMap::InlinePrefix {
                base,
                prefix_len: PREFIX.len(),
                max_len: inner.len(),
            },
            depth,
            declarations: Vec::new(),
            doc_attrs: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut inlines = Vec::new();
        let mut child = lowerer.node(root).first_child();
        let mut first_para = true;
        while let Some(c) = child {
            let next = lowerer.node(c).next_sibling();
            if !first_para && !inlines.is_empty() {
                inlines.push(InlinePiece::Text(" ".into(), lowerer.span_of(c)));
            }
            first_para = false;
            inlines.extend(lowerer.lower_inlines(c));
            child = next;
        }
        strip_inline_sentinels(&mut inlines, PREFIX, SUFFIX);
        diagnostics.extend(lowerer.diagnostics);
        let inlines = attach_inline_attrs(inlines, diagnostics);
        Expr::new(ExprKind::InlineLiteral(inlines), literal_span)
    }
}

fn strip_inline_sentinels(inlines: &mut Vec<InlinePiece>, prefix: &str, suffix: &str) {
    if let Some(InlinePiece::Text(text, _)) = inlines.first_mut()
        && let Some(stripped) = text.strip_prefix(prefix)
    {
        if stripped.is_empty() {
            inlines.remove(0);
        } else {
            *text = stripped.to_owned();
        }
    }
    if let Some(InlinePiece::Text(text, _)) = inlines.last_mut()
        && let Some(stripped) = text.strip_suffix(suffix)
    {
        if stripped.is_empty() {
            inlines.pop();
        } else {
            *text = stripped.to_owned();
        }
    }
}

fn dedent_block_literal(inner: &str, base: usize) -> (String, Vec<(usize, usize)>) {
    // Strip one optional leading newline and trailing whitespace-only line so
    // `[\n    hello\n]` starts cleanly at the first content line.
    let mut start_trim = 0usize;
    if inner.starts_with("\r\n") {
        start_trim = 2;
    } else if inner.starts_with('\n') {
        start_trim = 1;
    }
    let slice = &inner[start_trim..];
    let slice_base = base + start_trim;

    let mut min_indent = usize::MAX;
    for line in slice.split('\n') {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.trim().is_empty() {
            continue;
        }
        let indent = trimmed
            .bytes()
            .take_while(|b| matches!(b, b' ' | b'\t'))
            .count();
        min_indent = min_indent.min(indent);
    }
    if min_indent == usize::MAX {
        min_indent = 0;
    }

    let mut out = String::with_capacity(slice.len());
    let mut line_map: Vec<(usize, usize)> = Vec::new();
    let mut orig_offset = 0usize;
    for (idx, raw_line) in slice.split('\n').enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let strip = if raw_line.trim().is_empty() {
            raw_line.len().min(min_indent)
        } else {
            min_indent
        };
        let ded_start = out.len();
        let orig_start = slice_base + orig_offset + strip;
        line_map.push((ded_start, orig_start));
        out.push_str(&raw_line[strip..]);
        orig_offset += raw_line.len() + 1;
    }
    (out, line_map)
}

fn parse_with_rushdown(
    source_arc: Arc<str>,
    allow_bare_ident: bool,
    allow_top_level_directives: bool,
    inline_only: bool,
) -> (Arena, NodeRef) {
    let inline_ext = NotistInlineParser {
        source: Arc::clone(&source_arc),
        allow_bare_ident,
    };
    let block_ext = NotistBlockParser {
        allow_bare_ident,
        allow_top_level_directives,
    };

    let extension = gfm_table()
        .and(gfm_strikethrough())
        .and(parser_extension(move |p| {
            let inline_ext = inline_ext.clone();
            if !inline_only {
                p.add_block_parser(
                    move || AnyBlockParser::Extension(Box::new(block_ext)),
                    NoParserOptions,
                    350,
                );
                p.add_block_parser(
                    || AnyBlockParser::Extension(Box::new(MathBlockParser)),
                    NoParserOptions,
                    340,
                );
            }
            p.add_inline_parser(
                move || Box::new(inline_ext.clone()) as Box<dyn InlineParser>,
                NoParserOptions,
                150,
            );
            p.add_inline_parser(
                || Box::new(WikiLinkParser) as Box<dyn InlineParser>,
                NoParserOptions,
                150,
            );
            p.add_inline_parser(
                || Box::new(MathInlineParser) as Box<dyn InlineParser>,
                NoParserOptions,
                200,
            );
        }));

    let parser = Parser::with_extensions(Options::default(), extension);
    parser.parse(&mut BasicReader::new(&source_arc))
}

enum SpanMap {
    Identity,
    InlinePrefix {
        base: usize,
        prefix_len: usize,
        max_len: usize,
    },
    DedentedLines(Vec<(usize, usize)>),
}

impl SpanMap {
    fn map_offset(&self, pos: usize) -> usize {
        match self {
            Self::Identity => pos,
            Self::InlinePrefix {
                base,
                prefix_len,
                max_len,
            } => base + pos.saturating_sub(*prefix_len).min(*max_len),
            Self::DedentedLines(lines) => {
                if lines.is_empty() {
                    return pos;
                }
                let idx = lines.partition_point(|&(ded, _)| ded <= pos).saturating_sub(1);
                let (ded_start, orig_start) = lines[idx];
                orig_start + pos.saturating_sub(ded_start)
            }
        }
    }

    fn map_span(&self, start: usize, end: usize) -> Span {
        let s = self.map_offset(start) as u32;
        let e = self.map_offset(end) as u32;
        Span::new(s, e.max(s))
    }
}

struct Lowerer<'a> {
    src: &'a str,
    arena: &'a Arena,
    root: NodeRef,
    span_map: SpanMap,
    depth: usize,
    declarations: Vec<Decl>,
    doc_attrs: Vec<AttrEntry>,
    diagnostics: Vec<ParseDiagnostic>,
}

impl<'a> Lowerer<'a> {
    fn node(&self, node: NodeRef) -> &'a Node {
        self.arena.get(node).expect("valid rushdown node ref")
    }

    fn span_of(&self, node: NodeRef) -> Span {
        let start = self.node(node).pos().unwrap_or(0);
        let mut end = self.src.len();
        let mut cur = Some(node);
        while let Some(c) = cur {
            if let Some(next) = self.node(c).next_sibling()
                && let Some(pos) = self.node(next).pos()
            {
                end = pos;
                break;
            }
            cur = self.node(c).parent();
        }
        self.span_map.map_span(start, end.max(start))
    }

    fn parse_hash_expr_slice(&mut self, slice: &str, raw_offset: usize) -> Option<Expr> {
        let mapped = self.span_map.map_offset(raw_offset);
        if slice.starts_with('(') {
            parse_embedded_expr(slice, mapped, self.depth, &mut self.diagnostics).map(|(e, _)| e)
        } else {
            parse_embedded_postfix(slice, mapped, self.depth, &mut self.diagnostics).map(|(e, _)| e)
        }
    }

    fn lower_blocks(&mut self, parent: NodeRef) -> Vec<BlockPiece> {
        let mut blocks = Vec::new();
        let mut child = self.node(parent).first_child();

        while let Some(c) = child {
            let next = self.node(c).next_sibling();
            let span = self.span_of(c);

            match self.node(c).kind_data() {
                KindData::Extension(data) => {
                    if let Some(nb) = data.as_any().downcast_ref::<NotistBlock>() {
                        let text = nb.text.borrow().clone();
                        let nb_start = nb.start;
                        let nb_end = *nb.end.borrow();
                        let nb_span = self.span_map.map_span(nb_start, nb_end);
                        let mapped_start = self.span_map.map_offset(nb_start);
                        match nb.kind {
                            NotistBlockKind::DocAttr => {
                                if let Some((attrs, _)) = embedded::parse_doc_attr_list(
                                    &text,
                                    0,
                                    mapped_start,
                                    &mut self.diagnostics,
                                ) {
                                    if parent == self.root {
                                        self.doc_attrs.extend(attrs);
                                    } else {
                                        self.diagnostics.push(ParseDiagnostic {
                                            span: nb_span,
                                            message: "document attribute `@!(...)` is only valid at top level".into(),
                                        });
                                    }
                                }
                            }
                            NotistBlockKind::BlockAttr => {
                                if let Some((attrs, _)) = embedded::parse_attr_list(
                                    &text,
                                    0,
                                    mapped_start,
                                    &mut self.diagnostics,
                                ) {
                                    blocks.push(BlockPiece::Attr {
                                        attrs,
                                        span: nb_span,
                                    });
                                }
                            }
                            NotistBlockKind::Import => {
                                let stmt = text.strip_prefix('#').unwrap_or(&text);
                                if let Some(import) = embedded::parse_import(
                                    stmt,
                                    self.span_map.map_offset(nb_start + 1),
                                    &mut self.diagnostics,
                                ) {
                                    if parent == self.root {
                                        self.declarations.push(Decl::Import(import));
                                    } else {
                                        self.diagnostics.push(ParseDiagnostic {
                                            span: nb_span,
                                            message: "`#import` is only valid at top level".into(),
                                        });
                                    }
                                }
                            }
                            NotistBlockKind::Call => {
                                let slice = text.strip_prefix('#').unwrap_or(&text);
                                if let Some(expr) = self.parse_hash_expr_slice(slice, nb_start + 1)
                                {
                                    blocks.push(BlockPiece::Expr {
                                        expr: Box::new(expr),
                                        attrs: Vec::new(),
                                        span: nb_span,
                                    });
                                }
                            }
                            NotistBlockKind::AttributedCall => {
                                let bytes = text.as_bytes();
                                if let Some((attrs, next_i)) = embedded::parse_attr_list(
                                    &text,
                                    0,
                                    mapped_start,
                                    &mut self.diagnostics,
                                ) {
                                    let mut pos = next_i;
                                    while pos < bytes.len()
                                        && matches!(bytes[pos], b' ' | b'\t')
                                    {
                                        pos += 1;
                                    }
                                    if bytes.get(pos) == Some(&b'#')
                                        && let Some(expr) = self
                                            .parse_hash_expr_slice(&text[pos + 1..], nb_start + pos + 1)
                                    {
                                        blocks.push(BlockPiece::Attr {
                                            attrs,
                                            span: self.span_map.map_span(nb_start, nb_start + next_i),
                                        });
                                        blocks.push(BlockPiece::Expr {
                                            expr: Box::new(expr),
                                            attrs: Vec::new(),
                                            span: self.span_map.map_span(nb_start + pos, nb_end),
                                        });
                                    }
                                } else {
                                    let _ = find_balanced_delim(bytes, 1, b'(', b')');
                                }
                            }
                        }
                    } else if let Some(mb) = data.as_any().downcast_ref::<MathBlock>() {
                        let content = mb.lines.borrow().join("\n").trim().to_owned();
                        blocks.push(BlockPiece::Math {
                            text: content,
                            attrs: Vec::new(),
                            span: self.span_map.map_span(mb.start, *mb.end.borrow()),
                        });
                    }
                }
                KindData::Heading(h) => {
                    let level = h.level();
                    let title = self.lower_inlines(c);
                    blocks.push(BlockPiece::Heading {
                        level,
                        title,
                        attrs: Vec::new(),
                        span,
                    });
                }
                KindData::Paragraph(_) => {
                    // Check if paragraph contains a single block-flanked `$ ... $` math node.
                    if let Some(first) = self.node(c).first_child()
                        && self.node(first).next_sibling().is_none()
                        && let KindData::Extension(data) = self.node(first).kind_data()
                        && let Some(m) = data.as_any().downcast_ref::<MathInline>()
                        && m.block
                    {
                        blocks.push(BlockPiece::Math {
                            text: m.text.clone(),
                            attrs: Vec::new(),
                            span: self.span_map.map_span(m.start, m.end),
                        });
                    } else {
                        let inlines = self.lower_inlines(c);
                        if !inlines.is_empty() {
                            blocks.push(BlockPiece::Paragraph {
                                inlines,
                                attrs: Vec::new(),
                                span,
                            });
                        }
                    }
                }
                KindData::ThematicBreak(_) => {
                    blocks.push(BlockPiece::Divider {
                        attrs: Vec::new(),
                        span,
                    });
                }
                KindData::CodeBlock(cb) => {
                    let lang = cb
                        .info()
                        .map(|info| info.str(self.src).trim().to_owned())
                        .unwrap_or_default();
                    let text = cb
                        .value()
                        .iter(self.src)
                        .collect::<String>()
                        .replace("\r\n", "\n");
                    blocks.push(BlockPiece::CodeBlock {
                        lang,
                        text,
                        attrs: Vec::new(),
                        span,
                    });
                }
                KindData::Blockquote(_) => {
                    let body = self.lower_blocks(c);
                    blocks.push(BlockPiece::Quote {
                        body,
                        attrs: Vec::new(),
                        span,
                    });
                }
                KindData::List(list) => {
                    let ordered = list.is_ordered();
                    let start = if ordered {
                        u64::from(list.start())
                    } else {
                        1
                    };
                    let mut items = Vec::new();
                    let mut item = self.node(c).first_child();
                    while let Some(it) = item {
                        items.push(self.lower_blocks(it));
                        item = self.node(it).next_sibling();
                    }
                    blocks.push(BlockPiece::List {
                        ordered,
                        start,
                        items,
                        attrs: Vec::new(),
                        span,
                    });
                }
                KindData::Table(_) => {
                    let mut headers: Vec<Vec<InlinePiece>> = Vec::new();
                    let mut alignments: Vec<TableAlign> = Vec::new();
                    let mut rows: Vec<Vec<Vec<InlinePiece>>> = Vec::new();
                    let mut section = self.node(c).first_child();
                    while let Some(sec) = section {
                        let is_header =
                            matches!(self.node(sec).kind_data(), KindData::TableHeader(_));
                        let mut row_node = self.node(sec).first_child();
                        while let Some(r) = row_node {
                            let mut cells = Vec::new();
                            let mut row_aligns = Vec::new();
                            let mut cell_node = self.node(r).first_child();
                            while let Some(cell) = cell_node {
                                if is_header
                                    && alignments.is_empty()
                                    && let KindData::TableCell(tc) = self.node(cell).kind_data()
                                {
                                    row_aligns.push(match tc.alignment() {
                                        TableCellAlignment::Left => TableAlign::Left,
                                        TableCellAlignment::Center => TableAlign::Center,
                                        TableCellAlignment::Right => TableAlign::Right,
                                        TableCellAlignment::None | _ => TableAlign::None,
                                    });
                                }
                                cells.push(self.lower_inlines(cell));
                                cell_node = self.node(cell).next_sibling();
                            }
                            if is_header && headers.is_empty() {
                                headers = cells;
                                alignments = row_aligns;
                            } else {
                                rows.push(cells);
                            }
                            row_node = self.node(r).next_sibling();
                        }
                        section = self.node(sec).next_sibling();
                    }
                    blocks.push(BlockPiece::Table {
                        headers,
                        alignments,
                        rows,
                        attrs: Vec::new(),
                        span,
                    });
                }
                KindData::HtmlBlock(_) | KindData::LinkReferenceDefinition(_) => {}
                _ => {
                    let inner = self.lower_blocks(c);
                    blocks.extend(inner);
                }
            }

            child = next;
        }

        blocks
    }

    fn lower_inlines(&mut self, parent: NodeRef) -> Vec<InlinePiece> {
        let mut pieces: Vec<InlinePiece> = Vec::new();
        let mut text_buf = String::new();
        let mut text_start: Option<usize> = None;
        let mut text_end: usize = 0;

        let flush_text = |pieces: &mut Vec<InlinePiece>,
                          buf: &mut String,
                          start: &mut Option<usize>,
                          end: usize,
                          span_map: &SpanMap| {
            if let Some(s) = start.take() {
                let raw = std::mem::take(buf);
                if !raw.is_empty() {
                    pieces.push(InlinePiece::Text(
                        decode_md_text(&raw),
                        span_map.map_span(s, end.max(s)),
                    ));
                }
            }
        };

        let mut child = self.node(parent).first_child();
        while let Some(c) = child {
            let next = self.node(c).next_sibling();
            let span = self.span_of(c);

            match self.node(c).kind_data() {
                KindData::Text(t) => {
                    let s = t.str(self.src);
                    let pos = self.node(c).pos().unwrap_or(0);
                    if text_start.is_none() {
                        text_start = Some(pos);
                    }
                    text_buf.push_str(s);
                    text_end = pos + s.len();
                    if t.has_qualifiers(TextQualifier::SOFT_LINE_BREAK)
                        || t.has_qualifiers(TextQualifier::HARD_LINE_BREAK)
                    {
                        text_buf.push(' ');
                    }
                }
                KindData::Extension(data) => {
                    if let Some(ni) = data.as_any().downcast_ref::<NotistInline>() {
                        flush_text(
                            &mut pieces,
                            &mut text_buf,
                            &mut text_start,
                            text_end,
                            &self.span_map,
                        );
                        match ni {
                            NotistInline::Attr { text, start } => {
                                let mapped = self.span_map.map_offset(*start);
                                if let Some((attrs, end_rel)) = embedded::parse_attr_list(
                                    text,
                                    0,
                                    mapped,
                                    &mut self.diagnostics,
                                ) {
                                    pieces.push(InlinePiece::Attr {
                                        attrs,
                                        span: self.span_map.map_span(*start, *start + end_rel),
                                    });
                                }
                            }
                            NotistInline::Call { text, start, end } => {
                                let slice = text.strip_prefix('#').unwrap_or(text);
                                if let Some(expr) = self.parse_hash_expr_slice(slice, start + 1) {
                                    let call_span = self.span_map.map_span(*start, *end);
                                    pieces.push(InlinePiece::Expr(Box::new(expr), call_span));
                                }
                            }
                        }
                    } else if let Some(m) = data.as_any().downcast_ref::<MathInline>() {
                        flush_text(
                            &mut pieces,
                            &mut text_buf,
                            &mut text_start,
                            text_end,
                            &self.span_map,
                        );
                        pieces.push(InlinePiece::Math(
                            m.text.clone(),
                            self.span_map.map_span(m.start, m.end),
                        ));
                    } else if let Some(wl) = data.as_any().downcast_ref::<WikiLink>() {
                        flush_text(
                            &mut pieces,
                            &mut text_buf,
                            &mut text_start,
                            text_end,
                            &self.span_map,
                        );
                        let wl_span = self.span_map.map_span(wl.start, wl.end);
                        let label_text = wl.label.clone().unwrap_or_else(|| wl.target.clone());
                        pieces.push(InlinePiece::Link {
                            target: wl.target.clone(),
                            label: vec![InlinePiece::Text(label_text, wl_span)],
                            span: wl_span,
                        });
                    }
                }
                KindData::CodeSpan(cs) => {
                    flush_text(
                        &mut pieces,
                        &mut text_buf,
                        &mut text_start,
                        text_end,
                        &self.span_map,
                    );
                    pieces.push(InlinePiece::Raw(cs.str(self.src).into_owned(), span));
                }
                KindData::Emphasis(_) => {
                    flush_text(
                        &mut pieces,
                        &mut text_buf,
                        &mut text_start,
                        text_end,
                        &self.span_map,
                    );
                    let inner = self.lower_inlines(c);
                    pieces.push(InlinePiece::Emph(inner, span));
                }
                KindData::Strong(_) => {
                    flush_text(
                        &mut pieces,
                        &mut text_buf,
                        &mut text_start,
                        text_end,
                        &self.span_map,
                    );
                    let inner = self.lower_inlines(c);
                    pieces.push(InlinePiece::Strong(inner, span));
                }
                KindData::Link(link) => {
                    flush_text(
                        &mut pieces,
                        &mut text_buf,
                        &mut text_start,
                        text_end,
                        &self.span_map,
                    );
                    let target = decode_md_text(link.destination().str(self.src));
                    let label = self.lower_inlines(c);
                    pieces.push(InlinePiece::Link {
                        target,
                        label,
                        span,
                    });
                }
                KindData::Image(img) => {
                    flush_text(
                        &mut pieces,
                        &mut text_buf,
                        &mut text_start,
                        text_end,
                        &self.span_map,
                    );
                    let target = decode_md_text(img.destination().str(self.src));
                    let alt = self.collect_plain_text(c);
                    pieces.push(InlinePiece::Image { target, alt, span });
                }
                KindData::Strikethrough(_) => {
                    flush_text(
                        &mut pieces,
                        &mut text_buf,
                        &mut text_start,
                        text_end,
                        &self.span_map,
                    );
                    let inner = self.lower_inlines(c);
                    pieces.push(InlinePiece::Strike(inner, span));
                }
                KindData::RawHtml(_) => {}
                _ => {
                    flush_text(
                        &mut pieces,
                        &mut text_buf,
                        &mut text_start,
                        text_end,
                        &self.span_map,
                    );
                    let inner = self.lower_inlines(c);
                    pieces.extend(inner);
                }
            }

            child = next;
        }

        flush_text(
            &mut pieces,
            &mut text_buf,
            &mut text_start,
            text_end,
            &self.span_map,
        );
        pieces
    }

    fn collect_plain_text(&self, parent: NodeRef) -> String {
        let mut out = String::new();
        let mut child = self.node(parent).first_child();
        while let Some(c) = child {
            match self.node(c).kind_data() {
                KindData::Text(t) => out.push_str(&decode_md_text(t.str(self.src))),
                KindData::CodeSpan(cs) => out.push_str(&cs.str(self.src)),
                _ => out.push_str(&self.collect_plain_text(c)),
            }
            child = self.node(c).next_sibling();
        }
        out
    }
}

fn decode_md_text(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch == '\\'
            && let Some(&(_, next)) = chars.peek()
            && next.is_ascii_punctuation()
        {
            out.push(next);
            chars.next();
            continue;
        }
        if ch == '&' {
            let len = src[start + 1..]
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'#')
                .count();
            let end = start + 1 + len;
            if src.as_bytes().get(end) == Some(&b';') {
                let reference = &src.as_bytes()[start..=end];
                let decoded = resolve_entity_references(resolve_numeric_references(reference));
                out.push_str(&String::from_utf8_lossy(&decoded));
                for _ in 0..=len {
                    chars.next();
                }
                continue;
            }
        }
        out.push(ch);
    }
    out
}
