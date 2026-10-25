use rowan::{NodeOrToken, TextRange, TextSize};

use crate::lowering::{annotation_dict, call_header};
use notist_core::annotation::{Annotation, AnnotationKind};
use notist_core::diag::{Diagnostic, Phase};
use notist_syntax::Parse;
use notist_syntax::code::ast::Annotation as SyntaxAnnotation;
use notist_syntax::code::ast::CodeCall;
use notist_syntax::markup::ast::{Document, Embed, Heading, Link, List, ListItem, Table, WikiLink};
use notist_syntax::tree::{SyntaxKind, SyntaxNode, SyntaxToken};

use notist_core::expr::{BodyFlavor, Expr};
use notist_core::item::{Dict, Value};

fn tokens_text(tokens: &[SyntaxToken]) -> String {
    tokens
        .iter()
        .map(|t| t.text())
        .collect::<String>()
        .trim()
        .to_string()
}

/// Document AST → Expr forest (IR₁). A document is a module body (a
/// top-level expression sequence), not a constructor call; the `Doc` root
/// is introduced by materialize.
///
/// The CST is flat: blank lines and explicit block syntax delimit content
/// fragments; function result categories are checked later. Node annotations are
/// collected beside the forest and attached after shaping.
pub fn lower_document(
    parsed: &Parse,
    diags: &mut Vec<Diagnostic>,
) -> (Vec<Expr>, Dict, Vec<Annotation>) {
    let document = Document::cast(parsed.syntax()).expect("native content has a Document root");
    let elements: Vec<_> = document.elements().collect();
    let mut annotations = Vec::new();
    let (forest, module_attrs) = lower_blocks(&elements, parsed, diags, &mut annotations);
    (forest, module_attrs, annotations)
}

pub(crate) fn lower_content(
    parsed: &Parse,
    flavor: notist_syntax::content::ContentFlavor,
    diags: &mut Vec<Diagnostic>,
) -> crate::lowering::content::Lowered {
    let document = Document::cast(parsed.syntax()).expect("native content has a Document root");
    let elements: Vec<_> = document.elements().collect();
    let mut annotations = Vec::new();
    let forest = match flavor {
        notist_syntax::content::ContentFlavor::Inline => {
            lower_inline(elements.into_iter(), parsed, diags, &mut annotations)
        }
        notist_syntax::content::ContentFlavor::Block => {
            lower_blocks(&elements, parsed, diags, &mut annotations).0
        }
    };
    for node in document.syntax().descendants() {
        if node.kind() == SyntaxKind::ImportDecl {
            diags.push(Diagnostic::new(
                Phase::Semantic,
                node.text_range(),
                "imports are not valid in content literals",
            ));
        }
        if let Some(annotation) = SyntaxAnnotation::cast(node)
            && annotation.is_module()
            && !diags
                .iter()
                .any(|d| d.span == annotation.range() && d.message.starts_with("module annotation"))
        {
            diags.push(Diagnostic::new(
                Phase::Semantic,
                annotation.range(),
                "module annotations are not valid in content literals",
            ));
        }
    }
    crate::lowering::content::Lowered {
        forest,
        annotations,
    }
}

