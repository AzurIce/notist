use std::ops::Range;

use super::Parser;
use crate::syntax::SyntaxKind;

struct Row {
    cells: Vec<Range<usize>>,
    end: usize,
    has_pipe: bool,
}

impl Parser<'_> {
    /// A header and delimiter must have the same number of cells. Requiring
    /// an actual pipe in the pair avoids interpreting a plain `---` divider
    /// as a one-column table. Headers start at a container's block boundary.
    pub(super) fn at_table_at(&self, start: usize) -> bool {
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
        header.cells.len() == delimiter.cells.len()
            && delimiter.cells.iter().all(|cell| {
                let text: String = cell.clone().map(|i| self.lexed.text(i)).collect();
                let text = text.trim();
                let marker = text.strip_prefix(':').unwrap_or(text);
                let marker = marker.strip_suffix(':').unwrap_or(marker);
                !marker.is_empty() && marker.bytes().all(|c| c == b'-')
            })
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
            // Multiline strings/comments are opaque tokens, not table rows.
            if self.lexed.text(end).contains(['\n', '\r']) {
                return None;
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
        Some(Row {
            cells,
            end,
            has_pipe,
        })
    }

    pub(super) fn table(&mut self) {
        self.builder.start_node(SyntaxKind::Table.into());
        let header = self.table_row_at(self.pos).unwrap();
        self.table_row(header, false);
        self.eat(); // newline between header and delimiter
        let delimiter = self.table_row_at(self.pos).unwrap();
        self.table_row(delimiter, true);
        while self.cur() == Some(SyntaxKind::Newline) {
            let start = self.pos + 1;
            if self.line_ends_block_at(self.pos)
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
        self.builder.finish_node();
    }

    fn table_row(&mut self, row: Row, delimiter: bool) {
        self.builder.start_node(
            if delimiter {
                SyntaxKind::TableDelimiter
            } else {
                SyntaxKind::TableRow
            }
            .into(),
        );
        for cell in row.cells {
            while self.pos < cell.start {
                self.eat();
            }
            self.builder.start_node(SyntaxKind::TableCell.into());
            let previous_limit = self.limit.replace(cell.end);
            let previous_body = std::mem::replace(&mut self.in_body, false);
            if delimiter {
                while self.cur().is_some() {
                    self.eat();
                }
            } else {
                self.inline(|_| true);
            }
            self.in_body = previous_body;
            self.limit = previous_limit;
            self.builder.finish_node();
        }
        while self.pos < row.end {
            self.eat();
        }
        self.builder.finish_node();
    }
}
