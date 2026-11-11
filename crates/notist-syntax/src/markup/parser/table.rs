use std::ops::Range;

use super::Parser;
use crate::tree::SyntaxKind;

pub(super) struct Row {
    /// Stable source byte ranges, not indices into mode-dependent tokens.
    cells: Vec<Range<usize>>,
    end: usize,
    end_byte: usize,
    has_pipe: bool,
}

impl Parser<'_, '_> {
    /// A header and delimiter must have the same number of cells. Requiring
    /// an actual pipe in the pair avoids interpreting a plain `---` divider
    /// as a one-column table. Headers start at a container's block boundary.
    pub(super) fn at_table_at(&self, start: usize) -> bool {
        // The same line is asked about by both the block-end check and block
        // dispatch, under the same context.
        let key = (start, self.state.limit, self.block_indent);
        if let Some((at, limit, indent, answer)) = *self.table_probe.borrow()
            && (at, limit, indent) == key
        {
            return answer;
        }
        let answer = self.probe_table(start);
        *self.table_probe.borrow_mut() = Some((start, key.1, key.2, answer));
        answer
    }

    fn probe_table(&self, start: usize) -> bool {
        // A table needs a pipe in its two leading lines. Checking bytes first
        // keeps ordinary prose from tokenizing a line twice or allocating.
        if !self.line_has_pipe(start) {
            return false;
        }
        if !self.at_block_start(start) {
            return false;
        }
        if matches!(
            self.kind_at(start),
            None | Some(SyntaxKind::Whitespace | SyntaxKind::Newline)
        ) {
            return false;
        }
        let Some(header) = self.table_row_at(start) else {
            return false;
        };
        if self.kind_at(header.end) != Some(SyntaxKind::Newline) {
            return false;
        }
        let Some(delimiter) = self.table_row_at(header.end + 1) else {
            return false;
        };
        if !header.has_pipe && !delimiter.has_pipe {
            return false;
        }
        if header.cells.len() != delimiter.cells.len()
            || !delimiter.cells.iter().all(|cell| self.is_align_cell(cell))
        {
            return false;
        }
        // The consuming call reuses this plan instead of scanning the same two
        // lines again.
        *self.plan.borrow_mut() = Some((start, header, delimiter));
        true
    }

    /// An alignment cell is `-`+ with optional surrounding colons, trimmed.
    fn is_align_cell(&self, cell: &Range<usize>) -> bool {
        let text = self.state.source[cell.clone()].trim();
        let text = text.strip_prefix(':').unwrap_or(text);
        let text = text.strip_suffix(':').unwrap_or(text);
        !text.is_empty() && text.bytes().all(|byte| byte == b'-')
    }

    /// Whether the line starting at token `start` contains a pipe byte. A pipe
    /// inside an opaque payload still counts here: this only rejects lines
    /// that cannot possibly start a table.
    fn line_has_pipe(&self, start: usize) -> bool {
        let offset = usize::from(self.state.offset(start));
        let rest = &self.state.source[offset..];
        let line = rest.find(['\r', '\n']).map_or(rest, |end| &rest[..end]);
        line.as_bytes()
            .iter()
            .any(|byte| matches!(byte, b'|' | b'#' | b'$'))
            || rest.lines().nth(1).is_some_and(|line| line.contains('|'))
    }

    fn table_row_at(&self, start: usize) -> Option<Row> {
        if self.in_body && self.kind_at(start) == Some(SyntaxKind::RBracket) {
            return None;
        }
        let mut end = start;
        let mut pipes = Vec::new();
        while let Some(kind) = self.kind_at(end) {
            if kind == SyntaxKind::Newline {
                break;
            }
            // Strings and comments are already opaque tokens. Complete calls
            // and raw/math payloads own their internal separators as well.
            if kind == SyntaxKind::Hash
                && let Some(call_end) = self.call_end(end)
            {
                end = call_end;
                continue;
            }
            if let Some(close) = self.opaque_close_at(end) {
                end = close + 1;
                continue;
            }
            if kind == SyntaxKind::Pipe {
                pipes.push(end);
            }
            end += 1;
        }
        let mut left = start;
        let mut right = end;
        while self.kind_at(left) == Some(SyntaxKind::Whitespace) && left < right {
            left += 1;
        }
        while right > left && self.kind_at(right - 1) == Some(SyntaxKind::Whitespace) {
            right -= 1;
        }
        if left == right {
            return None;
        }
        let has_pipe = !pipes.is_empty();
        if self.kind_at(left) == Some(SyntaxKind::Pipe) {
            left += 1;
        }
        if right > left && self.kind_at(right - 1) == Some(SyntaxKind::Pipe) {
            right -= 1;
        }
        let mut cells = Vec::new();
        let mut cell_start = left;
        for pipe in pipes
            .into_iter()
            .filter(|pipe| *pipe >= left && *pipe < right)
        {
            cells.push(cell_start..pipe);
            cell_start = pipe + 1;
        }
        cells.push(cell_start..right);
        let cells = cells
            .into_iter()
            .map(|cell| {
                usize::from(self.state.offset(cell.start))..usize::from(self.state.offset(cell.end))
            })
            .collect();
        Some(Row {
            cells,
            end,
            end_byte: usize::from(self.state.offset(end)),
            has_pipe,
        })
    }

    pub(super) fn table(&mut self) {
        // Both rows are planned before any node is opened: a half-built table
        // must not swallow the header line as prose. The probe that selected
        // this table already planned them, so reuse that plan.
        let planned = self.plan.borrow_mut().take();
        let (header, delimiter) = match planned {
            Some((start, header, delimiter)) if start == self.state.pos => (header, delimiter),
            _ => {
                let Some(header) = self.table_row_at(self.state.pos) else {
                    self.eat();
                    return;
                };
                let Some(delimiter) = self.table_row_at(header.end + 1) else {
                    self.eat();
                    return;
                };
                (header, delimiter)
            }
        };
        self.state.builder.start_node(SyntaxKind::Table.into());
        self.table_row(header, false);
        self.eat(); // newline between header and delimiter
        self.table_row(delimiter, true);
        while self.cur() == Some(SyntaxKind::Newline) {
            let start = self.state.pos + 1;
            if self.line_ends_block_at(self.state.pos)
                || self.in_body && self.kind_at(start) == Some(SyntaxKind::RBracket)
            {
                break;
            }
            let Some(row) = self.table_row_at(start) else {
                break;
            };
            self.eat();
            self.table_row(row, false);
        }
        self.state.builder.finish_node();
    }

    fn table_row(&mut self, row: Row, delimiter: bool) {
        self.state.builder.start_node(
            if delimiter {
                SyntaxKind::TableDelimiter
            } else {
                SyntaxKind::TableRow
            }
            .into(),
        );
        for cell in row.cells {
            while usize::from(self.state.byte()) < cell.start {
                self.eat();
            }
            self.state.builder.start_node(SyntaxKind::TableCell.into());
            let previous_limit = self.state.limit.replace(cell.end);
            let previous_body = std::mem::replace(&mut self.in_body, false);
            if delimiter {
                while self.cur().is_some() {
                    self.eat();
                }
            } else {
                self.inline(|_| true);
            }
            self.in_body = previous_body;
            self.state.limit = previous_limit;
            self.state.builder.finish_node();
        }
        while usize::from(self.state.byte()) < row.end_byte {
            self.eat();
        }
        self.state.builder.finish_node();
    }
}