/// A flat element sequence → Expr forest, shared by the document body and
/// block-level `[...]` bodies. The returned Dict collects module (`@!`)
/// attrs; `[...]` bodies ignore it.
#[allow(unused_assignments)] // seen_content is loop-carried across match arms
fn lower_blocks(
    elements: &[NodeOrToken<SyntaxNode, SyntaxToken>],
    parsed: &Parse,
    diags: &mut Vec<Diagnostic>,
    annotations: &mut Vec<Annotation>,
) -> (Vec<Expr>, Dict) {
    let mut forest = Vec::new();
    let mut module_attrs = Dict::default();
    let mut seen_content = false;
    let mut run: Vec<NodeOrToken<SyntaxNode, SyntaxToken>> = Vec::new();

    macro_rules! flush_run {
        () => {
            if !run.is_empty() {
                let span = run_span(&run);
                let children = lower_inline(run.drain(..), parsed, diags, annotations);
                if !children.is_empty() {
                    seen_content = true;
                    forest.push(Expr::fragment(children, span));
                }
            }
        };
    }

    for i in 0..elements.len() {
        let el = &elements[i];
        match el {
            NodeOrToken::Node(node) => match node.kind() {
                SyntaxKind::ImportDecl => {}
                SyntaxKind::Annotation | SyntaxKind::BlockAnnotation => {
                    let annotation = SyntaxAnnotation::cast(node.clone()).unwrap();
                    let block = annotation.is_block();
                    let dict = annotation_dict(&annotation, diags);
                    if annotation.is_module() {
                        if !block {
                            diags.push(Diagnostic {
                                source: None,
                                code: None,
                                recovered: false,
                                phase: Phase::Semantic,
                                span: annotation.range(),
                                message: "module annotation is only valid at the document top"
                                    .to_string(),
                            });
                            continue;
                        }
                        flush_run!();
                        if seen_content {
                            diags.push(Diagnostic {
                                source: None,
                                code: None,
                                recovered: false,
                                phase: Phase::Semantic,
                                span: annotation.range(),
                                message: "module annotation must precede all content".to_string(),
                            });
                        } else {
                            module_attrs.extend(dict);
                        }
                    } else {
                        if block {
                            flush_run!();
                        }
                        annotations.push(Annotation {
                            dict,
                            span: node.text_range(),
                            kind: if block {
                                AnnotationKind::Block
                            } else {
                                AnnotationKind::Inline
                            },
                        });
                    }
                }
                SyntaxKind::Heading => {
                    flush_run!();
                    seen_content = true;
                    let heading = Heading::cast(node.clone()).unwrap();
                    let level = heading.level() as i64;
                    let children = lower_inline(heading.content(), parsed, diags, annotations);
                    forest.push(
                        Expr::call("notist.heading", heading.range())
                            .with_comptime(
                                "level",
                                Expr::Literal(Value::Int(level), node.text_range()),
                            )
                            .with_children(children),
                    );
                }
                SyntaxKind::List => {
                    flush_run!();
                    seen_content = true;
                    forest.push(lower_list(
                        &List::cast(node.clone()).unwrap(),
                        parsed,
                        diags,
                        annotations,
                    ));
                }
                SyntaxKind::Table => {
                    flush_run!();
                    seen_content = true;
                    forest.push(lower_table(
                        &Table::cast(node.clone()).unwrap(),
                        parsed,
                        diags,
                        annotations,
                    ));
                }
                SyntaxKind::Raw => {
                    flush_run!();
                    seen_content = true;
                    forest.push(lower_raw_block(node));
                }
                SyntaxKind::Divider => {
                    flush_run!();
                    seen_content = true;
                    forest.push(Expr::call("notist.divider", node.text_range()));
                }
                // boundaries with no core counterpart (ParBreak) or already
                // diagnosed (Error)
                SyntaxKind::ParBreak | SyntaxKind::Error => flush_run!(),
                _ => run.push(el.clone()),
            },
            NodeOrToken::Token(token) => {
                if token.kind() == SyntaxKind::Newline && is_blank_boundary(elements, i) {
                    flush_run!();
                } else {
                    run.push(el.clone());
                }
            }
        }
    }
    flush_run!();
    (forest, module_attrs)
}

/// Whether the newline at `i` ends a run: what follows (whitespace aside) is
/// another newline or nothing.
fn is_blank_boundary(elements: &[NodeOrToken<SyntaxNode, SyntaxToken>], i: usize) -> bool {
    let mut j = i + 1;
    while matches!(
        elements.get(j).map(|e| e.kind()),
        Some(SyntaxKind::Whitespace)
    ) {
        j += 1;
    }
    matches!(
        elements.get(j).map(|e| e.kind()),
        None | Some(SyntaxKind::Newline)
    )
}

