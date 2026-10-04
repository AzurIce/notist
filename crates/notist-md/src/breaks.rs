use std::collections::BTreeSet;

use rushdown::as_type_data_mut;
use rushdown::ast::{Arena, KindData, NodeRef, Paragraph, TextQualifier};
use rushdown::parser::{Context, ParagraphTransformer};
use rushdown::text::{BasicReader, Segment};

/// Locate paragraph breaks in parsed text, so opaque code/math payloads are
/// excluded. The second parse splits block sources before inline pairing.
pub(crate) fn positions(arena: &Arena, root: NodeRef, src: &str) -> BTreeSet<usize> {
    let mut positions = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        let node = &arena[node];
        pending.extend(node.children(arena));
        let KindData::Text(text) = node.kind_data() else {
            continue;
        };
        if !text.has_qualifiers(TextQualifier::HARD_LINE_BREAK)
            && !text.has_qualifiers(TextQualifier::SOFT_LINE_BREAK)
        {
            continue;
        }
        let Some(index) = text.index() else { continue };
        let Some(length) = src[index.start()..].find('\n') else {
            continue;
        };
        let mut end = index.start() + length;
        if src.as_bytes().get(end.wrapping_sub(1)) == Some(&b'\r') {
            end -= 1;
        }
        let slashes = src[..end]
            .bytes()
            .rev()
            .take_while(|ch| *ch == b'\\')
            .count();
        if slashes % 2 == 1 {
            positions.insert(end - 1);
        }
    }
    positions
}

#[derive(Debug, Clone)]
pub(crate) struct Breaks(pub BTreeSet<usize>);

impl ParagraphTransformer for Breaks {
    fn transform(
        &self,
        arena: &mut Arena,
        paragraph: NodeRef,
        _: &mut BasicReader,
        _: &mut Context,
    ) {
        let lines = as_type_data_mut!(arena, paragraph, Block).take_source();
        let mut groups = Vec::new();
        let mut current = Vec::new();
        for line in lines {
            if let Some(&marker) = self.0.range(line.start()..line.stop()).next() {
                if marker > line.start() {
                    current.push(Segment::new_with_padding(
                        line.start(),
                        marker,
                        line.padding(),
                    ));
                }
                if !current.is_empty() {
                    groups.push(std::mem::take(&mut current));
                }
            } else {
                current.push(line);
            }
        }
        if !current.is_empty() {
            groups.push(current);
        }
        if groups.len() <= 1 {
            as_type_data_mut!(arena, paragraph, Block)
                .put_back_source(groups.pop().unwrap_or_default());
            return;
        }
        let parent = arena[paragraph].parent().unwrap();
        for lines in groups {
            let node = arena.new_node(Paragraph::new());
            arena[node].set_pos(lines[0].start());
            as_type_data_mut!(arena, node, Block).put_back_source(lines);
            parent.insert_before(arena, paragraph, node);
        }
        paragraph.delete(arena);
    }
}
