use std::fmt;

use rushdown::ast::{Arena, KindData, NodeKind, NodeRef, NodeType, PrettyPrint};
use rushdown::parser::{Context, InlineParser};
use rushdown::text::{BlockReader, Reader};

/// An opaque math payload, kept outside Markdown's emphasis/link parsing.
#[derive(Debug)]
pub(crate) struct Math {
    pub text: String,
    pub end: usize,
}

impl NodeKind for Math {
    fn typ(&self) -> NodeType {
        NodeType::Inline
    }

    fn kind_name(&self) -> &'static str {
        "Math"
    }
}

impl PrettyPrint for Math {
    fn pretty_print(&self, w: &mut dyn fmt::Write, _: &str, _: usize) -> fmt::Result {
        write!(w, "{:?}", self.text)
    }
}

#[derive(Debug)]
pub(crate) struct MathParser;

impl InlineParser for MathParser {
    fn trigger(&self) -> &[u8] {
        b"$"
    }

    fn parse(
        &self,
        arena: &mut Arena,
        _: NodeRef,
        reader: &mut BlockReader,
        _: &mut Context,
    ) -> Option<NodeRef> {
        let (line, _) = reader.peek_line()?;
        if line.chars().nth(1)?.is_whitespace() {
            return None;
        }
        reader.advance(1);
        let mut text = String::new();
        let mut escaped = false;
        // BlockReader only exposes this Markdown block's lines, so a pair
        // may cross a soft break but cannot cross a paragraph/block boundary.
        while let Some((line, segment)) = reader.peek_line() {
            for (offset, ch) in line.char_indices() {
                if ch == '$' && !escaped {
                    if text.is_empty() {
                        return None;
                    }
                    if !text.chars().next_back().unwrap().is_whitespace() {
                        let end = segment.start() + offset + 1;
                        reader.advance(offset + 1);
                        return Some(
                            arena.new_node(KindData::Extension(Box::new(Math { text, end }))),
                        );
                    }
                }
                text.push(ch);
                escaped = ch == '\\' && !escaped;
            }
            reader.advance_line();
        }
        // rushdown restores the reader when an inline parser returns None.
        None
    }
}