/// The span covering a run's content (surrounding trivia excluded).
fn run_span(run: &[NodeOrToken<SyntaxNode, SyntaxToken>]) -> TextRange {
    let is_trivia = |e: &NodeOrToken<SyntaxNode, SyntaxToken>| {
        matches!(
            e.kind(),
            SyntaxKind::Whitespace
                | SyntaxKind::Newline
                | SyntaxKind::LineComment
                | SyntaxKind::BlockComment
        )
    };
    let start = run
        .iter()
        .find(|e| !is_trivia(e))
        .map(|e| e.text_range().start())
        .unwrap_or_default();
    let end = run
        .iter()
        .rev()
        .find(|e| !is_trivia(e))
        .map(|e| e.text_range().end())
        .unwrap_or(start);
    TextRange::new(start, end)
}

/// A fenced raw block: content between the fences; the tag is the rest of
/// the opening line. (Unclosed fences arrive wrapped in `Error` and are
/// dropped there.)
fn lower_raw_block(node: &SyntaxNode) -> Expr {
    let tokens: Vec<_> = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .collect();
    let nl = tokens.iter().position(|t| t.kind() == SyntaxKind::Newline);
    let close = tokens
        .iter()
        .rposition(|t| t.kind() == SyntaxKind::Backtick)
        .unwrap_or(tokens.len());
    let (tag, text) = match nl {
        Some(nl) => {
            let tag = tokens[1..nl]
                .iter()
                .map(|t| t.text())
                .collect::<String>()
                .trim()
                .to_string();
            let text: String = tokens[nl + 1..close]
                .iter()
                .map(|t| t.text())
                .collect::<String>();
            // Closing-fence indentation is framing; retain all payload line endings.
            let payload_end = text.rfind(['\n', '\r']).map_or(0, |i| i + 1);
            let indent = node
                .ancestors()
                .find_map(ListItem::cast)
                .map_or(0, |item| item.content_indent());
            let text = text[..payload_end]
                .split_inclusive('\n')
                .map(|line| strip_indent(line, indent))
                .collect::<String>();
            (tag, text)
        }
        None => (String::new(), String::new()),
    };
    let mut expr = Expr::call("notist.raw", node.text_range())
        .with_category(notist_core::definitions::Type::BlockContent)
        .with_literal_arg("text", Value::Str(text));
    if !tag.is_empty() {
        expr = expr.with_literal_arg("lang", Value::Str(tag));
    }
    expr
}

fn lower_list(
    list: &List,
    parsed: &Parse,
    diags: &mut Vec<Diagnostic>,
    annotations: &mut Vec<Annotation>,
) -> Expr {
    let ordered = list.items().next().and_then(|item| item.marker()) == Some(SyntaxKind::Plus);
    Expr::call("notist.list", list.range())
        .with_literal_arg("ordered", Value::Bool(ordered))
        .with_literal_arg("start", Value::Int(1))
        .with_children(
            list.items()
                .map(|item| lower_list_item(&item, parsed, diags, annotations))
                .collect(),
        )
}

fn lower_table(
    table: &Table,
    parsed: &Parse,
    diags: &mut Vec<Diagnostic>,
    annotations: &mut Vec<Annotation>,
) -> Expr {
    let align = table.alignments();
    let width = align.len();
    let rows = table
        .rows()
        .map(|row| {
            let mut cells: Vec<_> = row
                .cells()
                .take(width)
                .map(|cell| {
                    let inline = lower_inline(cell.content(), parsed, diags, annotations);
                    let children = if inline.is_empty() {
                        Vec::new()
                    } else {
                        vec![Expr::fragment(inline, cell.range())]
                    };
                    Expr::call("notist.cell", cell.range()).with_children(children)
                })
                .collect();
            while cells.len() < width {
                cells.push(Expr::call(
                    "notist.cell",
                    TextRange::empty(row.range().end()),
                ));
            }
            Expr::call("notist.row", row.range())
                .with_literal_arg("header", Value::Bool(row.is_header()))
                .with_children(cells)
        })
        .collect();
    Expr::call("notist.table", table.range())
        .with_literal_arg(
            "align",
            Value::Array(
                align
                    .into_iter()
                    .map(|a| Value::Str(a.as_str().into()))
                    .collect(),
            ),
        )
        .with_children(rows)
}

