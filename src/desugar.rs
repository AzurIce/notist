use rowan::{NodeOrToken, TextRange, TextSize};

use notist_syntax::ast::{
    Block, CodeCall, Document, Entry, Inline, Link, List, ListItem, WikiLink,
};
use notist_syntax::parser::Diagnostic;
use notist_syntax::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::expr::Expr;
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
/// Annotations land here: `@(..)` payloads (dict literals) become the attrs
/// of the immediately following block; `@!(..)` become the module's.
pub fn desugar(document: &Document, diags: &mut Vec<Diagnostic>) -> (Vec<Expr>, Dict) {
    let mut forest = Vec::new();
    let mut pending = Dict::default();
    let mut module_attrs = Dict::default();
    let mut seen_content = false;
    for block in document.blocks() {
        match block {
            Block::Annotation(annotation) => {
                for stray in annotation.stray_elements() {
                    diags.push(Diagnostic {
                        span: stray.text_range(),
                        message: "annotation entries must be `key: value`".to_string(),
                    });
                }
                let mut dict = Dict::default();
                for entry in annotation.entries() {
                    let Some(key) = entry.key_token() else {
                        continue;
                    };
                    let Some(key) = key_text(&key, diags) else {
                        continue;
                    };
                    let Some(value_el) = entry.value() else {
                        continue;
                    };
                    let Some(value) = syntax_value(&value_el, diags) else {
                        continue;
                    };
                    dict.insert(key, value);
                }
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
            let children = heading
                .inline()
                .map(|i| desugar_inline(&i, diags))
                .unwrap_or_default();
            Expr::call("heading", heading.range())
                .with_field("level", Value::Int(level))
                .with_children(children)
        }
        Block::Paragraph(paragraph) => {
            let children = paragraph
                .inline()
                .map(|i| desugar_inline(&i, diags))
                .unwrap_or_default();
            Expr::call("paragraph", paragraph.range()).with_children(children)
        }
        Block::List(list) => desugar_list(list, diags),
        Block::Annotation(_) => unreachable!("annotations are handled by the document loop"),
    }
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
    if let Some(inline) = item.inline() {
        children.extend(desugar_inline(&inline, diags));
    }
    for nested in item.lists() {
        children.push(desugar_list(&nested, diags));
    }
    Expr::call("item", item.range()).with_children(children)
}

fn desugar_code_call(node: &SyntaxNode, diags: &mut Vec<Diagnostic>) -> Expr {
    let call = CodeCall::cast(node.clone()).unwrap();
    let span = node.text_range();
    let mut args = Vec::new();
    let mut fields = Dict::default();
    for member in call.args() {
        match &member {
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::Entry => {
                let entry = Entry::cast(n.clone()).unwrap();
                let Some(key_token) = entry.key_token() else {
                    continue;
                };
                let Some(key) = key_text(&key_token, diags) else {
                    continue;
                };
                let Some(value_el) = entry.value() else {
                    continue;
                };
                let Some(value) = syntax_value(&value_el, diags) else {
                    continue;
                };
                fields.insert(key, value);
            }
            other => {
                if let Some(value) = syntax_value(other, diags) {
                    args.push(Expr::Literal(value, span));
                }
            }
        }
    }
    match call.name() {
        Some(name) => {
            let children = call
                .body()
                .map(|i| desugar_inline(&i, diags))
                .unwrap_or_default();
            Expr::Call {
                name,
                args,
                fields,
                children,
                attrs: Dict::default(),
                span,
            }
        }
        None => {
            if fields.iter().next().is_none() && args.len() == 1 {
                args.pop().unwrap()
            } else {
                diags.push(Diagnostic {
                    span,
                    message: "`#(..)` takes exactly one literal".to_string(),
                });
                Expr::Literal(Value::Unit, span)
            }
        }
    }
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

fn desugar_inline(inline: &Inline, diags: &mut Vec<Diagnostic>) -> Vec<Expr> {
    let mut items = Vec::new();
    let mut buf = String::new();
    let mut start: Option<TextSize> = None;
    let mut content_len = 0usize;
    let mut content_end: Option<TextSize> = None;

    for element in inline.elements() {
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
                    items.push(
                        Expr::call("raw", node.text_range()).with_field("text", Value::Str(text)),
                    );
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
                    items.push(
                        Expr::call("math", node.text_range()).with_field("text", Value::Str(text)),
                    );
                }
                SyntaxKind::Strong | SyntaxKind::Emph => {
                    flush_text(
                        &mut items,
                        &mut buf,
                        &mut start,
                        &mut content_len,
                        &mut content_end,
                    );
                    let name = if node.kind() == SyntaxKind::Strong {
                        "strong"
                    } else {
                        "emph"
                    };
                    let children = node
                        .children()
                        .find_map(Inline::cast)
                        .map(|i| desugar_inline(&i, diags))
                        .unwrap_or_default();
                    items.push(Expr::call(name, node.text_range()).with_children(children));
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
                            link.inline()
                                .map(|i| desugar_inline(&i, diags))
                                .unwrap_or_default(),
                        )
                    } else {
                        let link = WikiLink::cast(node.clone()).unwrap();
                        (
                            tokens_text(&link.target_tokens()),
                            link.inline()
                                .map(|i| desugar_inline(&i, diags))
                                .unwrap_or_default(),
                        )
                    };
                    items.push(
                        Expr::call("link", node.text_range())
                            .with_field("target", Value::Str(target))
                            .with_children(children),
                    );
                }
                SyntaxKind::CodeCall => {
                    flush_text(
                        &mut items,
                        &mut buf,
                        &mut start,
                        &mut content_len,
                        &mut content_end,
                    );
                    items.push(desugar_code_call(&node, diags));
                }
                _ => {}
            },
        }
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
