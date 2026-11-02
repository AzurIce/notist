use std::fmt;
use std::ops::Range;

use rushdown::ast::{Arena, KindData, NodeKind, NodeRef, NodeType, PrettyPrint};
use rushdown::parser::{Context, InlineParser};
use rushdown::text::{BlockReader, Reader};

/// Like native markup, wiki targets are raw text, not Markdown destinations.
#[derive(Debug)]
pub(crate) struct WikiLink {
    pub target: String,
    pub label: Option<Range<usize>>,
    pub end: usize,
}

impl NodeKind for WikiLink {
    fn typ(&self) -> NodeType {
        NodeType::Inline
    }

    fn kind_name(&self) -> &'static str {
        "WikiLink"
    }
}

impl PrettyPrint for WikiLink {
    fn pretty_print(&self, w: &mut dyn fmt::Write, _: &str, _: usize) -> fmt::Result {
        write!(w, "{:?}", self.target)
    }
}

#[derive(Debug)]
pub(crate) struct WikiLinkParser;

impl InlineParser for WikiLinkParser {
    fn trigger(&self) -> &[u8] {
        b"["
    }

    fn parse(
        &self,
        arena: &mut Arena,
        parent: NodeRef,
        reader: &mut BlockReader,
        _: &mut Context,
    ) -> Option<NodeRef> {
        let (line, segment) = reader.peek_line()?;
        let body = line.strip_prefix("[[")?;
        // Native wikilinks cannot cross a physical line. Do not consume input
        // on a miss, so ordinary Markdown brackets keep their usual behavior.
        let mut close = body.find("]]")?;
        let pipe = body[..close].find('|');
        if let Some(pipe) = pipe {
            // Reuse the Markdown content scanner for label boundaries: code,
            // math and nested call bodies may themselves contain brackets.
            // The pipe acts as the opening delimiter of this inline body.
            let end = super::markup::content_end(&line, pipe + 2)?;
            if !line[end..].starts_with(']') {
                return None;
            }
            close = end - 3;
        }
        let body = &body[..close];
        let (target, label) = match pipe {
            Some(pipe) => (
                if matches!(arena[parent].kind_data(), KindData::TableCell(_)) {
                    body[..pipe].strip_suffix('\\').unwrap_or(&body[..pipe])
                } else {
                    &body[..pipe]
                },
                Some(segment.start() + 2 + pipe + 1..segment.start() + 2 + close),
            ),
            None => (body, None),
        };
        let node = WikiLink {
            target: target.trim().to_owned(),
            label,
            end: segment.start() + 2 + close + 2,
        };
        reader.advance(2 + close + 2);
        Some(arena.new_node(KindData::Extension(Box::new(node))))
    }
}