fn lower_list_item(
    item: &ListItem,
    parsed: &Parse,
    diags: &mut Vec<Diagnostic>,
    annotations: &mut Vec<Annotation>,
) -> Expr {
    let elements: Vec<_> = item.content().collect();
    let (children, _) = lower_blocks(&elements, parsed, diags, annotations);
    Expr::call("notist.item", item.range()).with_children(children)
}

fn lower_code_call(
    call: &CodeCall,
    parsed: &Parse,
    diags: &mut Vec<Diagnostic>,
    annotations: &mut Vec<Annotation>,
) -> Vec<Expr> {
    let mut expr = call_header(call, call.has_body(), parsed, &super::NotistFrontend, diags);
    let (content, flavor) = call_body(call, parsed, diags, annotations);
    if let Expr::Call { children, body, .. } = &mut expr {
        *children = content;
        *body = flavor;
    }
    vec![expr]
}

/// Flavor of a bracketed content region, from the declared flanks: both
/// brackets padded on the inside → block.
fn is_block_content(elements: &[NodeOrToken<SyntaxNode, SyntaxToken>]) -> bool {
    let flank = |el: Option<&NodeOrToken<SyntaxNode, SyntaxToken>>| {
        matches!(
            el.map(|e| e.kind()),
            Some(SyntaxKind::Whitespace | SyntaxKind::Newline)
        )
    };
    flank(elements.first()) && flank(elements.last())
}

/// Desugar a bracketed content region with its derived flavor.
fn content_children(
    elements: &[NodeOrToken<SyntaxNode, SyntaxToken>],
    parsed: &Parse,
    diags: &mut Vec<Diagnostic>,
    annotations: &mut Vec<Annotation>,
) -> Vec<Expr> {
    if is_block_content(elements) {
        lower_blocks(elements, parsed, diags, annotations).0
    } else {
        lower_inline(elements.iter().cloned(), parsed, diags, annotations)
    }
}

/// The `[..]` body of a call as desugared children plus its derived flavor.
fn call_body(
    call: &CodeCall,
    parsed: &Parse,
    diags: &mut Vec<Diagnostic>,
    annotations: &mut Vec<Annotation>,
) -> (Vec<Expr>, BodyFlavor) {
    if !call.has_body() {
        return (Vec::new(), BodyFlavor::None);
    }
    let elements = call.body();
    let flavor = if is_block_content(&elements) {
        BodyFlavor::Block
    } else {
        BodyFlavor::Inline
    };
    (
        content_children(&elements, parsed, diags, annotations),
        flavor,
    )
}

/// The accumulating state of a text run inside an inline sequence.
/// Whitespace handling is positional: leading whitespace of a line or
/// sequence never enters the buffer; whitespace adjacent to an inline
/// element is content and stays; trailing whitespace at a line or
/// sequence boundary is dropped.
struct TextRun {
    buf: String,
    start: Option<TextSize>,
    content_len: usize,
    content_end: Option<TextSize>,
    last_end: Option<TextSize>,
}

impl TextRun {
    fn new() -> Self {
        Self {
            buf: String::new(),
            start: None,
            content_len: 0,
            content_end: None,
            last_end: None,
        }
    }

    fn push_whitespace(&mut self, text: &str, range: TextRange, after_element: bool) {
        if self.start.is_none() && !after_element {
            return;
        }
        if self.start.is_none() {
            self.start = Some(range.start());
        }
        self.buf.push_str(text);
        self.last_end = Some(range.end());
    }

    fn push_content(&mut self, text: &str, range: TextRange) {
        if self.start.is_none() {
            self.start = Some(range.start());
        }
        self.buf.push_str(text);
        self.content_len = self.buf.len();
        self.content_end = Some(range.end());
        self.last_end = Some(range.end());
    }

    /// Flush at an inline element: boundary whitespace is content, kept.
    fn flush_at_element(&mut self, items: &mut Vec<Expr>) {
        if let (Some(s), Some(e)) = (self.start, self.last_end)
            && !self.buf.is_empty()
        {
            items.push(Expr::text(
                std::mem::take(&mut self.buf),
                TextRange::new(s, e),
            ));
        }
        self.reset();
    }

