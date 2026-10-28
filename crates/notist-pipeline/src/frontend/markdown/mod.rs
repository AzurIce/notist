use std::cell::RefCell;

use notist_core::annotation::{Annotation, AnnotationKind};
use notist_core::diag::{Diagnostic, Phase};
use notist_core::expr::Expr;
use notist_core::frontend::{Frontend, FrontendOptions, FrontendOutput};
use notist_core::item::{Dict, Value};
use notist_syntax::content::{ContentFlavor, ContentParser, ParsedContent};
use rowan::{TextRange, TextSize};
use rushdown::ast::{Arena, KindData, Node, NodeRef, TextQualifier, TypeData};
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

impl ContentParser for MarkdownFrontend {
    fn parse_content(&self, source: &str, open: TextSize) -> ParsedContent {
        assert_eq!(source.as_bytes()[usize::from(open)], b'[');
        let end = markup::content_end(source, usize::from(open));
        let closed = end.is_some();
        let end = end.unwrap_or(source.len());
        let body = TextRange::new(
            open + TextSize::new(1),
            (end as u32 - u32::from(closed)).into(),
        );
        let text = &source[usize::from(body.start())..usize::from(body.end())];
        let block = text.chars().next().is_some_and(char::is_whitespace)
            && text.chars().next_back().is_some_and(char::is_whitespace);
        let mut diagnostics = Vec::new();
        if !closed {
            diagnostics.push(notist_syntax::Diagnostic {
                span: TextRange::empty((end as u32).into()),
                message: "unclosed content literal".into(),
            });
        }
        if !block && markup::has_blank_line(text) {
            diagnostics.push(notist_syntax::Diagnostic {
                span: body,
                message: "inline content cannot contain a blank line".into(),
            });
        }
        ParsedContent {
            range: TextRange::new(open, (end as u32).into()),
            body,
            flavor: if block {
                ContentFlavor::Block
            } else {
                ContentFlavor::Inline
            },
            closed,
            syntax: Box::new(parse_body(text, !block)),
            diagnostics,
        }
    }
}

