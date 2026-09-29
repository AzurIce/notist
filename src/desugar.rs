use rowan::{NodeOrToken, TextRange, TextSize};

use notist_syntax::ast::{Block, CodeCall, Document, Inline, Link, List, ListItem, WikiLink};
use notist_syntax::parser::Diagnostic;
use notist_syntax::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::code;
use crate::expr::Expr;
use crate::item::{Dict, Value};

fn tokens_text(tokens: &[SyntaxToken]) -> String {
    tokens.iter().map(|t| t.text()).collect::<String>().trim().to_string()
}

/// CST → Expr 森林：desugar 步骤。文档是模块体（顶层表达式序列），
/// 不是任何构造器调用；`doc` 根节点由 eval 在求值时引入。
///
/// 注解在这里落地：`@(…)` 的 payload（dict 字面量）成为紧随其后的块的
/// attrs，`@!(…)` 成为模块 attrs。
pub fn desugar(document: &Document, diags: &mut Vec<Diagnostic>) -> (Vec<Expr>, Dict) {
    let mut forest = Vec::new();
    let mut pending = Dict::default();
    let mut module_attrs = Dict::default();
    let mut seen_content = false;
    for block in document.blocks() {
        match block {
            Block::Annotation(annotation) => {
                let (text, base) = annotation.payload();
                let (dict, d) = code::parse_dict_entries(&text, base);
                diags.extend(d);
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
    let ordered =
        list.items().next().and_then(|item| item.marker()) == Some(SyntaxKind::Plus);
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
    if let Some((text, base)) = call.args() {
        let (positional, named, d) = code::parse_args(&text, base);
        diags.extend(d);
        args = positional.into_iter().map(|v| Expr::Literal(v, span)).collect();
        fields = named;
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
                    message: "`#(…)` takes exactly one literal".to_string(),
                });
                Expr::Literal(Value::Unit, span)
            }
        }
    }
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
                    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
                }
                SyntaxKind::LineComment | SyntaxKind::BlockComment => {}
                SyntaxKind::Whitespace => buf.push_str(token.text()),
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
                SyntaxKind::Escape => {
                    let escaped = node
                        .children_with_tokens()
                        .filter_map(|e| e.into_token())
                        .nth(1)
                        .unwrap();
                    if start.is_none() {
                        start = Some(node.text_range().start());
                    }
                    buf.push_str(escaped.text());
                    content_len = buf.len();
                    content_end = Some(node.text_range().end());
                }
                SyntaxKind::RawInline => {
                    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
                    let tokens: Vec<_> = node
                        .children_with_tokens()
                        .filter_map(|e| e.into_token())
                        .collect();
                    let text: String =
                        tokens[1..tokens.len() - 1].iter().map(|t| t.text()).collect();
                    items.push(
                        Expr::call("raw", node.text_range()).with_field("text", Value::Str(text)),
                    );
                }
                SyntaxKind::Math => {
                    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
                    let tokens: Vec<_> = node
                        .children_with_tokens()
                        .filter_map(|e| e.into_token())
                        .collect();
                    let text: String =
                        tokens[1..tokens.len() - 1].iter().map(|t| t.text()).collect();
                    items.push(
                        Expr::call("math", node.text_range()).with_field("text", Value::Str(text)),
                    );
                }
                SyntaxKind::Strong | SyntaxKind::Emph => {
                    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
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
                    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
                    let (target, children) = if node.kind() == SyntaxKind::Link {
                        let link = Link::cast(node.clone()).unwrap();
                        (
                            tokens_text(&link.target_tokens()),
                            link.inline().map(|i| desugar_inline(&i, diags)).unwrap_or_default(),
                        )
                    } else {
                        let link = WikiLink::cast(node.clone()).unwrap();
                        (
                            tokens_text(&link.target_tokens()),
                            link.inline().map(|i| desugar_inline(&i, diags)).unwrap_or_default(),
                        )
                    };
                    items.push(
                        Expr::call("link", node.text_range())
                            .with_field("target", Value::Str(target))
                            .with_children(children),
                    );
                }
                SyntaxKind::CodeCall => {
                    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
                    items.push(desugar_code_call(&node, diags));
                }
                _ => {}
            },
        }
    }
    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
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
