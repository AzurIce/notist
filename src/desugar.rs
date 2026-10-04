use rowan::{NodeOrToken, TextRange, TextSize};

use crate::diag::{Diagnostic, Phase};
use crate::literals::{key_text, syntax_value, value_children};
use notist_syntax::ast::{
    Annotation, CodeCall, Document, Embed, Entry, Heading, Link, List, ListItem, Table, WikiLink,
};
use notist_syntax::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::expr::{BodyFlavor, Expr};
use crate::item::{Dict, Value};

/// The `.not` frontend's lowering: parse + desugar, no materialization (that is shared).
pub fn lower_not(src: &str) -> (Vec<Expr>, Dict, Vec<Diagnostic>) {
    let parse = notist_syntax::parse_document(src);
    let mut diagnostics: Vec<Diagnostic> = parse
        .diagnostics
        .iter()
        .map(|d| Diagnostic::new(Phase::Syntax, d.span, d.message.clone()))
        .collect();
    let Some(document) = notist_syntax::ast::Document::cast(parse.syntax()) else {
        return (Vec::new(), Dict::default(), diagnostics);
    };
    let (forest, module_attrs) = desugar(&document, &mut diagnostics);
    (forest, module_attrs, diagnostics)
}

fn tokens_text(tokens: &[SyntaxToken]) -> String {
    tokens
        .iter()
        .map(|t| t.text())
        .collect::<String>()
        .trim()
        .to_string()
}

/// CST → Expr forest: the desugar step. A document is a module body (a
/// top-level expression sequence), not a constructor call; the `Doc` root
/// is introduced by materialize.
///
/// The CST is flat: maximal runs of inline content become `paragraph` exprs
/// here; blank lines and block nodes are run boundaries. Annotations attach
/// to the immediately following node; `@!(dict)` become the module's attrs.
pub fn desugar(document: &Document, diags: &mut Vec<Diagnostic>) -> (Vec<Expr>, Dict) {
    let elements: Vec<_> = document.elements().collect();
    desugar_blocks(&elements, diags)
}

