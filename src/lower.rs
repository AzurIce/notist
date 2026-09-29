use rowan::TextRange;

use notist_syntax::ast::{Block, Document, Line};
use notist_syntax::syntax::{SyntaxKind, SyntaxToken};

use crate::item::{Ctor, Item, Value};

pub fn lower(document: &Document) -> Item {
    let span = document.range();
    Item::new(Ctor::Doc, span).with_children(document.blocks().map(|b| lower_block(&b)).collect())
}

fn lower_block(block: &Block) -> Item {
    match block {
        Block::Heading(heading) => {
            let level = heading.level() as i64;
            Item::new(Ctor::Heading, heading.range())
                .with_field("level", Value::Int(level))
                .with_children(lower_lines(vec![Line {
                    tokens: heading.content_tokens(),
                    newline: None,
                }]))
        }
        Block::Paragraph(paragraph) => Item::new(Ctor::Paragraph, paragraph.range())
            .with_children(lower_lines(paragraph.lines())),
    }
}

fn lower_lines(lines: Vec<Line>) -> Vec<Item> {
    let mut children: Vec<Item> = Vec::new();
    let mut pending_break: Option<TextRange> = None;
    for line in lines {
        if let Some(text) = line_text(&line.tokens) {
            if let Some(span) = pending_break.take() {
                children.push(Item::new(Ctor::SoftBreak, span));
            }
            children.push(text);
        }
        if let Some(newline) = line.newline {
            pending_break = Some(newline.text_range());
        }
    }
    children
}

fn is_trivia(token: &SyntaxToken) -> bool {
    matches!(
        token.kind(),
        SyntaxKind::Whitespace | SyntaxKind::LineComment | SyntaxKind::BlockComment
    )
}

fn is_comment(token: &SyntaxToken) -> bool {
    matches!(token.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment)
}

fn line_text(tokens: &[SyntaxToken]) -> Option<Item> {
    let first = tokens.iter().position(|t| !is_trivia(t))?;
    let last = tokens.iter().rposition(|t| !is_trivia(t))?;
    let text: String = tokens[first..=last]
        .iter()
        .filter(|t| !is_comment(t))
        .map(|t| t.text())
        .collect();
    let span = TextRange::new(
        tokens[first].text_range().start(),
        tokens[last].text_range().end(),
    );
    Some(Item::text(text, span))
}
