//! Shared Markdown parser, block model, row layout and GPUI renderer (#1342).
//!
//! Parsed with `pulldown-cmark` into a tree of [`Block`]s, laid out as
//! virtualized [`Row`]s (a code line, list item, or table row each one) so long
//! documents and blocks draw only what is in view.

mod draw;
mod feed;
mod layout;
mod links;
mod parse;
mod view;

#[cfg(test)]
mod tests;

use std::ops::Range;
use std::sync::Arc;

use pulldown_cmark::Alignment;
use unicode_width::UnicodeWidthChar as _;

use crate::surfaces::files::document::{Document, TAB_WIDTH};
use crate::surfaces::files::highlight::Highlights;

pub(crate) use draw::reset_compact_press;
pub(crate) use feed::{
    render_compact, FeedMarkdown, FeedSelection, FeedSelectionHandler, FeedSelectionPhase,
};
#[cfg(test)]
pub(crate) use feed::{LINE_COUNT, PARSE_COUNT, RENDER_COUNT};
#[cfg(test)]
pub(crate) use layout::Marker;
pub(crate) use layout::{Row, RowKind};
pub(crate) use links::{image_format, LinkTarget};
#[cfg(test)]
pub(crate) use links::{route, route_web};
#[cfg(test)]
pub(crate) use view::DRAWN_CODE_BYTES;
pub(crate) use view::{MarkdownView, OpenLink, Position};

/// How many quotes, lists and list items may nest. Parsing, laying out and
/// dropping a page each recurse once per level, and a file of a few KB can ask
/// for tens of thousands, which would overflow the stack and end the app.
const MAX_NESTING: usize = 32;

/// The most bytes of a code line one row draws: a longer line is split
/// across rows, so one long line never makes a frame shape all of it.
const FRAGMENT_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InlineStyle {
    pub(crate) strong: bool,
    pub(crate) emphasis: bool,
    pub(crate) strike: bool,
    pub(crate) code: bool,
    pub(crate) link: bool,
}

/// A run of text with its styles and the links in it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Inline {
    pub(crate) text: String,
    /// Consecutive, covering `text`.
    pub(crate) styles: Vec<(Range<usize>, InlineStyle)>,
    pub(crate) links: Vec<(Range<usize>, Arc<str>)>,
}

impl Inline {
    pub(super) fn push(&mut self, text: &str, style: InlineStyle) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        match self.styles.last_mut() {
            Some((range, last)) if *last == style && range.end == start => {
                range.end = self.text.len()
            }
            _ => self.styles.push((start..self.text.len(), style)),
        }
    }

    pub(super) fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// A local image, by its path from the root, and the link it sits in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImageRef {
    pub(crate) path: String,
    pub(crate) alt: String,
    pub(crate) link: Option<Arc<str>>,
}

/// Inline content: text, or a local image drawn as itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Piece {
    Text(Inline),
    Image(ImageRef),
}

/// A table cell's content.
pub(crate) type Cell = Vec<Piece>;

pub(super) fn cell_text(cell: &[Piece]) -> String {
    cell.iter()
        .map(|piece| match piece {
            Piece::Text(inline) => inline.text.as_str(),
            Piece::Image(image) => image.alt.as_str(),
        })
        .collect()
}

pub(crate) struct CodeBlock {
    pub(crate) document: Document,
    pub(crate) highlights: Option<Highlights>,
}

impl std::fmt::Debug for CodeBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("CodeBlock")
            .field(&self.document.text())
            .finish()
    }
}

impl CodeBlock {
    /// `bytes` of a line as drawn, tabs as spaces to their stops, in runs
    /// of one highlight capture.
    pub(super) fn fragment(
        &self,
        line: usize,
        bytes: Range<usize>,
    ) -> (String, Vec<(usize, Option<u16>)>) {
        let spans = self
            .highlights
            .as_ref()
            .map_or(&[][..], |highlights| highlights.line(line));
        let source = self.document.line(line);
        let mut column = self.document.column_at(line, bytes.start);
        let mut span = spans.partition_point(|found| found.end as usize <= bytes.start);
        let mut text = String::new();
        let mut runs: Vec<(usize, Option<u16>)> = Vec::new();
        for (index, character) in source[bytes.clone()].char_indices() {
            let offset = bytes.start + index;
            while spans
                .get(span)
                .is_some_and(|found| found.end as usize <= offset)
            {
                span += 1;
            }
            let capture = spans
                .get(span)
                .filter(|found| found.start as usize <= offset)
                .map(|found| found.capture);
            let before = text.len();
            if character == '\t' {
                let cells = TAB_WIDTH - column % TAB_WIDTH;
                text.extend(std::iter::repeat_n(' ', cells));
                column += cells;
            } else {
                text.push(character);
                column += character.width().unwrap_or(1);
            }
            let added = text.len() - before;
            match runs.last_mut() {
                Some(last) if last.1 == capture => last.0 += added,
                _ => runs.push((added, capture)),
            }
        }
        (text, runs)
    }
}

#[derive(Debug)]
pub(crate) struct ListItem {
    /// A task list item's box: whether it is checked.
    pub(crate) task: Option<bool>,
    pub(crate) blocks: Vec<Block>,
}