impl crate::lowering::content::ContentLowering for MarkdownFrontend {
    fn lower_content(
        &self,
        content: &ParsedContent,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> crate::lowering::content::Lowered {
        let syntax = content
            .syntax
            .as_ref()
            .as_any()
            .downcast_ref::<MarkdownSyntax>()
            .expect("Markdown content parser returns MarkdownSyntax");
        let output = lower_parsed(&syntax.source, &syntax.arena, syntax.root, false);
        let mut forest = content_forest(output.forest, content.flavor == ContentFlavor::Inline);
        for expr in &mut forest {
            expr.map_spans(&mut |span| span + content.body.start());
        }
        let mut annotations = output.annotations;
        for annotation in &mut annotations {
            annotation.map_spans(&mut |span| span + content.body.start());
        }
        diagnostics.extend(output.diagnostics.into_iter().map(|mut d| {
            d.span += content.body.start();
            d
        }));
        crate::lowering::content::Lowered {
            forest,
            annotations,
        }
    }
}

fn content_forest(forest: Vec<Expr>, inline: bool) -> Vec<Expr> {
    if !inline {
        return forest;
    }
    forest
        .into_iter()
        .flat_map(|expr| match expr {
            Expr::Fragment { children, .. } => children,
            expr => vec![expr],
        })
        .collect()
}

fn parse_body(src: &str, inline: bool) -> MarkdownSyntax {
    let (mut arena, mut root) = parser(None, inline).parse(&mut BasicReader::new(src));
    let positions = breaks::positions(&arena, root, src);
    if !positions.is_empty() {
        (arena, root) =
            parser(Some(breaks::Breaks(positions)), inline).parse(&mut BasicReader::new(src));
    }
    MarkdownSyntax {
        source: src.to_owned(),
        arena,
        root,
    }
}

fn compile_body(
    src: &str,
    inline: bool,
    allow_module: bool,
    capture_syntax: bool,
) -> FrontendOutput {
    let syntax = parse_body(src, inline);
    let mut output = lower_parsed(src, &syntax.arena, syntax.root, allow_module);
    if capture_syntax {
        output.syntax = Some(Box::new(syntax));
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
        annotations: RefCell::new(Vec::new()),
        imports: RefCell::new(Vec::new()),
    };
    let forest = lowerer.children(root);
    let imports = lowerer.imports.into_inner();
    FrontendOutput {
        imports,
        forest,
        annotations: lowerer.annotations.into_inner(),
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
    annotations: RefCell<Vec<Annotation>>,
    imports: RefCell<Vec<notist_core::definitions::Import>>,
}

/// Rushdown isolates standalone calls to keep their multi-line bodies opaque.
/// They still belong to the surrounding source fragment unless a hard
/// separator (blank line, explicit break or block syntax) intervenes.
fn append_fragment(out: &mut Vec<Expr>, expr: Expr, join: bool) {
    if let Expr::Fragment { mut children, span } = expr {
        if join
            && let Some(Expr::Fragment {
                children: previous,
                span: previous_span,
            }) = out.last_mut()
        {
            previous.append(&mut children);
            *previous_span = previous_span.cover(span);
        } else {
            out.push(Expr::fragment(children, span));
        }
    } else {
        out.push(expr);
    }
}

fn normalize_table_inline(exprs: &mut [Expr]) {
    for expr in exprs {
        if let Expr::Fragment { children, .. } = expr {
            normalize_table_inline(children);
            continue;
        }
        if let Expr::Call {
            callee,
            args,
            children,
            ..
        } = expr
        {
            if callee.name_path().as_deref() == Some("notist.math") {
                for arg in args {
                    if let notist_core::expr::CallArg::Named {
                        name,
                        value: Expr::Literal(Value::Str(text), _),
                    } = arg
                        && name == "text"
                    {
                        *text = text.replace("\\|", "|");
                    }
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
            if let Expr::Call { children, .. } = expr {
                if let Some(Value::Str(text)) = expr
                    .named_arg("text")
                    .or_else(|| expr.named_arg("description"))
                    .and_then(Expr::literal_value)
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
            && let Some(parent) = self.node(node).parent()
        {
            // rushdown pads short rows with cells that have no source pos.
            return TextRange::empty(self.span_of(parent).end());
        }
        let start = self.node(node).pos().unwrap_or(0) as u32;
        let mut end = self.src.len() as u32;
        let mut cur = Some(node);
        'walk: while let Some(c) = cur {
            if let Some(heading_end) = self.atx_heading_end(c) {
                end = heading_end;
                break;
            }
            if let Some(next) = self.node(c).next_sibling()
                && let Some(pos) = self.node(next).pos()
            {
                end = pos as u32;
                break 'walk;
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
        let mut join_fragment = false;
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

        let mut child = self.node(node).first_child();
        while let Some(c) = child {
            let n = self.node(c);
            let blank_before =
                matches!(n.type_data(), TypeData::Block(block) if block.has_blank_previous_line());
            let explicit_before = n.pos().is_some_and(|pos| {
                self.src[..pos]
                    .trim_end()
                    .bytes()
                    .rev()
                    .take_while(|ch| *ch == b'\\')
                    .count()
                    % 2
                    == 1
            });
            if let KindData::Extension(data) = n.kind_data()
                && let Some(markup) = data.as_any().downcast_ref::<markup::Markup>()
            {
                flush(&mut out, &mut text_start, &mut text_buf, text_end, false);
                let lowered = markup.lower(&mut self.diagnostics.borrow_mut());
                match lowered {
                    markup::Lowered::Imports { imports, span } => {
                        if !self.allow_module || node != self.root || !markup.is_block() {
                            self.diagnostics.borrow_mut().push(Diagnostic::new(
                                Phase::Semantic,
                                span,
                                "imports are only valid at the document top level",
                            ));
                        } else {
                            self.imports.borrow_mut().extend(imports);
                        }
                    }
                    markup::Lowered::Annotation {
                        attrs,
                        module,
                        block,
                        span,
                    } => {
                        join_fragment = false;
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
                                } else {
                                    self.module_attrs.borrow_mut().extend(attrs);
                                }
                            }
                        } else {
                            self.annotations.borrow_mut().push(Annotation {
                                dict: attrs,
                                span,
                                kind: if block {
                                    AnnotationKind::Block
                                } else {
                                    AnnotationKind::Inline
                                },
                            });
                        }
                    }
                    markup::Lowered::Calls { exprs, annotations } => {
                        if markup.is_block() {
                            for (i, expr) in exprs.into_iter().enumerate() {
                                let expr = if i == 0 {
                                    let span = expr.span();
                                    Expr::fragment(vec![expr], span)
                                } else {
                                    expr
                                };
                                append_fragment(
                                    &mut out,
                                    expr,
                                    join_fragment && (i > 0 || !blank_before && !explicit_before),
                                );
                                join_fragment = matches!(out.last(), Some(Expr::Fragment { .. }));
                            }
                        } else {
                            out.extend(exprs);
                        }
                        self.annotations.borrow_mut().extend(annotations);
                        at_line_start = false;
                    }
                }
                child = n.next_sibling();
                continue;
            }
            if let KindData::Text(text) = n.kind_data() {
                if text_start.is_none() {
                    text_start = n.pos();
                }
                let value = text.str(self.src);
                text_buf.push_str(if at_line_start {
                    value.trim_start()
                } else {
                    value
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
                let expr = self.item(c);
                let fragment = matches!(expr, Expr::Fragment { .. });
                append_fragment(
                    &mut out,
                    expr,
                    join_fragment && !blank_before && !explicit_before,
                );
                join_fragment = fragment;
                at_line_start = false;
            }
            child = n.next_sibling();
        }
        flush(&mut out, &mut text_start, &mut text_buf, text_end, true);
        out
    }

    fn item(&self, node: NodeRef) -> Expr {
        let span = self.span_of(node);
        match self.node(node).kind_data() {
            KindData::Paragraph(_) => Expr::fragment(self.children(node), span),
            KindData::Heading(h) => Expr::call("notist.heading", span)
                .with_comptime("level", Expr::Literal(Value::Int(h.level() as i64), span))
                .with_children(self.children(node)),
            KindData::ThematicBreak(_) => Expr::call("notist.divider", span),
            KindData::CodeBlock(b) => {
                let mut expr = Expr::call("notist.raw", span)
                    .with_category(notist_core::definitions::Type::BlockContent)
                    .with_literal_arg(
                        "text",
                        Value::Str(b.value().iter(self.src).collect::<String>()),
                    );
                if let Some(info) = b.info() {
                    let lang = info.str(self.src).trim().to_string();
                    if !lang.is_empty() {
                        expr = expr.with_literal_arg("lang", Value::Str(lang));
                    }
                }
                expr
            }
            KindData::Blockquote(_) => Expr::call("notist.callout", span)
                .with_literal_arg("kind", Value::Str("quote".into()))
                .with_children(self.children(node)),
            KindData::List(l) => Expr::call("notist.list", span)
                .with_literal_arg("ordered", Value::Bool(l.is_ordered()))
                .with_literal_arg(
                    "start",
                    Value::Int(if l.is_ordered() { l.start() as i64 } else { 1 }),
                )
                .with_children(self.children(node)),
            KindData::ListItem(_) => {
                Expr::call("notist.item", span).with_children(self.children(node))
            }
            KindData::CodeSpan(c) => Expr::call("notist.raw", span)
                .with_literal_arg("text", Value::Str(c.str(self.src).into_owned())),
            KindData::Emphasis(_) => {
                Expr::call("notist.emph", span).with_children(self.children(node))
            }
            KindData::Strong(_) => {
                Expr::call("notist.strong", span).with_children(self.children(node))
            }
            KindData::Link(l) => Expr::call("notist.link", span)
                .with_literal_arg(
                    "target",
                    Value::Str(strings::decode(l.destination().str(self.src))),
                )
                .with_children(self.children(node)),
            KindData::Image(i) => {
                let mut expr = Expr::call("notist.embed", span)
                    .with_literal_arg(
                        "target",
                        Value::Str(strings::decode(i.destination().str(self.src))),
                    )
                    .with_literal_arg("description", Value::Str(self.description_of(node)));
                if let Some(title) = i.title_str(self.src) {
                    expr = expr.with_literal_arg("title", Value::Str(strings::decode(&title)));
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
                        rows.push(self.item(r).with_literal_arg("header", Value::Bool(header)));
                        row = self.node(r).next_sibling();
                    }
                    container = self.node(c).next_sibling();
                }
                Expr::call("notist.table", span)
                    .with_literal_arg("align", Value::Array(align))
                    .with_children(rows)
            }
            KindData::TableRow(_) => {
                Expr::call("notist.row", span).with_children(self.children(node))
            }
            KindData::TableCell(_) => {
                let mut inline = self.children(node);
                normalize_table_inline(&mut inline);
                let children = if inline.is_empty() {
                    Vec::new()
                } else {
                    vec![Expr::fragment(inline, span)]
                };
                Expr::call("notist.cell", span).with_children(children)
            }
            KindData::Strikethrough(_) => {
                Expr::call("notist.strike", span).with_children(self.children(node))
            }
            KindData::Extension(data) if data.as_any().is::<math::Math>() => {
                let math = data.as_any().downcast_ref::<math::Math>().unwrap();
                let span = TextRange::new(span.start(), (math.end as u32).into());
                let expr = Expr::call("notist.math", span);
                let expr = if math.block {
                    expr.with_category(notist_core::definitions::Type::BlockContent)
                } else {
                    expr
                };
                expr.with_literal_arg("text", Value::Str(math.text.clone()))
            }
            _ => {
                // TableHeader / TableBody / LinkReferenceDefinition / 其他：子节点透传
                let mut out = Vec::new();
                out.extend(self.children(node));
                Expr::call("notist.node", span).with_children(out)
            }
        }
    }
}
