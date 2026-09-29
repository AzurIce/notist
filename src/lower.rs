use rowan::{NodeOrToken, TextRange, TextSize};

use notist_syntax::ast::{Block, Document, Inline, Link, WikiLink};
use notist_syntax::syntax::{SyntaxKind, SyntaxToken};

use crate::item::{Ctor, Item, Value};

fn tokens_text(tokens: &[SyntaxToken]) -> String {
    tokens.iter().map(|t| t.text()).collect::<String>().trim().to_string()
}

pub fn lower(document: &Document) -> Item {
    let span = document.range();
    Item::new(Ctor::Doc, span).with_children(document.blocks().map(|b| lower_block(&b)).collect())
}

fn lower_block(block: &Block) -> Item {
    match block {
        Block::Heading(heading) => {
            let level = heading.level() as i64;
            let children = heading.inline().map(|i| lower_inline(&i)).unwrap_or_default();
            Item::new(Ctor::Heading, heading.range())
                .with_field("level", Value::Int(level))
                .with_children(children)
        }
        Block::Paragraph(paragraph) => {
            let children = paragraph.inline().map(|i| lower_inline(&i)).unwrap_or_default();
            Item::new(Ctor::Paragraph, paragraph.range()).with_children(children)
        }
    }
}

fn lower_inline(inline: &Inline) -> Vec<Item> {
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
                    items.push(Item::new(Ctor::SoftBreak, token.text_range()));
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
                SyntaxKind::HardBreak => {
                    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
                    items.push(Item::new(Ctor::HardBreak, node.text_range()));
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
                        Item::new(Ctor::RawInline, node.text_range())
                            .with_field("text", Value::Str(text)),
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
                        Item::new(Ctor::Math, node.text_range())
                            .with_field("text", Value::Str(text)),
                    );
                }
                SyntaxKind::Strong | SyntaxKind::Emph => {
                    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
                    let ctor = if node.kind() == SyntaxKind::Strong {
                        Ctor::Strong
                    } else {
                        Ctor::Emph
                    };
                    let children = node
                        .children()
                        .find_map(Inline::cast)
                        .map(|i| lower_inline(&i))
                        .unwrap_or_default();
                    items.push(Item::new(ctor, node.text_range()).with_children(children));
                }
                SyntaxKind::Link | SyntaxKind::WikiLink => {
                    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
                    let (target, children) = if node.kind() == SyntaxKind::Link {
                        let link = Link::cast(node.clone()).unwrap();
                        (
                            tokens_text(&link.target_tokens()),
                            link.inline().map(|i| lower_inline(&i)).unwrap_or_default(),
                        )
                    } else {
                        let link = WikiLink::cast(node.clone()).unwrap();
                        (
                            tokens_text(&link.target_tokens()),
                            link.inline().map(|i| lower_inline(&i)).unwrap_or_default(),
                        )
                    };
                    items.push(
                        Item::new(Ctor::Link, node.text_range())
                            .with_field("target", Value::Str(target))
                            .with_children(children),
                    );
                }
                _ => {}
            },
        }
    }
    flush_text(&mut items, &mut buf, &mut start, &mut content_len, &mut content_end);
    items
}

fn flush_text(
    items: &mut Vec<Item>,
    buf: &mut String,
    start: &mut Option<TextSize>,
    content_len: &mut usize,
    content_end: &mut Option<TextSize>,
) {
    if let (Some(s), Some(e)) = (*start, *content_end) {
        let text = buf[..*content_len].trim_start().to_string();
        if !text.is_empty() {
            items.push(Item::text(text, TextRange::new(s, e)));
        }
    }
    buf.clear();
    *start = None;
    *content_len = 0;
    *content_end = None;
}