#[derive(Debug)]
pub(crate) enum Block {
    Heading(u8, Inline),
    Paragraph(Inline),
    /// A local image on a line of its own.
    Image(ImageRef),
    Code(Arc<CodeBlock>),
    Quote(Vec<Block>),
    List {
        /// The first number of an ordered list.
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Table {
        alignments: Vec<Alignment>,
        header: Vec<Cell>,
        rows: Vec<Vec<Cell>>,
    },
    Rule,
    /// Block HTML, shown as written.
    Html(String),
}

/// Markdown parsed into blocks and laid out as rows. Slow on a large file
/// with syntax highlighting: run [`Page::parse`] off the UI thread.
#[derive(Debug, Default)]
pub(crate) struct Page {
    /// The parsed blocks the rows were laid out from, for tests.
    #[cfg(test)]
    pub(crate) blocks: Vec<Block>,
    pub(crate) rows: Vec<Row>,
    /// Trailing bottom padding for the Compact look (in pixels).
    pub(super) bottom: f32,
}

impl Page {
    /// Parses `text`, the Markdown file at `path` from the root, for the
    /// Document look (Files preview).
    pub(crate) fn parse(text: &str, path: &str) -> Self {
        let blocks = parse::parse_document_blocks(text, path);
        let mut rows = Vec::new();
        let bottom = layout::flatten(&blocks, layout::LayoutLook::Document, &mut rows);
        Self {
            #[cfg(test)]
            blocks,
            rows,
            bottom,
        }
    }

    /// Parses `text` for the Compact look (feed, question cards, prompts,
    /// Skills preview and Info popover).
    pub(crate) fn parse_compact(text: &str) -> Self {
        #[cfg(test)]
        feed::PARSE_COUNT.set(feed::PARSE_COUNT.get() + 1);
        let blocks = parse::parse_compact_blocks(text);
        let mut rows = Vec::new();
        let bottom = layout::flatten(&blocks, layout::LayoutLook::Compact, &mut rows);
        Self {
            #[cfg(test)]
            blocks,
            rows,
            bottom,
        }
    }

    /// The selected text, rows joined by line breaks without one before the
    /// continued piece of a long code line; a table row is taken whole, its
    /// cells joined by tabs. Returns `None` only for a collapsed or
    /// out-of-range selection, preserving `Some("")` when distinct endpoints
    /// cover zero bytes (e.g. across a code-fragment boundary).
    pub(crate) fn selected_text(&self, anchor: Position, head: Position) -> Option<String> {
        let (start, end) = (anchor.min(head), anchor.max(head));
        if start == end || start.row >= self.rows.len() {
            return None;
        }
        let mut copied = String::new();
        for row in start.row..=end.row.min(self.rows.len().saturating_sub(1)) {
            let line = &self.rows[row];
            let text = line.text();
            let range = layout::selected_range(line, row, start, end, &text);
            if row > start.row && !line.continues() {
                copied.push('\n');
            }
            copied.push_str(&text[range]);
        }
        Some(copied)
    }

    /// Whether row continuation and rendered row text match `other`, keeping
    /// existing row/offset selection coordinates valid.
    pub(crate) fn matches_text(&self, other: &Page) -> bool {
        self.rows.len() == other.rows.len()
            && self.rows.iter().zip(&other.rows).all(|(left, right)| {
                left.continues() == right.continues() && left.text() == right.text()
            })
    }

    /// Whether every rendered row (styles, links, markers, code highlights,
    /// layout spacing, and images) matches `other`.
    pub(crate) fn same_rendered(&self, other: &Page) -> bool {
        if self.rows.len() != other.rows.len() {
            return false;
        }
        let mut last_code: Option<(*const CodeBlock, *const CodeBlock)> = None;
        self.rows.iter().zip(&other.rows).all(|(left, right)| {
            if !left.same_layout(right) {
                return false;
            }
            match (&left.kind, &right.kind) {
                (RowKind::Heading(la, ia), RowKind::Heading(lb, ib)) => la == lb && ia == ib,
                (RowKind::Paragraph(ia), RowKind::Paragraph(ib)) => ia == ib,
                (RowKind::Image(ia), RowKind::Image(ib)) => ia == ib,
                (
                    RowKind::Code {
                        code: a,
                        line: la,
                        bytes: ba,
                    },
                    RowKind::Code {
                        code: b,
                        line: lb,
                        bytes: bb,
                    },
                ) => {
                    if la != lb || ba != bb {
                        return false;
                    }
                    let pair = (Arc::as_ptr(a), Arc::as_ptr(b));
                    if Arc::ptr_eq(a, b) || last_code == Some(pair) {
                        return true;
                    }
                    let same =
                        a.document.text() == b.document.text() && a.highlights == b.highlights;
                    if same {
                        last_code = Some(pair);
                    }
                    same
                }
                (
                    RowKind::TableRow {
                        alignments: aa,
                        cells: ca,
                        head: ha,
                        last: la,
                    },
                    RowKind::TableRow {
                        alignments: ab,
                        cells: cb,
                        head: hb,
                        last: lb,
                    },
                ) => ha == hb && la == lb && ca == cb && (Arc::ptr_eq(aa, ab) || aa == ab),
                (RowKind::Rule, RowKind::Rule) => true,
                (RowKind::Html(ha), RowKind::Html(hb)) => ha == hb,
                _ => false,
            }
        })
    }

    /// Every local image the page shows, by its path from the root.
    pub(crate) fn images(&self) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        let mut add = |image: &ImageRef| {
            if !found.contains(&image.path) {
                found.push(image.path.clone());
            }
        };
        for row in &self.rows {
            match &row.kind {
                RowKind::Image(image) => add(image),
                RowKind::TableRow { cells, .. } => {
                    for piece in cells.iter().flatten() {
                        if let Piece::Image(image) = piece {
                            add(image);
                        }
                    }
                }
                _ => {}
            }
        }
        found
    }
}
