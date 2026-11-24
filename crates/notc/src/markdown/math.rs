use std::cell::RefCell;
use std::fmt;

use rushdown::ast::{Arena, KindData, NodeKind, NodeRef, NodeType, PrettyPrint};
use rushdown::parser::{BlockParser, Context, InlineParser, State};
use rushdown::text::{BasicReader, BlockReader, Reader};

/// Inline `$...$` or flanked `$ ... $` math node.
#[derive(Debug)]
pub(crate) struct MathInline {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub block: bool,
}

impl NodeKind for MathInline {
    fn typ(&self) -> NodeType {
        NodeType::Inline
    }
    fn kind_name(&self) -> &'static str {
        "MathInline"
    }
}

impl PrettyPrint for MathInline {
    fn pretty_print(&self, w: &mut dyn fmt::Write, _: &str, _: usize) -> fmt::Result {
        write!(w, "{:?}", self.text)
    }
}

#[derive(Debug)]
pub(crate) struct MathInlineParser;

impl InlineParser for MathInlineParser {
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
        let (line, first_seg) = reader.peek_line()?;
        let start = first_seg.start();
        let second = line.chars().nth(1)?;
        if second == '$' {
            return None;
        }
        let block = second.is_whitespace();
        reader.advance(1);
        let mut text = String::new();
        let mut escaped = false;
        while let Some((line, segment)) = reader.peek_line() {
            for (offset, ch) in line.char_indices() {
                if ch == '$' && !escaped {
                    if text.is_empty() {
                        return None;
                    }
                    if text.chars().next_back().unwrap().is_whitespace() == block {
                        if text.trim().is_empty() {
                            return None;
                        }
                        let end = segment.start() + offset + 1;
                        reader.advance(offset + 1);
                        return Some(arena.new_node(KindData::Extension(Box::new(MathInline {
                            text: if block { text.trim().to_owned() } else { text },
                            start,
                            end,
                            block,
                        }))));
                    }
                }
                text.push(ch);
                escaped = ch == '\\' && !escaped;
            }
            reader.advance_line();
        }
        None
    }
}

/// Fenced display math block: `$$` ... `$$` or single-line `$$expr$$`.
#[derive(Debug)]
pub(crate) struct MathBlock {
    pub lines: RefCell<Vec<String>>,
    pub start: usize,
    pub end: RefCell<usize>,
    pub closed: RefCell<bool>,
}

impl NodeKind for MathBlock {
    fn typ(&self) -> NodeType {
        NodeType::LeafBlock
    }
    fn kind_name(&self) -> &'static str {
        "MathBlock"
    }
}

impl PrettyPrint for MathBlock {
    fn pretty_print(&self, w: &mut dyn fmt::Write, _: &str, _: usize) -> fmt::Result {
        write!(w, "{:?}", self.lines.borrow().join("\n"))
    }
}

#[derive(Debug)]
pub(crate) struct MathBlockParser;

impl BlockParser for MathBlockParser {
    fn trigger(&self) -> &[u8] {
        b"$"
    }

    fn open(
        &self,
        arena: &mut Arena,
        _: NodeRef,
        reader: &mut BasicReader,
        ctx: &mut Context,
    ) -> Option<(NodeRef, State)> {
        let (line, segment) = reader.peek_line()?;
        let offset = ctx.block_offset()?;
        let rest = line[offset..].trim_end_matches(['\r', '\n']);
        let trimmed = rest.trim_end();
        let start = segment.start() + offset;

        // Single-line `$$expr$$`
        if trimmed.starts_with("$$") && trimmed.ends_with("$$") && trimmed.len() >= 4 {
            let content = trimmed[2..trimmed.len() - 2].trim().to_owned();
            let end = start + rest.len();
            reader.advance_to_eol();
            let node = arena.new_node(KindData::Extension(Box::new(MathBlock {
                lines: RefCell::new(vec![content]),
                start,
                end: RefCell::new(end),
                closed: RefCell::new(true),
            })));
            return Some((node, State::NO_CHILDREN));
        }

        // Multi-line `$$` fence
        if trimmed == "$$" {
            let end = start + rest.len();
            reader.advance_to_eol();
            let node = arena.new_node(KindData::Extension(Box::new(MathBlock {
                lines: RefCell::new(Vec::new()),
                start,
                end: RefCell::new(end),
                closed: RefCell::new(false),
            })));
            return Some((node, State::NO_CHILDREN));
        }

        None
    }

    fn cont(
        &self,
        arena: &mut Arena,
        node: NodeRef,
        reader: &mut BasicReader,
        _: &mut Context,
    ) -> Option<State> {
        let KindData::Extension(data) = arena[node].kind_data() else {
            return None;
        };
        let math = data.as_any().downcast_ref::<MathBlock>()?;
        if *math.closed.borrow() {
            return None;
        }
        let (line, segment) = reader.peek_line()?;
        let stripped = line.trim_end_matches(['\r', '\n']);
        *math.end.borrow_mut() = segment.start() + stripped.len();
        if stripped.trim() == "$$" {
            *math.closed.borrow_mut() = true;
            reader.advance_to_eol();
            return Some(State::NO_CHILDREN);
        }
        math.lines.borrow_mut().push(stripped.to_owned());
        reader.advance_to_eol();
        Some(State::NO_CHILDREN)
    }

    fn can_interrupt_paragraph(&self) -> bool {
        true
    }
}
