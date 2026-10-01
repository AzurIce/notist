use rowan::{NodeOrToken, TextRange, TextSize};

use notist_syntax::ast::{
    Annotation, Block, CodeCall, Document, Entry, Link, List, ListItem, WikiLink,
};
use notist_syntax::parser::Diagnostic;
use notist_syntax::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::expr::{BodyFlavor, Expr};
use crate::item::{Dict, Value};

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
/// is introduced by eval.
///
/// Annotations land here: `@(dict)` payloads become the attrs of the
/// immediately following block; `@!(dict)` become the module's.
pub fn desugar(document: &Document, diags: &mut Vec<Diagnostic>) -> (Vec<Expr>, Dict) {
    desugar_blocks(document.blocks(), diags)
}

/// A block sequence → Expr forest, sharing the annotation-pending logic
/// between the document body and block-level `[...]` bodies. The returned
/// Dict collects module (`@!`) attrs; `[...]` bodies ignore it.
fn desugar_blocks(
    blocks: impl Iterator<Item = Block>,
    diags: &mut Vec<Diagnostic>,
) -> (Vec<Expr>, Dict) {
    let mut forest = Vec::new();
    let mut pending = Dict::default();
    let mut module_attrs = Dict::default();
    let mut seen_content = false;
    for block in blocks {
        match block {
            Block::Annotation(annotation) => {
                let dict = annotation_dict(&annotation, diags);
                if annotation.is_module() {
                    if seen_content {
                        diags.push(Diagnostic {
                            span: annotation.range(),
                            message: "module annotation must precede all content".to_string(),
                        });
                    }
                    module_attrs.extend(dict);
                } else {
                    pending.extend(dict);
                }
            }
            _ => {
                seen_content = true;
                let mut expr = desugar_block(&block, diags);
                expr.set_attrs(pending.take());
                forest.push(expr);
            }
        }
    }
    (forest, module_attrs)
}

fn desugar_block(block: &Block, diags: &mut Vec<Diagnostic>) -> Expr {
    match block {
        Block::Heading(heading) => {
            let level = heading.level() as i64;
            let children = desugar_inline(heading.content(), diags);
            Expr::call("heading", heading.range())
                .with_field("level", Value::Int(level))
                .with_children(children)
        }
        Block::Paragraph(paragraph) => {
            let children = desugar_inline(paragraph.content(), diags);
            Expr::call("paragraph", paragraph.range()).with_children(children)
        }
        Block::List(list) => desugar_list(list, diags),
        Block::CodeCall(call) => desugar_code_call(call, diags)
            .into_iter()
            .next()
            .expect("a call desugars to exactly one expr"),
        Block::Annotation(_) => unreachable!("annotations are handled by the document loop"),
    }
}