    /// Flush at a line or sequence boundary: trailing whitespace dropped.
    fn flush_at_boundary(&mut self, items: &mut Vec<Expr>) {
        if let (Some(s), Some(e)) = (self.start, self.content_end) {
            let text = &self.buf[..self.content_len];
            if !text.is_empty() {
                items.push(Expr::text(text.to_string(), TextRange::new(s, e)));
            }
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.start = None;
        self.content_len = 0;
        self.content_end = None;
        self.last_end = None;
    }
}

fn append_description(expr: &Expr, out: &mut String) {
    match expr {
        Expr::Literal(value, _) => out.push_str(&value.to_string()),
        Expr::Ref { .. } | Expr::Member { .. } => {}
        Expr::Content { forest, .. }
        | Expr::Fragment {
            children: forest, ..
        } => {
            for e in forest {
                append_description(e, out);
            }
        }
        Expr::Block { value, .. } | Expr::Unary { value, .. } | Expr::Comptime { value, .. } => {
            append_description(value, out)
        }
        Expr::If { .. }
        | Expr::Match { .. }
        | Expr::Binary { .. }
        | Expr::Array(..)
        | Expr::Dict(..) => {}
        Expr::Call { children, .. } => {
            if let Some(Value::Str(text)) = expr
                .named_arg("text")
                .or_else(|| expr.named_arg("description"))
                .and_then(Expr::literal_value)
            {
                out.push_str(text);
            }
            for child in children {
                append_description(child, out);
            }
        }
    }
}

fn strip_indent(line: &str, indent: usize) -> String {
    let mut column = 0;
    let mut end = 0;
    for (offset, ch) in line.char_indices() {
        if column >= indent || !matches!(ch, ' ' | '\t') {
            break;
        }
        column = if ch == '\t' {
            (column / 4 + 1) * 4
        } else {
            column + 1
        };
        end = offset + ch.len_utf8();
    }
    format!(
        "{}{}",
        " ".repeat(column.saturating_sub(indent)),
        &line[end..]
    )
}

fn lower_inline(
    elements: impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>>,
    parsed: &Parse,
    diags: &mut Vec<Diagnostic>,
    annotations: &mut Vec<Annotation>,
) -> Vec<Expr> {
    let mut items = Vec::new();
    let mut run = TextRun::new();
    let mut after_element = false;

    for element in elements {
        match element {
            NodeOrToken::Token(token) => match token.kind() {
                SyntaxKind::Newline => {
                    run.flush_at_boundary(&mut items);
                    after_element = false;
                }
                SyntaxKind::LineComment | SyntaxKind::BlockComment => {}
                SyntaxKind::Whitespace => {
                    run.push_whitespace(token.text(), token.text_range(), after_element)
                }
                SyntaxKind::Escape => run.push_content(&token.text()[1..], token.text_range()),
                _ => run.push_content(token.text(), token.text_range()),
            },
            NodeOrToken::Node(node) => match node.kind() {
                SyntaxKind::ImportDecl => {}
                SyntaxKind::Annotation | SyntaxKind::BlockAnnotation => {
                    let annotation = SyntaxAnnotation::cast(node.clone()).unwrap();
                    if annotation.is_module() {
                        diags.push(Diagnostic {
                            source: None,
                            code: None,
                            recovered: false,
                            phase: Phase::Semantic,
                            span: node.text_range(),
                            message: "module annotation is only valid at the document top"
                                .to_string(),
                        });
                    } else {
                        annotations.push(Annotation {
                            dict: annotation_dict(&annotation, diags),
                            span: node.text_range(),
                            kind: AnnotationKind::Inline,
                        });
                    }
                }
                SyntaxKind::RawInline => {
                    run.flush_at_element(&mut items);
                    let tokens: Vec<_> = node
                        .children_with_tokens()
                        .filter_map(|e| e.into_token())
                        .collect();
                    let text: String = tokens[1..tokens.len() - 1]
                        .iter()
                        .map(|t| t.text())
                        .collect();
                    let expr = Expr::call("notist.raw", node.text_range())
                        .with_literal_arg("text", Value::Str(text));
                    items.push(expr);
                    after_element = true;
                }
                SyntaxKind::Math => {
                    run.flush_at_element(&mut items);
                    let tokens: Vec<_> = node
                        .children_with_tokens()
                        .filter_map(|e| e.into_token())
                        .collect();
                    let text: String = tokens[1..tokens.len() - 1]
                        .iter()
                        .map(|t| t.text())
                        .collect();
                    let block = matches!(
                        tokens[1].kind(),
                        SyntaxKind::Whitespace | SyntaxKind::Newline
                    );
                    let indent = node
                        .ancestors()
                        .find_map(ListItem::cast)
                        .map_or(0, |item| item.content_indent());
                    let text = text
                        .split_inclusive(['\r', '\n'])
                        .enumerate()
                        .map(|(i, line)| {
                            if i == 0 {
                                line.to_string()
                            } else {
                                strip_indent(line, indent)
                            }
                        })
                        .collect::<String>();
                    let text = if block { text.trim().to_owned() } else { text };
                    let mut expr = Expr::call("notist.math", node.text_range());
                    if block {
                        expr = expr.with_category(notist_core::definitions::Type::BlockContent);
                    }
                    let expr = expr.with_literal_arg("text", Value::Str(text));
                    items.push(expr);
                    after_element = true;
                }
                SyntaxKind::Strong | SyntaxKind::Emph | SyntaxKind::Strike => {
                    run.flush_at_element(&mut items);
                    let (name, delim) = match node.kind() {
                        SyntaxKind::Strong => ("strong", SyntaxKind::Star),
                        SyntaxKind::Emph => ("emph", SyntaxKind::Underscore),
                        SyntaxKind::Strike => ("strike", SyntaxKind::Tilde),
                        _ => unreachable!(),
                    };
                    let elements: Vec<_> = node.children_with_tokens().collect();
                    let end = if elements.last().is_some_and(|el| el.kind() == delim) {
                        elements.len() - 1
                    } else {
                        elements.len()
                    };
                    let children =
                        lower_inline(elements[1..end].iter().cloned(), parsed, diags, annotations);
                    let expr = Expr::call(&format!("notist.{name}"), node.text_range())
                        .with_children(children);
                    items.push(expr);
                    after_element = true;
                }
                SyntaxKind::Embed => {
                    run.flush_at_element(&mut items);
                    let embed = Embed::cast(node.clone()).unwrap();
                    let (target, title) = embed.destination();
                    let content = lower_inline(embed.content(), parsed, diags, annotations);
                    let mut description = String::new();
                    for expr in &content {
                        append_description(expr, &mut description);
                    }
                    let mut expr = Expr::call("notist.embed", embed.range())
                        .with_literal_arg("target", Value::Str(target))
                        .with_literal_arg("description", Value::Str(description));
                    if let Some(title) = title {
                        expr = expr.with_literal_arg("title", Value::Str(title));
                    }
                    items.push(expr);
                    after_element = true;
                }
                SyntaxKind::Link | SyntaxKind::WikiLink => {
                    run.flush_at_element(&mut items);
                    let (target, children) = if node.kind() == SyntaxKind::Link {
                        let link = Link::cast(node.clone()).unwrap();
                        (
                            link.target(),
                            lower_inline(link.content(), parsed, diags, annotations),
                        )
                    } else {
                        let link = WikiLink::cast(node.clone()).unwrap();
                        (
                            tokens_text(&link.target_tokens()),
                            lower_inline(link.content(), parsed, diags, annotations),
                        )
                    };
                    let expr = Expr::call("notist.link", node.text_range())
                        .with_literal_arg("target", Value::Str(target))
                        .with_children(children);
                    items.push(expr);
                    after_element = true;
                }
                SyntaxKind::CodeCall => {
                    run.flush_at_element(&mut items);
                    items.extend(lower_code_call(
                        &CodeCall::cast(node.clone()).unwrap(),
                        parsed,
                        diags,
                        annotations,
                    ));
                    after_element = true;
                }
                _ => {}
            },
        }
    }
    run.flush_at_boundary(&mut items);
    items
}
