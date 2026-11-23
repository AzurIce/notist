//! Rushdown-based CommonMark/GFM [`DocumentFrontend`] for `notc`.
//!
//! Dependency direction is strictly one-way (`notist-markdown -> notc`):
//! `notist-markdown` parses Markdown via `rushdown` with lightweight Notist
//! extensions (`@!(...)`, `@(...)`, `#import`, `#call(...)`/`#call[...]`,
//! `$...$`/`$$...$$` math, `[[wikilink]]`) and lowers directly into a standard
//! `notc::Module` exporting `fn __doc__() -> Block`. Any `[...]` content literal
//! inside an embedded `#call[...]` is parsed natively by `notc` (never re-entering
//! Markdown), eliminating Markup ↔ Code parser ping-pong.

mod math;
mod notist_ext;
mod wikilink;

use std::sync::Arc;

use notc::{
    AttrEntry, BlockPiece, Decl, DocumentFrontend, InlinePiece, ParseDiagnostic, ParseResult,
    Program, Span, TableAlign, build_document_module, embedded,
};
use rushdown::ast::{Arena, KindData, Node, NodeRef, TableCellAlignment, TextQualifier};
use rushdown::parser::{
    AnyBlockParser, InlineParser, NoParserOptions, Options, Parser, ParserExtension,
    gfm_strikethrough, gfm_table, parser_extension,
};
use rushdown::text::BasicReader;
use rushdown::util::{resolve_entity_references, resolve_numeric_references};

use crate::math::{MathBlock, MathBlockParser, MathInline, MathInlineParser};
use crate::notist_ext::{
    NotistBlock, NotistBlockKind, NotistBlockParser, NotistInline, NotistInlineParser,
    find_balanced_delim,
};
use crate::wikilink::{WikiLink, WikiLinkParser};

/// CommonMark + GFM + Notist-extension document frontend for `.md` / `.markdown` files.
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

