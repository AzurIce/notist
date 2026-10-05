use std::cell::RefCell;

use notist_core::diag::{Diagnostic, Phase};
use notist_core::expr::Expr;
use notist_core::frontend::{Frontend, FrontendOptions, FrontendOutput};
use notist_core::item::{Dict, Value};
use rowan::TextRange;
use rushdown::ast::{Arena, KindData, Node, NodeRef, TextQualifier};
use rushdown::parser::{
    AnyBlockParser, AnyParagraphTransformer, InlineParser, NoParserOptions, Options, Parser,
    ParserExtension, gfm_strikethrough, gfm_table, parser_extension,
};
use rushdown::text::BasicReader;

mod breaks;
mod markup;
mod math;
mod strings;

/// Markdown and Notist Markdown source frontend.
#[derive(Debug, Default)]
pub struct MarkdownFrontend;

/// Rushdown's parsed tree, retained only when syntax capture is requested.
#[derive(Debug)]
pub struct MarkdownSyntax {
    pub source: String,
    pub arena: Arena,
    pub root: NodeRef,
}

impl Frontend for MarkdownFrontend {
    fn extensions(&self) -> &[&str] {
        &["md", "markdown", "notmd", "nmd"]
    }

    fn compile(&self, source: &str, options: FrontendOptions) -> FrontendOutput {
        compile_body(source, false, true, options.capture_syntax)
    }
}

fn compile_body(
    src: &str,
    inline: bool,
    allow_module: bool,
    capture_syntax: bool,
) -> FrontendOutput {
    let (mut arena, mut root) = parser(None, inline).parse(&mut BasicReader::new(src));
    let positions = breaks::positions(&arena, root, src);
    if !positions.is_empty() {
        (arena, root) =
            parser(Some(breaks::Breaks(positions)), inline).parse(&mut BasicReader::new(src));
    }
    let mut output = lower_parsed(src, &arena, root, allow_module);
    if capture_syntax {
        output.syntax = Some(Box::new(MarkdownSyntax {
            source: src.to_owned(),
            arena,
            root,
        }));
    }
    output
}

fn lower_parsed(src: &str, arena: &Arena, root: NodeRef, allow_module: bool) -> FrontendOutput {
    let lowerer = Lowerer {
        src,
        arena,
        root,
        allow_module,
        diagnostics: RefCell::new(Vec::new()),
        module_attrs: RefCell::new(Dict::default()),
    };
    let forest = lowerer.children(root);
    FrontendOutput {
        forest,
        module_attrs: lowerer.module_attrs.into_inner(),
        diagnostics: lowerer.diagnostics.into_inner(),
        syntax: None,
    }
}

fn parser(breaks: Option<breaks::Breaks>, inline: bool) -> Parser {
    let extension = gfm_strikethrough().and(parser_extension(move |p| {
        p.add_inline_parser(
            || Box::new(markup::MarkupInlineParser) as Box<dyn InlineParser>,
            NoParserOptions,
            150,
        );
        if inline {
            p.add_block_parser(
                || AnyBlockParser::Extension(Box::new(markup::InlineBodyParser)),
                NoParserOptions,
                0,
            );
        } else {
            p.add_block_parser(
                || AnyBlockParser::Extension(Box::new(markup::MarkupBlockParser)),
                NoParserOptions,
                350,
            );
        }
        p.add_inline_parser(
            || Box::new(math::MathParser) as Box<dyn InlineParser>,
            NoParserOptions,
            200,
        );
        if let Some(breaks) = breaks.clone() {
            p.add_paragraph_transformer(
                move || AnyParagraphTransformer::Extension(Box::new(breaks.clone())),
                NoParserOptions,
                300,
            );
        }
    }));
    if inline {
        Parser::with_extensions(Options::default(), extension)
    } else {
        Parser::with_extensions(Options::default(), extension.and(gfm_table()))
    }
}

struct Lowerer<'a> {
    src: &'a str,
    arena: &'a Arena,
    root: NodeRef,
    allow_module: bool,
    diagnostics: RefCell<Vec<Diagnostic>>,
    module_attrs: RefCell<Dict>,
}