/// The dict carried by an `@(dict)` annotation (stray members diagnosed).
fn annotation_dict(annotation: &Annotation, diags: &mut Vec<Diagnostic>) -> Dict {
    let mut dict = Dict::default();
    if let Some(node) = annotation.payload_dict() {
        for el in value_children(&node) {
            // the colon of the empty-dict spelling `(:)` is structural
            if !matches!(el.kind(), SyntaxKind::Entry | SyntaxKind::Colon) {
                diags.push(Diagnostic {
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
        .with_children(
            list.items()
                .map(|item| desugar_list_item(&item, diags))
                .collect(),
        )
}

fn desugar_list_item(item: &ListItem, diags: &mut Vec<Diagnostic>) -> Expr {
    let mut children = Vec::new();
    children.extend(desugar_inline(item.content(), diags));
    for nested in item.lists() {
        children.push(desugar_list(&nested, diags));
    }
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
                    span: open_span,
                    message: "content is mounted with `[..]` after the call, not passed as an argument"
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

/// Flavor derived from a bracketed content region: block iff it contains a
/// block-level node.
fn is_block_content(elements: &[NodeOrToken<SyntaxNode, SyntaxToken>]) -> bool {
    elements.iter().any(|el| {
        matches!(
            el.kind(),
            SyntaxKind::Paragraph
                | SyntaxKind::Heading
                | SyntaxKind::List
                | SyntaxKind::ListItem
                | SyntaxKind::Raw
                | SyntaxKind::ParBreak
                | SyntaxKind::Error
        )
    })
}

/// Desugar a bracketed content region with its derived flavor.
fn content_children(
    elements: &[NodeOrToken<SyntaxNode, SyntaxToken>],
    diags: &mut Vec<Diagnostic>,
) -> Vec<Expr> {
    if is_block_content(elements) {
        desugar_blocks(
            elements
                .iter()
                .filter_map(|el| el.as_node().and_then(|n| Block::cast(n.clone()))),
            diags,
        )
        .0
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

/// code CST → Value. The four string forms are resolved here (escapes, raw,
/// multiline framing).
fn syntax_value(
    element: &NodeOrToken<SyntaxNode, SyntaxToken>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Value> {
    match element {
        NodeOrToken::Token(token) => match token.kind() {
            SyntaxKind::Str => unquote(token.text(), token.text_range(), diags).map(Value::Str),
            SyntaxKind::Number => {
                let text = token.text();
                let parsed = if text.contains('.') {
                    text.parse::<f64>().ok().map(Value::Float)
                } else {
                    text.parse::<i64>().ok().map(Value::Int)
                };
                match parsed {
                    Some(value) => Some(value),
                    None => {
                        diags.push(Diagnostic {
                            span: token.text_range(),
                            message: "number out of range".to_string(),
                        });
                        None
                    }
                }
            }
            SyntaxKind::Ident => match token.text() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => {
                    diags.push(Diagnostic {
                        span: token.text_range(),
                        message: "bare names are not literals".to_string(),
                    });
                    None
                }
            },
            _ => None,
        },
        NodeOrToken::Node(node) => match node.kind() {
            SyntaxKind::Unit => Some(Value::Unit),
            SyntaxKind::Neg => {
                let number = node
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == SyntaxKind::Number)?;
                let value = syntax_value(&NodeOrToken::Token(number), diags)?;
                Some(match value {
                    Value::Int(i) => Value::Int(-i),
                    Value::Float(f) => Value::Float(-f),
                    other => other,
                })
            }
            SyntaxKind::Array => {
                let mut items = Vec::new();
                for child in value_children(node) {
                    if let Some(value) = syntax_value(&child, diags) {
                        items.push(value);
                    }
                }
                Some(Value::Array(items))
            }
            SyntaxKind::Dict => {
                let mut dict = Dict::default();
                for child in node.children().filter_map(Entry::cast) {
                    let Some(key_token) = child.key_token() else {
                        continue;
                    };
                    let Some(key) = key_text(&key_token, diags) else {
                        continue;
                    };
                    let Some(value_el) = child.value() else {
                        continue;
                    };
                    if let Some(value) = syntax_value(&value_el, diags) {
                        dict.insert(key, value);
                    }
                }
                Some(Value::Dict(dict))
            }
            _ => None,
        },
    }
}

/// A group node's value members: everything except trivia, commas, and the
/// parens themselves.
fn value_children(node: &SyntaxNode) -> impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> {
    node.children_with_tokens().filter(|el| {
        !matches!(
            el.kind(),
            SyntaxKind::Whitespace
                | SyntaxKind::Newline
                | SyntaxKind::LineComment
                | SyntaxKind::BlockComment
                | SyntaxKind::Comma
                | SyntaxKind::LParen
                | SyntaxKind::RParen
        )
    })
}

fn key_text(token: &SyntaxToken, diags: &mut Vec<Diagnostic>) -> Option<String> {
    match token.kind() {
        SyntaxKind::Ident => Some(token.text().to_string()),
        SyntaxKind::Str => unquote(token.text(), token.text_range(), diags),
        _ => None,
    }
}

/// String token text → value content: strips the raw prefix, quotes, and
/// multiline framing; processes the five escapes for non-raw forms.
fn unquote(text: &str, range: TextRange, diags: &mut Vec<Diagnostic>) -> Option<String> {
    let mut body = text;
    let mut hashes = 0;
    if let Some(rest) = body.strip_prefix('r') {
        hashes = rest.bytes().take_while(|&b| b == b'#').count();
        body = &rest[hashes..];
    }
    let quote_len = if body.starts_with("\"\"\"") { 3 } else { 1 };
    let closer = format!("{}{}", "\"".repeat(quote_len), "#".repeat(hashes));
    if !body.starts_with('"')
        || !body.ends_with(closer.as_str())
        || body.len() < quote_len + closer.len()
    {
        diags.push(Diagnostic {
            span: range,
            message: "unclosed string".to_string(),
        });
        return None;
    }
    let mut inner = &body[quote_len..body.len() - closer.len()];
    if quote_len == 3 {
        if let Some(stripped) = inner.strip_prefix('\n') {
            inner = stripped;
        }
        if let Some(stripped) = inner.strip_suffix('\n') {
            inner = stripped;
        }
    }
    if hashes > 0 {
        return Some(inner.to_string());
    }
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            _ => {
                diags.push(Diagnostic {
                    span: range,
                    message: "unknown escape".to_string(),
                });
                return None;
            }
        }
    }
    Some(out)
}

fn desugar_inline(
    elements: impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>>,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Expr> {
    let mut items = Vec::new();
    let mut buf = String::new();
    let mut start: Option<TextSize> = None;
    let mut content_len = 0usize;
    let mut content_end: Option<TextSize> = None;
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
                        | SyntaxKind::RawInline
                        | SyntaxKind::Math
                        | SyntaxKind::Link
                        | SyntaxKind::WikiLink
                        | SyntaxKind::CodeCall
                        | SyntaxKind::Annotation
                ),
                NodeOrToken::Token(_) => false,
            };
            if !followed {
                diags.push(Diagnostic {
                    span: element.text_range(),
                    message: "annotation must be immediately followed by an element".to_string(),
                });
                pending = Dict::default();
            }
        }
        match element {
            NodeOrToken::Token(token) => match token.kind() {
                SyntaxKind::Newline => {
                    flush_text(
                        &mut items,
                        &mut buf,
                        &mut start,
                        &mut content_len,
                        &mut content_end,
                    );
                }
                SyntaxKind::LineComment | SyntaxKind::BlockComment => {}
                SyntaxKind::Whitespace => buf.push_str(token.text()),
                SyntaxKind::Escape => {
                    if start.is_none() {
                        start = Some(token.text_range().start());
                    }
                    buf.push_str(&token.text()[1..]);
                    content_len = buf.len();
                    content_end = Some(token.text_range().end());
                }
                _ => {
                    if start.is_none() {
                        start = Some(token.text_range().start());
                    }
                    buf.push_str(token.text());
                    content_len = buf.len();
                    content_end = Some(token.text_range().end());
                }
            },
            NodeOrToken::Node(node) => match node.kind() {
                SyntaxKind::Annotation => {
                    let annotation = Annotation::cast(node.clone()).unwrap();
                    if annotation.is_module() {
                        diags.push(Diagnostic {
                            span: node.text_range(),
                            message: "module annotation is only valid at the document top"
                                .to_string(),
                        });
                    }
                    pending.extend(annotation_dict(&annotation, diags));
                    pending_range = Some(node.text_range());
                }
                SyntaxKind::RawInline => {
                    flush_text(
                        &mut items,
                        &mut buf,
                        &mut start,
                        &mut content_len,
                        &mut content_end,
                    );
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
                }
                SyntaxKind::Math => {
                    flush_text(
                        &mut items,
                        &mut buf,
                        &mut start,
                        &mut content_len,
                        &mut content_end,
                    );
                    let tokens: Vec<_> = node
                        .children_with_tokens()
                        .filter_map(|e| e.into_token())
                        .collect();
                    let text: String = tokens[1..tokens.len() - 1]
                        .iter()
                        .map(|t| t.text())
                        .collect();
                    let mut expr =
                        Expr::call("math", node.text_range()).with_field("text", Value::Str(text));
                    expr.set_attrs(pending.take());
                    items.push(expr);
                }
                SyntaxKind::Strong | SyntaxKind::Emph => {
                    flush_text(
                        &mut items,
                        &mut buf,
                        &mut start,
                        &mut content_len,
                        &mut content_end,
                    );
                    let (name, delim) = if node.kind() == SyntaxKind::Strong {
                        ("strong", SyntaxKind::Star)
                    } else {
                        ("emph", SyntaxKind::Underscore)
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
                }
                SyntaxKind::Link | SyntaxKind::WikiLink => {
                    flush_text(
                        &mut items,
                        &mut buf,
                        &mut start,
                        &mut content_len,
                        &mut content_end,
                    );
                    let (target, children) = if node.kind() == SyntaxKind::Link {
                        let link = Link::cast(node.clone()).unwrap();
                        (
                            tokens_text(&link.target_tokens()),
                            desugar_inline(link.content(), diags),
                        )
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
                }
                SyntaxKind::CodeCall => {
                    flush_text(
                        &mut items,
                        &mut buf,
                        &mut start,
                        &mut content_len,
                        &mut content_end,
                    );
                    let mut exprs = desugar_code_call(&CodeCall::cast(node.clone()).unwrap(), diags);
                    for expr in &mut exprs {
                        expr.set_attrs(pending.take());
                    }
                    items.extend(exprs);
                }
                _ => {}
            },
        }
    }
    if !pending.is_empty() {
        diags.push(Diagnostic {
            span: pending_range.unwrap_or_default(),
            message: "annotation without a following element".to_string(),
        });
    }
    flush_text(
        &mut items,
        &mut buf,
        &mut start,
        &mut content_len,
        &mut content_end,
    );
    items
}

fn flush_text(
    items: &mut Vec<Expr>,
    buf: &mut String,
    start: &mut Option<TextSize>,
    content_len: &mut usize,
    content_end: &mut Option<TextSize>,
) {
    if let (Some(s), Some(e)) = (*start, *content_end) {
        let text = buf[..*content_len].trim_start().to_string();
        if !text.is_empty() {
            items.push(Expr::text(text, TextRange::new(s, e)));
        }
    }
    buf.clear();
    *start = None;
    *content_len = 0;
    *content_end = None;
}
