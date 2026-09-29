use rowan::TextRange;

use notist_syntax::ast::{Block, Document};
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
                .with_children(lower_lines(vec![heading.content_tokens()]))
        }
        Block::Paragraph(paragraph) => Item::new(Ctor::Paragraph, paragraph.range())
            .with_children(lower_lines(paragraph.lines())),
    }
}

fn lower_lines(lines: Vec<Vec<SyntaxToken>>) -> Vec<Item> {
    let mut children: Vec<Item> = Vec::new();
    for line in lines {
        let Some(text) = line_text(&line) else {
            continue;
        };
        if let Some(prev) = children.last() {
            let gap = TextRange::new(prev.span.end(), text.span.start());
            children.push(Item::new(Ctor::SoftBreak, gap));
        }
        children.push(text);
    }
    children
}

fn line_text(tokens: &[SyntaxToken]) -> Option<Item> {
    let first = tokens.iter().position(|t| t.kind() != SyntaxKind::Whitespace)?;
    let last = tokens.iter().rposition(|t| t.kind() != SyntaxKind::Whitespace)?;
    let text: String = tokens[first..=last].iter().map(|t| t.text()).collect();
    let span = TextRange::new(
        tokens[first].text_range().start(),
        tokens[last].text_range().end(),
    );
    Some(Item::text(text, span))
}
