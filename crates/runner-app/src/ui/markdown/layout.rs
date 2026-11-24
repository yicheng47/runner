use std::ops::Range;
use std::sync::Arc;

use pulldown_cmark::Alignment;
use unicode_width::UnicodeWidthChar as _;

use super::view::Position;
use super::{cell_text, Block, Cell, CodeBlock, ImageRef, Inline, FRAGMENT_BYTES};

/// Which look's spacing and paragraph-splitting rules [`flatten`] applies.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum LayoutLook {
    Document,
    Compact,
}

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
    /// Space above the row, in pixels (outside quote bars in Compact).
    pub(super) top: f32,
    /// Space above the row inside quote bars in Compact, keeping the quote
    /// border continuous across blocks inside a single quote.
    pub(super) inner_top: f32,
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
            && self.inner_top.to_bits() == other.inner_top.to_bits()
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

/// Splits an [`Inline`] at `'\n'` boundaries into one [`Inline`] per line,
/// slicing style ranges and sharing link destinations across lines in O(n).
fn split_inline_lines(inline: &Inline) -> Vec<Inline> {
    if !inline.text.contains('\n') {
        return vec![inline.clone()];
    }
    let text = inline.text.strip_suffix('\n').unwrap_or(&inline.text);
    let mut out = Vec::new();
    let mut style_idx = 0;
    let mut link_idx = 0;
    let mut line_start = 0;

    for segment in text.split('\n') {
        let line_end = line_start + segment.len();
        let mut line = Inline {
            text: segment.to_owned(),
            styles: Vec::new(),
            links: Vec::new(),
        };

        while style_idx < inline.styles.len() && inline.styles[style_idx].0.end <= line_start {
            style_idx += 1;
        }
        let mut s = style_idx;
        while s < inline.styles.len() && inline.styles[s].0.start < line_end {
            let (range, style) = &inline.styles[s];
            let start = range.start.max(line_start) - line_start;
            let end = range.end.min(line_end) - line_start;
            if start < end {
                line.styles.push((start..end, *style));
            }
            s += 1;
        }

        while link_idx < inline.links.len() && inline.links[link_idx].0.end <= line_start {
            link_idx += 1;
        }
        let mut l = link_idx;
        while l < inline.links.len() && inline.links[l].0.start < line_end {
            let (range, url) = &inline.links[l];
            let start = range.start.max(line_start) - line_start;
            let end = range.end.min(line_end) - line_start;
            if start < end {
                line.links.push((start..end, Arc::clone(url)));
            }
            l += 1;
        }

        out.push(line);
        line_start = line_end + 1;
    }

    out
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CompactKind {
    Paragraph,
    ListItem,
    Other,
}

fn compact_margins(block: &Block) -> (CompactKind, f32, f32) {
    match block {
        Block::Paragraph(_) => (CompactKind::Paragraph, 6., 6.),
        Block::List { .. } => (CompactKind::ListItem, 6., 6.),
        Block::Heading(1, _) => (CompactKind::Other, 12., 6.),
        Block::Heading(2, _) => (CompactKind::Other, 12., 4.),
        Block::Heading(_, _) => (CompactKind::Other, 8., 4.),
        Block::Quote(_) | Block::Code(_) | Block::Table { .. } => (CompactKind::Other, 8., 8.),
        Block::Rule => (CompactKind::Other, 12., 12.),
        Block::Html(_) | Block::Image(_) => (CompactKind::Other, 6., 6.),
    }
}

fn collapse_compact_top(previous: Option<(CompactKind, f32)>, kind: CompactKind, top: f32) -> f32 {
    match previous {
        Some((CompactKind::ListItem, _)) if kind == CompactKind::ListItem => 4.,
        Some((CompactKind::Paragraph, bottom)) if kind == CompactKind::Paragraph => bottom + top,
        Some((_, bottom)) => bottom.max(top),
        None => top,
    }
}

fn document_top(block: &Block) -> f32 {
    match block {
        Block::Heading(1 | 2, _) => 24.,
        Block::Heading(3, _) => 20.,
        Block::Heading(_, _) => 16.,
        Block::Paragraph(_) | Block::Image(_) | Block::Html(_) => 10.,
        Block::Code(_) | Block::Table { .. } => 12.,
        Block::Rule => 16.,
        Block::Quote(_) | Block::List { .. } => 0.,
    }
}

/// Lays `blocks` out as rows for `look` and returns the trailing bottom
/// padding (`0.` in Document, last top-level block's bottom margin in Compact).
pub(super) fn flatten(blocks: &[Block], look: LayoutLook, rows: &mut Vec<Row>) -> f32 {
    flatten_blocks(blocks, look, 0, 0, false, rows)
}

fn flatten_blocks(
    blocks: &[Block],
    look: LayoutLook,
    indent: u8,
    quotes: u8,
    first_top_zero: bool,
    rows: &mut Vec<Row>,
) -> f32 {
    let compact = look == LayoutLook::Compact;
    let push = |rows: &mut Vec<Row>, kind: RowKind, gap: f32| {
        let (top, inner_top) = if compact && quotes > 0 {
            (0., gap)
        } else {
            (gap, 0.)
        };
        rows.push(Row {
            kind,
            top,
            inner_top,
            indent,
            quotes,
            marker: None,
        })
    };
    let mut previous = None::<(CompactKind, f32)>;
    for block in blocks {
        let (gap, next_prev) = match look {
            LayoutLook::Document => (document_top(block), None),
            LayoutLook::Compact => {
                let (kind, raw_top, raw_bottom) = compact_margins(block);
                let gap = if previous.is_none() && first_top_zero {
                    0.
                } else if indent > 0 && kind == CompactKind::ListItem {
                    4.
                } else {
                    collapse_compact_top(previous, kind, raw_top)
                };
                (gap, Some((kind, raw_bottom)))
            }
        };
        let first = rows.len();
        match block {
            Block::Heading(level, text) => {
                push(rows, RowKind::Heading(*level, text.clone()), gap);
            }
            Block::Paragraph(text) => {
                if compact {
                    for (line_index, line) in split_inline_lines(text).into_iter().enumerate() {
                        push(
                            rows,
                            RowKind::Paragraph(line),
                            if line_index == 0 { gap } else { 0. },
                        );
                    }
                } else {
                    push(rows, RowKind::Paragraph(text.clone()), gap);
                }
            }
            Block::Image(image) => push(rows, RowKind::Image(image.clone()), gap),
            Block::Code(code) => {
                for line in 0..code.document.line_count() {
                    for bytes in fragments(code.document.line(line)) {
                        let top = if line == 0 && bytes.start == 0 {
                            gap
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
                flatten_blocks(inner, look, indent, quotes + 1, compact, rows);
                if let Some(row) = rows.get_mut(first) {
                    if compact {
                        if quotes == 0 {
                            row.top = gap;
                        } else {
                            row.inner_top = gap;
                        }
                    } else {
                        row.top = row.top.max(10.);
                    }
                }
            }
            Block::List { start, items } => {
                for (index, item) in items.iter().enumerate() {
                    let item_first = rows.len();
                    flatten_blocks(&item.blocks, look, indent + 1, quotes, compact, rows);
                    if rows.len() == item_first {
                        push(rows, RowKind::Paragraph(Inline::default()), 0.);
                        rows[item_first].indent = indent + 1;
                    }
                    let item_gap = match look {
                        LayoutLook::Document => {
                            if index == 0 {
                                8.
                            } else {
                                4.
                            }
                        }
                        LayoutLook::Compact => {
                            if index == 0 {
                                gap
                            } else {
                                4.
                            }
                        }
                    };
                    let row = &mut rows[item_first];
                    if compact && quotes > 0 {
                        row.inner_top = item_gap;
                    } else {
                        row.top = item_gap;
                    }
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
                    push(rows, kind, if index == 0 { gap } else { 0. });
                }
            }
            Block::Rule => push(rows, RowKind::Rule, gap),
            Block::Html(html) => {
                for (index, line) in html.lines().enumerate() {
                    push(
                        rows,
                        RowKind::Html(line.to_owned()),
                        if index == 0 { gap } else { 0. },
                    );
                }
            }
        }
        if compact && rows.len() > first {
            previous = next_prev;
        }
    }
    if compact {
        previous.map_or(0., |(_, bottom)| bottom)
    } else {
        0.
    }
}

fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The bytes of `row`'s text the selection from `start` to `end` covers;
/// all of a table row it touches.
pub(super) fn selected_range(
    row: &Row,
    index: usize,
    start: Position,
    end: Position,
    text: &str,
) -> Range<usize> {
    if index < start.row || index > end.row || start == end {
        return 0..0;
    }
    if matches!(row.kind, RowKind::TableRow { .. }) {
        return 0..text.len();
    }
    let from = if index == start.row {
        floor_char_boundary(text, start.offset)
    } else {
        0
    };
    let to = if index == end.row {
        floor_char_boundary(text, end.offset)
    } else {
        text.len()
    };
    from..to.max(from)
}