fn normalize_table_inline(exprs: &mut [Expr]) {
    for expr in exprs {
        if let Expr::Call {
            name,
            fields,
            children,
            ..
        } = expr
        {
            if let Some(Value::Str(text)) = fields.get("text") {
                let text = match name.as_str() {
                    "math" => Some(text.replace("\\|", "|")),
                    _ => None,
                };
                if let Some(text) = text {
                    fields.insert("text", Value::Str(text));
                }
            }
            normalize_table_inline(children);
        }
    }
}

impl<'a> Lowerer<'a> {
    fn node(&self, node: NodeRef) -> &'a Node {
        self.arena.get(node).unwrap()
    }

    /// Flatten the complete inline description, including sibling text,
    /// nested formatting and code/math payloads, into the embedding field.
    fn description_of(&self, node: NodeRef) -> String {
        fn append(expr: &Expr, out: &mut String) {
            if let Expr::Call {
                fields, children, ..
            } = expr
            {
                if let Some(Value::Str(text)) =
                    fields.get("text").or_else(|| fields.get("description"))
                {
                    out.push_str(text);
                }
                for child in children {
                    append(child, out);
                }
            }
        }
        let mut out = String::new();
        for child in self.children(node) {
            append(&child, &mut out);
        }
        out
    }

    /// A node's span: its `pos` to the next sibling's `pos` (walking up
    /// through parents when there is none), or EOF. ATX headings and their
    /// descendants are bounded by the heading's physical line.
    fn span_of(&self, node: NodeRef) -> TextRange {
        if matches!(self.node(node).kind_data(), KindData::TableCell(_))
            && self.node(node).pos().is_none()
        {
            if let Some(parent) = self.node(node).parent() {
                // rushdown pads short rows with cells that have no source pos.
                return TextRange::empty(self.span_of(parent).end());
            }
        }
        let start = self.node(node).pos().unwrap_or(0) as u32;
        let mut end = self.src.len() as u32;
        let mut cur = Some(node);
        'walk: while let Some(c) = cur {
            if let Some(heading_end) = self.atx_heading_end(c) {
                end = heading_end;
                break;
            }
            if let Some(next) = self.node(c).next_sibling() {
                if let Some(pos) = self.node(next).pos() {
                    end = pos as u32;
                    break 'walk;
                }
            }
            cur = self.node(c).parent();
        }
        TextRange::new(start.into(), end.into())
    }

    fn atx_heading_end(&self, node: NodeRef) -> Option<u32> {
        let KindData::Heading(heading) = self.node(node).kind_data() else {
            return None;
        };
        let start = self.node(node).pos()?;
        let line = &self.src[start..];
        let marker = line.bytes().take_while(|b| *b == b'#').count();
        if marker != heading.level() as usize
            || !matches!(
                line.as_bytes().get(marker),
                None | Some(b' ' | b'\t' | b'\r' | b'\n')
            )
        {
            return None;
        }
        Some((start + line.find(['\r', '\n']).unwrap_or(line.len())) as u32)
    }

    fn children(&self, node: NodeRef) -> Vec<Expr> {
        let mut out = Vec::new();
        let mut text_start = None;
        let mut text_end = 0usize;
        let mut text_buf = String::new();
        let mut at_line_start = true;
        let flush = |out: &mut Vec<Expr>,
                     text_start: &mut Option<usize>,
                     text_buf: &mut String,
                     text_end: usize,
                     boundary: bool| {
            if let Some(start) = text_start.take() {
                if boundary {
                    text_buf.truncate(text_buf.trim_end().len());
                }
                if !text_buf.is_empty() {
                    out.push(Expr::text(
                        strings::decode(&std::mem::take(text_buf)),
                        TextRange::new((start as u32).into(), (text_end as u32).into()),
                    ));
                } else {
                    text_buf.clear();
                }
            }
        };

        let mut pending: Option<(Dict, TextRange, bool)> = None;
        let mut child = self.node(node).first_child();
        while let Some(c) = child {
            let n = self.node(c);
            if let KindData::Extension(data) = n.kind_data()
                && let Some(markup) = data.as_any().downcast_ref::<markup::Markup>()
            {
                flush(&mut out, &mut text_start, &mut text_buf, text_end, false);
                let lowered = markup.lower(&mut self.diagnostics.borrow_mut());
                match lowered {
                    markup::Lowered::Annotation {
                        attrs,
                        module,
                        block,
                        span,
                    } => {
                        if module {
                            if !self.allow_module || node != self.root || !block {
                                self.diagnostics.borrow_mut().push(Diagnostic::new(
                                    Phase::Semantic,
                                    span,
                                    "module annotation is only valid at the document top",
                                ));
                            } else {
                                if !out.is_empty() {
                                    self.diagnostics.borrow_mut().push(Diagnostic::new(
                                        Phase::Semantic,
                                        span,
                                        "module annotation must precede all content",
                                    ));
                                }
                                self.module_attrs.borrow_mut().extend(attrs);
                            }
                        } else if let Some((dict, range, inline)) = &mut pending {
                            dict.extend(attrs);
                            *range = range.cover(span);
                            *inline = !block;
                        } else {
                            pending = Some((attrs, span, !block));
                        }
                    }
                    markup::Lowered::Calls(exprs) => {
                        for mut expr in exprs {
                            self.attach(&mut expr, &mut pending);
                            out.push(expr);
                        }
                        at_line_start = false;
                    }
                }
                child = n.next_sibling();
                continue;
            }
            if pending.as_ref().is_some_and(|(_, _, inline)| *inline)
                && (matches!(n.kind_data(), KindData::Text(_))
                    || pending.as_ref().unwrap().1.end() != self.span_of(c).start())
            {
                let (_, span, _) = pending.take().unwrap();
                self.diagnostics.borrow_mut().push(Diagnostic::new(
                    Phase::Semantic,
                    span,
                    "annotation must be immediately followed by an element",
                ));
            }
            if let KindData::Text(text) = n.kind_data() {
                if text_start.is_none() {
                    text_start = n.pos();
                }
                let value = text.str(self.src);
                text_buf.push_str(if at_line_start {
                    value.trim_start()
                } else {
                    &value
                });
                text_end = n.pos().unwrap_or(0) + text.str(self.src).len();
                at_line_start = false;
                if text.has_qualifiers(TextQualifier::SOFT_LINE_BREAK)
                    || text.has_qualifiers(TextQualifier::HARD_LINE_BREAK)
                {
                    // Explicit backslash breaks have already split paragraphs.
                    // Remaining physical breaks join directly, including
                    // Markdown's two-space hard breaks.
                    flush(&mut out, &mut text_start, &mut text_buf, text_end, true);
                    at_line_start = true;
                }
            } else if !matches!(n.kind_data(), KindData::HtmlBlock(_) | KindData::RawHtml(_)) {
                // Ignore inline HTML tags without breaking the surrounding
                // text run; raw HTML blocks and their payload are skipped.
                flush(&mut out, &mut text_start, &mut text_buf, text_end, false);
                let mut expr = self.item(c);
                self.attach(&mut expr, &mut pending);
                out.push(expr);
                at_line_start = false;
            }
            child = n.next_sibling();
        }
        flush(&mut out, &mut text_start, &mut text_buf, text_end, true);
        if let Some((_, span, _)) = pending {
            self.diagnostics.borrow_mut().push(Diagnostic::new(
                Phase::Semantic,
                span,
                "annotation has no following element",
            ));
        }
        out
    }

    fn attach(&self, expr: &mut Expr, pending: &mut Option<(Dict, TextRange, bool)>) {
        if let Some((dict, span, inline)) = pending.take() {
            if inline && span.end() != expr.span().start() {
                self.diagnostics.borrow_mut().push(Diagnostic::new(
                    Phase::Semantic,
                    span,
                    "annotation must be immediately followed by an element",
                ));
            } else if let Expr::Call { attrs, .. } = expr {
                // Outer annotations precede annotations written on the target.
                let mut merged = dict;
                merged.extend(attrs.take());
                *attrs = merged;
            }
        }
    }

    fn item(&self, node: NodeRef) -> Expr {
        let span = self.span_of(node);
        match self.node(node).kind_data() {
            KindData::Paragraph(_) => {
                Expr::call("paragraph", span).with_children(self.children(node))
            }
            KindData::Heading(h) => Expr::call("heading", span)
                .with_field("level", Value::Int(h.level() as i64))
                .with_children(self.children(node)),
            KindData::ThematicBreak(_) => Expr::call("divider", span),
            KindData::CodeBlock(b) => {
                let mut expr = Expr::call("raw", span)
                    .with_field("block", Value::Bool(true))
                    .with_field(
                        "text",
                        Value::Str(b.value().iter(self.src).collect::<String>()),
                    );
                if let Some(info) = b.info() {
                    let lang = info.str(self.src).trim().to_string();
                    if !lang.is_empty() {
                        expr = expr.with_field("lang", Value::Str(lang));
                    }
                }
                expr
            }
            KindData::Blockquote(_) => Expr::call("callout", span)
                .with_field("kind", Value::Str("quote".into()))
                .with_children(self.children(node)),
            KindData::List(l) => Expr::call("list", span)
                .with_field("ordered", Value::Bool(l.is_ordered()))
                .with_field(
                    "start",
                    Value::Int(if l.is_ordered() { l.start() as i64 } else { 1 }),
                )
                .with_children(self.children(node)),
            KindData::ListItem(_) => Expr::call("item", span).with_children(self.children(node)),
            KindData::CodeSpan(c) => {
                Expr::call("raw", span).with_field("text", Value::Str(c.str(self.src).into_owned()))
            }
            KindData::Emphasis(_) => Expr::call("emph", span).with_children(self.children(node)),
            KindData::Strong(_) => Expr::call("strong", span).with_children(self.children(node)),
            KindData::Link(l) => Expr::call("link", span)
                .with_field(
                    "target",
                    Value::Str(strings::decode(l.destination().str(self.src))),
                )
                .with_children(self.children(node)),
            KindData::Image(i) => {
                let mut expr = Expr::call("embed", span)
                    .with_field(
                        "target",
                        Value::Str(strings::decode(i.destination().str(self.src))),
                    )
                    .with_field("description", Value::Str(self.description_of(node)));
                if let Some(title) = i.title_str(self.src) {
                    expr = expr.with_field("title", Value::Str(strings::decode(&title)));
                }
                expr
            }
            KindData::Table(_) => {
                let mut rows = Vec::new();
                let mut align = Vec::new();
                let mut container = self.node(node).first_child();
                while let Some(c) = container {
                    let header = matches!(self.node(c).kind_data(), KindData::TableHeader(_));
                    let mut row = self.node(c).first_child();
                    while let Some(r) = row {
                        if header {
                            let mut cell = self.node(r).first_child();
                            while let Some(cell_ref) = cell {
                                if let KindData::TableCell(data) = self.node(cell_ref).kind_data() {
                                    align.push(Value::Str(data.alignment().as_str().into()));
                                }
                                cell = self.node(cell_ref).next_sibling();
                            }
                        }
                        rows.push(self.item(r).with_field("header", Value::Bool(header)));
                        row = self.node(r).next_sibling();
                    }
                    container = self.node(c).next_sibling();
                }
                Expr::call("table", span)
                    .with_field("align", Value::Array(align))
                    .with_children(rows)
            }
            KindData::TableRow(_) => Expr::call("row", span).with_children(self.children(node)),
            KindData::TableCell(_) => {
                let mut inline = self.children(node);
                normalize_table_inline(&mut inline);
                let children = if inline.is_empty() {
                    Vec::new()
                } else {
                    vec![Expr::call("paragraph", span).with_children(inline)]
                };
                Expr::call("cell", span).with_children(children)
            }
            KindData::Strikethrough(_) => {
                Expr::call("strike", span).with_children(self.children(node))
            }
            KindData::Extension(data) if data.as_any().is::<math::Math>() => {
                let math = data.as_any().downcast_ref::<math::Math>().unwrap();
                let span = TextRange::new(span.start(), (math.end as u32).into());
                let expr = Expr::call("math", span);
                let expr = if math.block {
                    expr.with_field("block", Value::Bool(true))
                } else {
                    expr
                };
                expr.with_field("text", Value::Str(math.text.clone()))
            }
            _ => {
                // TableHeader / TableBody / LinkReferenceDefinition / 其他：子节点透传
                let mut out = Vec::new();
                out.extend(self.children(node));
                Expr::call("group", span).with_children(out)
            }
        }
    }
}
