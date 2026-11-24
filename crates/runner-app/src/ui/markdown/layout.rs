use std::ops::Range;
use std::sync::Arc;

use pulldown_cmark::Alignment;
use unicode_width::UnicodeWidthChar as _;

use super::view::Position;
use super::{cell_text, Block, Cell, CodeBlock, ImageRef, Inline, FRAGMENT_BYTES};

/// What starts a list item's first row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Marker {
    Bullet,
    Number(u64),
    Task(bool),
}

/// One row of the drawn page: the unit the list virtualizes, so a long
/// code block, list or table draws only its rows in view.
#[derive(Debug)]
pub(crate) struct Row {
    pub(crate) kind: RowKind,
    /// Space above the row, in pixels.
    pub(super) top: f32,
    /// How many lists deep the row sits.
    pub(super) indent: u8,
    /// How many quotes deep the row sits.
    pub(super) quotes: u8,
    pub(super) marker: Option<Marker>,
}

#[derive(Debug)]
pub(crate) enum RowKind {
    Heading(u8, Inline),
    Paragraph(Inline),
    Image(ImageRef),
    /// A code line, or a piece of a long one.
    Code {
        code: Arc<CodeBlock>,
        line: usize,
        bytes: Range<usize>,
    },
    TableRow {
        alignments: Arc<Vec<Alignment>>,
        cells: Vec<Cell>,
        head: bool,
        last: bool,
    },
    Rule,
    /// A line of block HTML.
    Html(String),
}

impl Row {
    /// Whether the row goes on from the row before it, as the later pieces
    /// of a long code line do: copying joins them without a line break.
    pub(super) fn continues(&self) -> bool {
        matches!(&self.kind, RowKind::Code { bytes, .. } if bytes.start > 0)
    }

    /// The row's text as selection and copy see it.
    pub(super) fn text(&self) -> String {
        match &self.kind {
            RowKind::Heading(_, inline) | RowKind::Paragraph(inline) => inline.text.clone(),
            RowKind::Code { code, line, bytes } => code.fragment(*line, bytes.clone()).0,
            RowKind::TableRow { cells, .. } => cells
                .iter()
                .map(|cell| cell_text(cell))
                .collect::<Vec<_>>()
                .join("\t"),
            RowKind::Html(line) => line.clone(),
            RowKind::Image(_) | RowKind::Rule => String::new(),
        }
    }

    pub(super) fn same_layout(&self, other: &Row) -> bool {
        self.top.to_bits() == other.top.to_bits()
            && self.indent == other.indent
            && self.quotes == other.quotes
            && self.marker == other.marker
    }
}

/// A line's bytes in pieces of at most about [`FRAGMENT_BYTES`], split on
/// character boundaries and never before a combining mark.
pub(super) fn fragments(line: &str) -> Vec<Range<usize>> {
    let mut pieces = Vec::new();
    let mut start = 0;
    while start < line.len() || pieces.is_empty() {
        let mut end = (start + FRAGMENT_BYTES).min(line.len());
        while !line.is_char_boundary(end) {
            end += 1;
        }
        while let Some(next) = line[end..].chars().next() {
            if next.width() != Some(0) {
                break;
            }
            end += next.len_utf8();
        }
        pieces.push(start..end);
        start = end;
    }
    pieces
}

/// Lays `blocks` out as rows, `indent` lists and `quotes` quotes deep.
pub(super) fn flatten(blocks: &[Block], indent: u8, quotes: u8, rows: &mut Vec<Row>) {
    let push = |rows: &mut Vec<Row>, kind: RowKind, top: f32| {
        rows.push(Row {
            kind,
            top,
            indent,
            quotes,
            marker: None,
        })
    };
    for block in blocks {
        match block {
            Block::Heading(level, text) => {
                let top = match level {
                    1 | 2 => 24.,
                    3 => 20.,
                    _ => 16.,
                };
                push(rows, RowKind::Heading(*level, text.clone()), top);
            }
            Block::Paragraph(text) => push(rows, RowKind::Paragraph(text.clone()), 10.),
            Block::Image(image) => push(rows, RowKind::Image(image.clone()), 10.),
            Block::Code(code) => {
                for line in 0..code.document.line_count() {
                    for bytes in fragments(code.document.line(line)) {
                        let top = if line == 0 && bytes.start == 0 {
                            12.
                        } else {
                            0.
                        };
                        let kind = RowKind::Code {
                            code: code.clone(),
                            line,
                            bytes,
                        };
                        push(rows, kind, top);
                    }
                }
            }
            Block::Quote(inner) => {
                let first = rows.len();
                flatten(inner, indent, quotes + 1, rows);
                if let Some(row) = rows.get_mut(first) {
                    row.top = row.top.max(10.);
                }
            }
            Block::List { start, items } => {
                for (index, item) in items.iter().enumerate() {
                    let first = rows.len();
                    flatten(&item.blocks, indent + 1, quotes, rows);
                    if rows.len() == first {
                        push(rows, RowKind::Paragraph(Inline::default()), 0.);
                        rows[first].indent = indent + 1;
                    }
                    let row = &mut rows[first];
                    row.top = if index == 0 { 8. } else { 4. };
                    row.marker = Some(match (item.task, start) {
                        (Some(checked), _) => Marker::Task(checked),
                        (None, Some(start)) => Marker::Number(start + index as u64),
                        (None, None) => Marker::Bullet,
                    });
                }
            }
            Block::Table {
                alignments,
                header,
                rows: body,
            } => {
                let alignments = Arc::new(alignments.clone());
                let last = body.len();
                for (index, cells) in std::iter::once(header).chain(body).enumerate() {
                    let kind = RowKind::TableRow {
                        alignments: alignments.clone(),
                        cells: cells.clone(),
                        head: index == 0,
                        last: index == last,
                    };
                    push(rows, kind, if index == 0 { 12. } else { 0. });
                }
            }
            Block::Rule => push(rows, RowKind::Rule, 16.),
            Block::Html(html) => {
                for (index, line) in html.lines().enumerate() {
                    push(
                        rows,
                        RowKind::Html(line.to_owned()),
                        if index == 0 { 10. } else { 0. },
                    );
                }
            }
        }
    }
}

/// The bytes of `row`'s text (`len` long) the selection from `start` to
/// `end` covers; all of a table row it touches.
pub(super) fn selected_range(
    row: &Row,
    index: usize,
    start: Position,
    end: Position,
    len: usize,
) -> Range<usize> {
    if index < start.row || index > end.row || start == end {
        return 0..0;
    }
    if matches!(row.kind, RowKind::TableRow { .. }) {
        return 0..len;
    }
    let from = if index == start.row {
        start.offset.min(len)
    } else {
        0
    };
    let to = if index == end.row {
        end.offset.min(len)
    } else {
        len
    };
    from..to.max(from)
}