/// A flat element sequence → Expr forest, shared by the document body and
/// block-level `[...]` bodies. The returned Dict collects module (`@!`)
/// attrs; `[...]` bodies ignore it.
#[allow(unused_assignments)] // seen_content is loop-carried across match arms
fn desugar_blocks(
    elements: &[NodeOrToken<SyntaxNode, SyntaxToken>],
    diags: &mut Vec<Diagnostic>,
) -> (Vec<Expr>, Dict) {
    let mut forest = Vec::new();
    let mut pending = Dict::default();
    let mut module_attrs = Dict::default();
    let mut seen_content = false;
    let mut run: Vec<NodeOrToken<SyntaxNode, SyntaxToken>> = Vec::new();

    macro_rules! flush_run {
        () => {
            if !run.is_empty() {
                let span = run_span(&run);
                let children = desugar_inline(run.drain(..), diags);
                if !children.is_empty() {
                    let mut expr = Expr::call("paragraph", span).with_children(children);
                    expr.set_attrs(pending.take());
                    seen_content = true;
                    forest.push(expr);
                }
            }
        };
    }

    for i in 0..elements.len() {
        let el = &elements[i];
        match el {
            NodeOrToken::Node(node) => match node.kind() {
                SyntaxKind::Annotation => {
                    let annotation = Annotation::cast(node.clone()).unwrap();
                    // 紧邻下一个元素的注解属于行内（进 run，由 desugar_inline
                    // 挂到紧随的构造）；否则是块级注解（挂到下一个块/段落）
                    let adjacent = elements.get(i + 1).is_some_and(|next| {
                        !matches!(next.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline)
                    });
                    if adjacent && !annotation.is_module() {
                        run.push(el.clone());
                    } else {
                        flush_run!();
                        let dict = annotation_dict(&annotation, diags);
                        if annotation.is_module() {
                            if seen_content {
                                diags.push(Diagnostic {
                                    phase: Phase::Semantic,
                                    span: annotation.range(),
                                    message: "module annotation must precede all content"
                                        .to_string(),
                                });
                            }
                            module_attrs.extend(dict);
                        } else {
                            pending.extend(dict);
                        }
                    }
                }
                SyntaxKind::Heading => {
                    flush_run!();
                    seen_content = true;
                    let heading = Heading::cast(node.clone()).unwrap();
                    let level = heading.level() as i64;
                    let children = desugar_inline(heading.content(), diags);
                    let mut expr = Expr::call("heading", heading.range())
                        .with_field("level", Value::Int(level))
                        .with_children(children);
                    expr.set_attrs(pending.take());
                    forest.push(expr);
                }
                SyntaxKind::List => {
                    flush_run!();
                    seen_content = true;
                    let mut expr = desugar_list(&List::cast(node.clone()).unwrap(), diags);
                    expr.set_attrs(pending.take());
                    forest.push(expr);
                }
                SyntaxKind::Table => {
                    flush_run!();
                    seen_content = true;
                    let mut expr = desugar_table(&Table::cast(node.clone()).unwrap(), diags);
                    expr.set_attrs(pending.take());
                    forest.push(expr);
                }
                SyntaxKind::Raw => {
                    flush_run!();
                    seen_content = true;
                    let mut expr = desugar_raw_block(node);
                    expr.set_attrs(pending.take());
                    forest.push(expr);
                }
                SyntaxKind::Divider => {
                    flush_run!();
                    seen_content = true;
                    let mut expr = Expr::call("divider", node.text_range());
                    expr.set_attrs(pending.take());
                    forest.push(expr);
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
fn desugar_raw_block(node: &SyntaxNode) -> Expr {
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
    let mut expr = Expr::call("raw", node.text_range())
        .with_field("block", Value::Bool(true))
        .with_field("text", Value::Str(text));
    if !tag.is_empty() {
        expr = expr.with_field("lang", Value::Str(tag));
    }
    expr
}

/// The dict carried by an `@(dict)` annotation (stray members diagnosed).
fn annotation_dict(annotation: &Annotation, diags: &mut Vec<Diagnostic>) -> Dict {
    let mut dict = Dict::default();
    if let Some(node) = annotation.payload_dict() {
        for el in value_children(&node) {
            // the colon of the empty-dict spelling `(:)` is structural
            if !matches!(el.kind(), SyntaxKind::Entry | SyntaxKind::Colon) {
                diags.push(Diagnostic {
                    phase: Phase::Semantic,
                    span: el.text_range(),
                    message: "annotation entries must be `key: value`".to_string(),
                });
            }
        }
        if let Some(Value::Dict(d)) = syntax_value(&NodeOrToken::Node(node), diags) {
            dict = d;
        }
    }
    dict
}

fn desugar_list(list: &List, diags: &mut Vec<Diagnostic>) -> Expr {
    let ordered = list.items().next().and_then(|item| item.marker()) == Some(SyntaxKind::Plus);
    Expr::call("list", list.range())
        .with_field("ordered", Value::Bool(ordered))
        .with_field("start", Value::Int(1))
        .with_children(
            list.items()
                .map(|item| desugar_list_item(&item, diags))
                .collect(),
        )
}

fn desugar_table(table: &Table, diags: &mut Vec<Diagnostic>) -> Expr {
    let align = table.alignments();
    let width = align.len();
    let rows = table
        .rows()
        .map(|row| {
            let mut cells: Vec<_> = row
                .cells()
                .take(width)
                .map(|cell| {
                    let payloads: Vec<_> = cell
                        .content()
                        .filter_map(|el| el.into_node())
                        .flat_map(|node| node.descendants())
                        .filter(|node| {
                            matches!(node.kind(), SyntaxKind::RawInline | SyntaxKind::Math)
                        })
                        .map(|node| node.text_range())
                        .collect();
                    let mut inline = desugar_inline(cell.content(), diags);
                    // Pipe escaping belongs to table syntax, including opaque raw/math
                    // payloads, where ordinary markup escapes otherwise stay literal.
                    unescape_table_pipes(&mut inline, &payloads);
                    let children = if inline.is_empty() {
                        Vec::new()
                    } else {
                        vec![Expr::call("paragraph", cell.range()).with_children(inline)]
                    };
                    Expr::call("cell", cell.range()).with_children(children)
                })
                .collect();
            while cells.len() < width {
                cells.push(Expr::call("cell", TextRange::empty(row.range().end())));
            }
            Expr::call("row", row.range())
                .with_field("header", Value::Bool(row.is_header()))
                .with_children(cells)
        })
        .collect();
    Expr::call("table", table.range())
        .with_field(
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

fn unescape_table_pipes(exprs: &mut [Expr], payloads: &[TextRange]) {
    for expr in exprs {
        if let Expr::Call {
            name,
            fields,
            children,
            span,
            ..
        } = expr
        {
            if matches!(name.as_str(), "raw" | "math") && payloads.contains(span) {
                if let Some(Value::Str(text)) = fields.get("text") {
                    fields.insert("text", Value::Str(text.replace("\\|", "|")));
                }
            }
            unescape_table_pipes(children, payloads);
        }
    }
}

fn desugar_list_item(item: &ListItem, diags: &mut Vec<Diagnostic>) -> Expr {
    let elements: Vec<_> = item.content().collect();
    let (children, _) = desugar_blocks(&elements, diags);
    Expr::call("item", item.range()).with_children(children)
}

fn desugar_code_call(call: &CodeCall, diags: &mut Vec<Diagnostic>) -> Vec<Expr> {
    let span = call.range();
    let mut args = Vec::new();
    let mut fields = Dict::default();
    let arg_els = call.args();
    let mut i = 0;
    while i < arg_els.len() {
        match &arg_els[i] {
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::Entry => {
                let entry = Entry::cast(n.clone()).unwrap();
                let Some(key_token) = entry.key_token() else {
                    i += 1;
                    continue;
                };
                let Some(key) = key_text(&key_token, diags) else {
                    i += 1;
                    continue;
                };
                let Some(value_el) = entry.value() else {
                    i += 1;
                    continue;
                };
                if value_el.kind() == SyntaxKind::LBracket {
                    diags.push(Diagnostic {
                        phase: Phase::Semantic,
                        span: value_el.text_range(),
                        message: "content literals as entry values are not supported yet"
                            .to_string(),
                    });
                    i += 1;
                    continue;
                }
                if let Some(value) = syntax_value(&value_el, diags) {
                    fields.insert(key, value);
                }
                i += 1;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::LBracket => {
                // content is mounted via the body slot, never passed as an argument
                let open_span = t.text_range();
                diags.push(Diagnostic {
                    phase: Phase::Semantic,
                    span: open_span,
                    message:
                        "content is mounted with `[..]` after the call, not passed as an argument"
                            .to_string(),
                });
                let mut depth = 0usize;
                i += 1;
                while i < arg_els.len() {
                    match arg_els[i].kind() {
                        SyntaxKind::LBracket => depth += 1,
                        SyntaxKind::RBracket => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
            }
            other => {
                if let Some(value) = syntax_value(other, diags) {
                    args.push(Expr::Literal(value, span));
                }
                i += 1;
            }
        }
    }
    let (children, flavor) = call_body(call, diags);
    // `#[..]` is the anonymous constructor call: a transparent group node
    let name = call.name().unwrap_or_else(|| "group".to_string());
    vec![Expr::Call {
        name,
        args,
        fields,
        children,
        body: flavor,
        attrs: Dict::default(),
        span,
    }]
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
    diags: &mut Vec<Diagnostic>,
) -> Vec<Expr> {
    if is_block_content(elements) {
        desugar_blocks(elements, diags).0
    } else {
        desugar_inline(elements.iter().cloned(), diags)
    }
}

/// The `[..]` body of a call as desugared children plus its derived flavor.
fn call_body(call: &CodeCall, diags: &mut Vec<Diagnostic>) -> (Vec<Expr>, BodyFlavor) {
    if !call.has_body() {
        return (Vec::new(), BodyFlavor::None);
    }
    let elements = call.body();
    let flavor = if is_block_content(&elements) {
        BodyFlavor::Block
    } else {
        BodyFlavor::Inline
    };
    (content_children(&elements, diags), flavor)
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
        if let (Some(s), Some(e)) = (self.start, self.last_end) {
            if !self.buf.is_empty() {
                items.push(Expr::text(
                    std::mem::take(&mut self.buf),
                    TextRange::new(s, e),
                ));
            }
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
        Expr::Call {
            fields, children, ..
        } => {
            if let Some(Value::Str(text)) = fields.get("text").or_else(|| fields.get("description"))
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

fn desugar_inline(
    elements: impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>>,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Expr> {
    let mut items = Vec::new();
    let mut run = TextRun::new();
    let mut after_element = false;
    let mut pending = Dict::default();
    let mut pending_range: Option<TextRange> = None;

    for element in elements {
        // an inline annotation must be immediately followed by an element
        if !pending.is_empty() {
            let followed = match &element {
                NodeOrToken::Node(n) => matches!(
                    n.kind(),
                    SyntaxKind::Strong
                        | SyntaxKind::Emph
                        | SyntaxKind::Strike
                        | SyntaxKind::RawInline
                        | SyntaxKind::Math
                        | SyntaxKind::Link
                        | SyntaxKind::Embed
                        | SyntaxKind::WikiLink
                        | SyntaxKind::CodeCall
                        | SyntaxKind::Annotation
                ),
                NodeOrToken::Token(_) => false,
            };
            if !followed {
                diags.push(Diagnostic {
                    phase: Phase::Semantic,
                    span: element.text_range(),
                    message: "annotation must be immediately followed by an element".to_string(),
                });
                pending = Dict::default();
            }
        }
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
                SyntaxKind::Annotation => {
                    let annotation = Annotation::cast(node.clone()).unwrap();
                    if annotation.is_module() {
                        diags.push(Diagnostic {
                            phase: Phase::Semantic,
                            span: node.text_range(),
                            message: "module annotation is only valid at the document top"
                                .to_string(),
                        });
                    }
                    pending.extend(annotation_dict(&annotation, diags));
                    pending_range = Some(node.text_range());
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
                    let mut expr =
                        Expr::call("raw", node.text_range()).with_field("text", Value::Str(text));
                    expr.set_attrs(pending.take());
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
                    let indent = node
                        .ancestors()
                        .find_map(ListItem::cast)
                        .map_or(0, |item| item.content_indent());
                    let text = text
                        .split_inclusive('\n')
                        .enumerate()
                        .map(|(i, line)| {
                            if i == 0 {
                                line.to_string()
                            } else {
                                strip_indent(line, indent)
                            }
                        })
                        .collect::<String>();
                    let mut expr =
                        Expr::call("math", node.text_range()).with_field("text", Value::Str(text));
                    expr.set_attrs(pending.take());
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
                    let children = desugar_inline(elements[1..end].iter().cloned(), diags);
                    let mut expr = Expr::call(name, node.text_range()).with_children(children);
                    expr.set_attrs(pending.take());
                    items.push(expr);
                    after_element = true;
                }
                SyntaxKind::Embed => {
                    run.flush_at_element(&mut items);
                    let embed = Embed::cast(node.clone()).unwrap();
                    let (target, title) = embed.destination();
                    let content = desugar_inline(embed.content(), diags);
                    let mut description = String::new();
                    for expr in &content {
                        append_description(expr, &mut description);
                    }
                    let mut expr = Expr::call("embed", embed.range())
                        .with_field("target", Value::Str(target))
                        .with_field("description", Value::Str(description));
                    if let Some(title) = title {
                        expr = expr.with_field("title", Value::Str(title));
                    }
                    expr.set_attrs(pending.take());
                    items.push(expr);
                    after_element = true;
                }
                SyntaxKind::Link | SyntaxKind::WikiLink => {
                    run.flush_at_element(&mut items);
                    let (target, children) = if node.kind() == SyntaxKind::Link {
                        let link = Link::cast(node.clone()).unwrap();
                        (link.target(), desugar_inline(link.content(), diags))
                    } else {
                        let link = WikiLink::cast(node.clone()).unwrap();
                        (
                            tokens_text(&link.target_tokens()),
                            desugar_inline(link.content(), diags),
                        )
                    };
                    let mut expr = Expr::call("link", node.text_range())
                        .with_field("target", Value::Str(target))
                        .with_children(children);
                    expr.set_attrs(pending.take());
                    items.push(expr);
                    after_element = true;
                }
                SyntaxKind::CodeCall => {
                    run.flush_at_element(&mut items);
                    let mut exprs =
                        desugar_code_call(&CodeCall::cast(node.clone()).unwrap(), diags);
                    for expr in &mut exprs {
                        expr.set_attrs(pending.take());
                    }
                    items.extend(exprs);
                    after_element = true;
                }
                _ => {}
            },
        }
    }
    if !pending.is_empty() {
        diags.push(Diagnostic {
            phase: Phase::Semantic,
            span: pending_range.unwrap_or_default(),
            message: "annotation without a following element".to_string(),
        });
    }
    run.flush_at_boundary(&mut items);
    items
}