/// Parse a Markdown document into a standard `notc::ParseResult` exporting `fn __doc__() -> Block`.
pub fn parse_markdown_document(source: &str) -> ParseResult {
    let source_arc: Arc<str> = Arc::from(source);
    let inline_ext = NotistInlineParser {
        source: Arc::clone(&source_arc),
    };
    let extension = gfm_table()
        .and(gfm_strikethrough())
        .and(parser_extension(move |p| {
            let inline_ext = inline_ext.clone();
            p.add_block_parser(
                || AnyBlockParser::Extension(Box::new(NotistBlockParser)),
                NoParserOptions,
                350,
            );
            p.add_block_parser(
                || AnyBlockParser::Extension(Box::new(MathBlockParser)),
                NoParserOptions,
                340,
            );
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
    let (arena, root) = parser.parse(&mut BasicReader::new(source));

    let mut lowerer = Lowerer {
        src: source,
        arena: &arena,
        root,
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

struct Lowerer<'a> {
    src: &'a str,
    arena: &'a Arena,
    root: NodeRef,
    declarations: Vec<Decl>,
    doc_attrs: Vec<AttrEntry>,
    diagnostics: Vec<ParseDiagnostic>,
}

impl<'a> Lowerer<'a> {
    fn node(&self, node: NodeRef) -> &'a Node {
        self.arena.get(node).expect("valid rushdown node ref")
    }

    fn span_of(&self, node: NodeRef) -> Span {
        let start = self.node(node).pos().unwrap_or(0) as u32;
        let mut end = self.src.len() as u32;
        let mut cur = Some(node);
        while let Some(c) = cur {
            if let Some(next) = self.node(c).next_sibling()
                && let Some(pos) = self.node(next).pos()
            {
                end = pos as u32;
                break;
            }
            cur = self.node(c).parent();
        }
        Span::new(start, end.max(start))
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
                        let nb_span = Span::new(nb_start as u32, nb_end as u32);
                        match nb.kind {
                            NotistBlockKind::DocAttr => {
                                if let Some((attrs, _)) = embedded::parse_doc_attr_list(
                                    &text,
                                    0,
                                    nb_start,
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
                                    nb_start,
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
                                if let Some(import) =
                                    embedded::parse_import(stmt, nb_start + 1, &mut self.diagnostics)
                                {
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
                                if let Some((expr, _)) = embedded::parse_postfix(
                                    slice,
                                    nb_start + 1,
                                    &mut self.diagnostics,
                                ) {
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
                                    nb_start,
                                    &mut self.diagnostics,
                                ) {
                                    let mut pos = next_i;
                                    while pos < bytes.len()
                                        && matches!(bytes[pos], b' ' | b'\t')
                                    {
                                        pos += 1;
                                    }
                                    if bytes.get(pos) == Some(&b'#')
                                        && let Some((expr, _)) = embedded::parse_postfix(
                                            &text[pos + 1..],
                                            nb_start + pos + 1,
                                            &mut self.diagnostics,
                                        )
                                    {
                                        blocks.push(BlockPiece::Attr {
                                            attrs,
                                            span: Span::new(nb_start as u32, (nb_start + next_i) as u32),
                                        });
                                        blocks.push(BlockPiece::Expr {
                                            expr: Box::new(expr),
                                            attrs: Vec::new(),
                                            span: Span::new((nb_start + pos) as u32, nb_end as u32),
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
                            span: Span::new(mb.start as u32, *mb.end.borrow() as u32),
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
                            span: Span::new(m.start as u32, m.end as u32),
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
                                if is_header && alignments.is_empty()
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
        let mut text_start: Option<u32> = None;
        let mut text_end: u32 = 0;

        let flush_text = |pieces: &mut Vec<InlinePiece>,
                          buf: &mut String,
                          start: &mut Option<u32>,
                          end: u32| {
            if let Some(s) = start.take() {
                let raw = std::mem::take(buf);
                if !raw.is_empty() {
                    pieces.push(InlinePiece::Text(
                        decode_md_text(&raw),
                        Span::new(s, end.max(s)),
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
                    let pos = self.node(c).pos().unwrap_or(0) as u32;
                    if text_start.is_none() {
                        text_start = Some(pos);
                    }
                    text_buf.push_str(s);
                    text_end = pos + s.len() as u32;
                    if t.has_qualifiers(TextQualifier::SOFT_LINE_BREAK)
                        || t.has_qualifiers(TextQualifier::HARD_LINE_BREAK)
                    {
                        text_buf.push(' ');
                    }
                }
                KindData::Extension(data) => {
                    if let Some(ni) = data.as_any().downcast_ref::<NotistInline>() {
                        flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
                        match ni {
                            NotistInline::Attr { text, start } => {
                                if let Some((attrs, end_rel)) = embedded::parse_attr_list(
                                    text,
                                    0,
                                    *start,
                                    &mut self.diagnostics,
                                ) {
                                    pieces.push(InlinePiece::Attr {
                                        attrs,
                                        span: Span::new(*start as u32, (*start + end_rel) as u32),
                                    });
                                }
                            }
                            NotistInline::Call { text, start, end } => {
                                let slice = text.strip_prefix('#').unwrap_or(text);
                                if let Some((expr, _)) = embedded::parse_postfix(
                                    slice,
                                    start + 1,
                                    &mut self.diagnostics,
                                ) {
                                    let call_span = Span::new(*start as u32, *end as u32);
                                    pieces.push(InlinePiece::Expr(Box::new(expr), call_span));
                                }
                            }
                        }
                    } else if let Some(m) = data.as_any().downcast_ref::<MathInline>() {
                        flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
                        pieces.push(InlinePiece::Math(
                            m.text.clone(),
                            Span::new(m.start as u32, m.end as u32),
                        ));
                    } else if let Some(wl) = data.as_any().downcast_ref::<WikiLink>() {
                        flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
                        let wl_span = Span::new(wl.start as u32, wl.end as u32);
                        let label_text = wl.label.clone().unwrap_or_else(|| wl.target.clone());
                        pieces.push(InlinePiece::Link {
                            target: wl.target.clone(),
                            label: vec![InlinePiece::Text(label_text, wl_span)],
                            span: wl_span,
                        });
                    }
                }
                KindData::CodeSpan(cs) => {
                    flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
                    pieces.push(InlinePiece::Raw(cs.str(self.src).into_owned(), span));
                }
                KindData::Emphasis(_) => {
                    flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
                    let inner = self.lower_inlines(c);
                    pieces.push(InlinePiece::Emph(inner, span));
                }
                KindData::Strong(_) => {
                    flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
                    let inner = self.lower_inlines(c);
                    pieces.push(InlinePiece::Strong(inner, span));
                }
                KindData::Link(link) => {
                    flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
                    let target = decode_md_text(link.destination().str(self.src));
                    let label = self.lower_inlines(c);
                    pieces.push(InlinePiece::Link {
                        target,
                        label,
                        span,
                    });
                }
                KindData::Image(img) => {
                    flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
                    let target = decode_md_text(img.destination().str(self.src));
                    let alt = self.collect_plain_text(c);
                    pieces.push(InlinePiece::Image { target, alt, span });
                }
                KindData::Strikethrough(_) => {
                    flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
                    let inner = self.lower_inlines(c);
                    pieces.push(InlinePiece::Strike(inner, span));
                }
                KindData::RawHtml(_) => {}
                _ => {
                    flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
                    let inner = self.lower_inlines(c);
                    pieces.extend(inner);
                }
            }

            child = next;
        }

        flush_text(&mut pieces, &mut text_buf, &mut text_start, text_end);
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
