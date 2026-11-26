use std::fmt;

use rushdown::ast::{Arena, KindData, NodeKind, NodeRef, NodeType, PrettyPrint};
use rushdown::parser::{Context, InlineParser};
use rushdown::text::{BlockReader, Reader};

/// `[[target]]` or `[[target|label]]` wiki-style link.
#[derive(Debug)]
pub(crate) struct WikiLink {
    pub target: String,
    pub label: Option<String>,
    pub start: usize,
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
        write!(w, "[[{}]]", self.target)
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
        let close = body.find("]]")?;
        let inner = &body[..close];
        if inner.trim().is_empty() || inner.contains('\n') {
            return None;
        }
        let (target, label) = match inner.find('|') {
            Some(pipe) => {
                let raw_target = if matches!(arena[parent].kind_data(), KindData::TableCell(_)) {
                    inner[..pipe].strip_suffix('\\').unwrap_or(&inner[..pipe])
                } else {
                    &inner[..pipe]
                };
                (
                    raw_target.trim().to_owned(),
                    Some(inner[pipe + 1..].to_owned()),
                )
            }
            None => (inner.trim().to_owned(), None),
        };
        let start = segment.start();
        let end = start + 2 + close + 2;
        reader.advance(2 + close + 2);
        Some(arena.new_node(KindData::Extension(Box::new(WikiLink {
            target,
            label,
            start,
            end,
        }))))
    }
}
