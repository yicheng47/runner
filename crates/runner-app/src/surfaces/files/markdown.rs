//! A Markdown file rendered in the file viewer: parsed with `pulldown-cmark`
//! off the UI thread into blocks, its code blocks highlighted as code, and
//! laid out as rows (a code line, a list item, a table row each one) that a
//! virtualized list draws, so a long file or block draws only what is in
//! view. Text selects by dragging across rows and copies with ⌘C.
//!
//! Beyond CommonMark it renders GFM tables, task lists (read-only) and
//! strikethrough. HTML, inline or block, shows as its literal text, and
//! footnotes and math stay as written.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, img, px, relative, svg, AnyElement, App, Context, FocusHandle, Font, FontStyle,
    FontWeight, Hsla, ImageSource, InteractiveText, ListAlignment, ListOffset, ListState, Pixels,
    SharedString, StrikethroughStyle, StyledText, TextRun, UnderlineStyle, Window,
};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Tag, TagEnd};
use unicode_width::UnicodeWidthChar as _;

use super::document::{Document, TAB_WIDTH};
use super::highlight::{self, Highlights};
use super::syntax::SyntaxTheme;
use crate::theme;

/// The rendered page's side margins and the gap above its first block.
const MARGIN: f32 = 20.;

/// Where a link in the page leads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LinkTarget {
    /// Opens in the browser.
    Url(String),
    /// A file in the root, by its path from the root: opens in a new tab.
    File(String),
    /// Nothing to open: an anchor, another scheme, or a path that leaves
    /// the root.
    None,
}

/// Where `href`, written in the Markdown file at `from` (a path from the
/// root), leads. A relative path resolves against the file's folder and a
/// path starting with `/` against the root, as on a forge; neither may leave
/// the root.
pub(crate) fn route(href: &str, from: &str) -> LinkTarget {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return LinkTarget::None;
    }
    if let Some((scheme, _)) = href.split_once(':') {
        let is_scheme = scheme.len() > 1
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if is_scheme || scheme.len() == 1 {
            return match scheme.to_ascii_lowercase().as_str() {
                "http" | "https" | "mailto" => LinkTarget::Url(href.to_owned()),
                _ => LinkTarget::None,
            };
        }
    }
    let path = href.split(['#', '?']).next().unwrap_or_default();
    if path.is_empty() || path.contains('\\') {
        return LinkTarget::None;
    }
    let path = percent_decode(path);
    let mut parts: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        let folder = from.rsplit_once('/').map_or("", |(folder, _)| folder);
        folder.split('/').filter(|part| !part.is_empty()).collect()
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return LinkTarget::None;
                }
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        LinkTarget::None
    } else {
        LinkTarget::File(parts.join("/"))
    }
}

/// `%20` and its kind back to the bytes they stand for; anything that is
/// not a valid escape stays as written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|b| (*b as char).to_digit(16));
        match (bytes[index], hex(index + 1), hex(index + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, ..) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

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
    pub(crate) links: Vec<(Range<usize>, String)>,
}

impl Inline {
    fn push(&mut self, text: &str, style: InlineStyle) {
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

    fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// A local image, by its path from the root, and the link it sits in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImageRef {
    pub(crate) path: String,
    pub(crate) alt: String,
    pub(crate) link: Option<String>,
}

/// Inline content: text, or a local image drawn as itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Piece {
    Text(Inline),
    Image(ImageRef),
}

/// A table cell's content.
pub(crate) type Cell = Vec<Piece>;

fn cell_text(cell: &[Piece]) -> String {
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

/// The most bytes of a code line one row draws: a longer line is split
/// across rows, so one long line never makes a frame shape all of it.
const FRAGMENT_BYTES: usize = 512;

impl CodeBlock {
    /// `bytes` of a line as drawn, tabs as spaces to their stops, in runs
    /// of one highlight capture.
    fn fragment(&self, line: usize, bytes: Range<usize>) -> (String, Vec<(usize, Option<u16>)>) {
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

/// A line's bytes in pieces of at most about [`FRAGMENT_BYTES`], split on
/// character boundaries and never before a combining mark.
fn fragments(line: &str) -> Vec<Range<usize>> {
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
    top: f32,
    /// How many lists deep the row sits.
    indent: u8,
    /// How many quotes deep the row sits.
    quotes: u8,
    marker: Option<Marker>,
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
    fn continues(&self) -> bool {
        matches!(&self.kind, RowKind::Code { bytes, .. } if bytes.start > 0)
    }

    /// The row's text as selection and copy see it.
    fn text(&self) -> String {
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
}

/// A file's Markdown parsed into blocks and laid out as rows, its code
/// highlighted. Slow on a large file: run it off the UI thread.
#[derive(Debug, Default)]
pub(crate) struct Page {
    /// The parsed blocks the rows were laid out from, for tests.
    #[cfg(test)]
    pub(crate) blocks: Vec<Block>,
    pub(crate) rows: Vec<Row>,
}

impl Page {
    /// Parses `text`, the Markdown file at `path` from the root.
    pub(crate) fn parse(text: &str, path: &str) -> Self {
        let options =
            Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
        let events: Vec<Event> = pulldown_cmark::Parser::new_ext(text, options).collect();
        let mut parser = Parser {
            events,
            at: 0,
            path,
        };
        let blocks = parser.blocks(None);
        let mut rows = Vec::new();
        flatten(&blocks, 0, 0, &mut rows);
        Self {
            #[cfg(test)]
            blocks,
            rows,
        }
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

/// Lays `blocks` out as rows, `indent` lists and `quotes` quotes deep.
fn flatten(blocks: &[Block], indent: u8, quotes: u8, rows: &mut Vec<Row>) {
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

struct Parser<'a, 'p> {
    events: Vec<Event<'a>>,
    at: usize,
    /// The Markdown file's path from the root, for its images.
    path: &'p str,
}

impl<'a> Parser<'a, '_> {
    fn next(&mut self) -> Option<Event<'a>> {
        let event = self.events.get(self.at).cloned();
        self.at += 1;
        event
    }

    fn peek(&self) -> Option<&Event<'a>> {
        self.events.get(self.at)
    }

    /// Blocks until `end`, or to the end of the document.
    fn blocks(&mut self, end: Option<TagEnd>) -> Vec<Block> {
        let mut blocks = Vec::new();
        while let Some(event) = self.peek() {
            match event {
                Event::End(tag) if Some(*tag) == end => {
                    self.at += 1;
                    break;
                }
                Event::Start(Tag::Paragraph) => {
                    self.at += 1;
                    let pieces = self.inline(Some(TagEnd::Paragraph), true);
                    push_pieces(&mut blocks, pieces);
                }
                Event::Start(Tag::Heading { .. }) => {
                    let end = TagEnd::Heading(match self.next() {
                        Some(Event::Start(Tag::Heading { level, .. })) => level,
                        _ => unreachable!(),
                    });
                    let level = match end {
                        TagEnd::Heading(level) => level as u8,
                        _ => unreachable!(),
                    };
                    let text = flatten_inline(self.inline(Some(end), false));
                    blocks.push(Block::Heading(level, text));
                }
                Event::Start(Tag::BlockQuote(_)) => {
                    let kind = match self.next() {
                        Some(Event::Start(Tag::BlockQuote(kind))) => kind,
                        _ => unreachable!(),
                    };
                    let inner = self.blocks(Some(TagEnd::BlockQuote(kind)));
                    blocks.push(Block::Quote(inner));
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    let language = match kind {
                        CodeBlockKind::Fenced(info) => info
                            .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
                            .next()
                            .map(str::to_owned),
                        CodeBlockKind::Indented => None,
                    };
                    self.at += 1;
                    let mut text = String::new();
                    while let Some(event) = self.next() {
                        match event {
                            Event::Text(code) => text.push_str(&code),
                            Event::End(TagEnd::CodeBlock) => break,
                            _ => {}
                        }
                    }
                    let document =
                        Document::decode(text.strip_suffix('\n').unwrap_or(&text).as_bytes());
                    let highlights = language
                        .as_deref()
                        .and_then(highlight::grammar_named)
                        .and_then(|grammar| highlight::highlight(&document, grammar, None));
                    blocks.push(Block::Code(Arc::new(CodeBlock {
                        document,
                        highlights,
                    })));
                }
                Event::Start(Tag::List(start)) => {
                    let start = *start;
                    self.at += 1;
                    let mut items = Vec::new();
                    while let Some(event) = self.next() {
                        match event {
                            Event::Start(Tag::Item) => {
                                let task = self.task_marker();
                                let blocks = self.blocks(Some(TagEnd::Item));
                                items.push(ListItem { task, blocks });
                            }
                            Event::End(TagEnd::List(_)) => break,
                            _ => {}
                        }
                    }
                    blocks.push(Block::List { start, items });
                }
                Event::Start(Tag::Table(alignments)) => {
                    let alignments = alignments.clone();
                    self.at += 1;
                    let mut header = Vec::new();
                    let mut rows = Vec::new();
                    let mut row: Vec<Cell> = Vec::new();
                    while let Some(event) = self.next() {
                        match event {
                            Event::Start(Tag::TableCell) => {
                                let mut cell = self.inline(Some(TagEnd::TableCell), true);
                                cell.retain(
                                    |piece| !matches!(piece, Piece::Text(text) if text.is_blank()),
                                );
                                row.push(cell);
                            }
                            Event::End(TagEnd::TableHead) => header = std::mem::take(&mut row),
                            Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
                            Event::End(TagEnd::Table) => break,
                            _ => {}
                        }
                    }
                    blocks.push(Block::Table {
                        alignments,
                        header,
                        rows,
                    });
                }
                Event::Start(Tag::HtmlBlock) => {
                    self.at += 1;
                    let mut html = String::new();
                    while let Some(event) = self.next() {
                        match event {
                            Event::Html(text) | Event::Text(text) => html.push_str(&text),
                            Event::End(TagEnd::HtmlBlock) => break,
                            _ => {}
                        }
                    }
                    blocks.push(Block::Html(html.trim_end().to_owned()));
                }
                Event::Rule => {
                    self.at += 1;
                    blocks.push(Block::Rule);
                }
                Event::Start(_) | Event::End(_) if !is_inline(event) => {
                    // A block this page does not draw: its content still shows.
                    self.at += 1;
                }
                _ => {
                    // Text straight inside a tight list's item.
                    let pieces = self.inline(None, true);
                    push_pieces(&mut blocks, pieces);
                }
            }
        }
        blocks
    }

    /// A task list item's box, just inside the item: straight after it in
    /// a tight list, or inside its first paragraph in a loose one.
    fn task_marker(&mut self) -> Option<bool> {
        match (self.peek(), self.events.get(self.at + 1)) {
            (Some(Event::TaskListMarker(checked)), _) => {
                let checked = *checked;
                self.at += 1;
                Some(checked)
            }
            (Some(Event::Start(Tag::Paragraph)), Some(Event::TaskListMarker(checked))) => {
                let checked = *checked;
                self.events.remove(self.at + 1);
                Some(checked)
            }
            _ => None,
        }
    }

    /// Inline content until `end`, or, with none, until the next block.
    /// With `images`, a local image becomes a piece of its own, linked when
    /// it sits in a link; without, it shows its alt text.
    fn inline(&mut self, end: Option<TagEnd>, images: bool) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut current = Inline::default();
        let mut style = InlineStyle::default();
        let mut styles: Vec<InlineStyle> = Vec::new();
        let mut links: Vec<(usize, String)> = Vec::new();
        while let Some(event) = self.peek() {
            if end.is_none() && !is_inline(event) {
                break;
            }
            let Some(event) = self.next() else {
                break;
            };
            match event {
                Event::End(tag) if Some(tag) == end => break,
                Event::Text(text) => current.push(&text, style),
                Event::Code(text) => current.push(
                    &text,
                    InlineStyle {
                        code: true,
                        ..style
                    },
                ),
                Event::Html(text) | Event::InlineHtml(text) => current.push(&text, style),
                Event::InlineMath(text) => current.push(&format!("${text}$"), style),
                Event::DisplayMath(text) => current.push(&format!("$${text}$$"), style),
                Event::FootnoteReference(name) => current.push(&format!("[^{name}]"), style),
                Event::SoftBreak => current.push(" ", style),
                Event::HardBreak => current.push("\n", style),
                Event::TaskListMarker(checked) => {
                    current.push(if checked { "[x] " } else { "[ ] " }, style)
                }
                Event::Start(Tag::Emphasis) => {
                    styles.push(style);
                    style.emphasis = true;
                }
                Event::Start(Tag::Strong) => {
                    styles.push(style);
                    style.strong = true;
                }
                Event::Start(Tag::Strikethrough) => {
                    styles.push(style);
                    style.strike = true;
                }
                Event::Start(Tag::Link { dest_url, .. }) => {
                    styles.push(style);
                    style.link = true;
                    links.push((current.text.len(), dest_url.into_string()));
                }
                Event::End(TagEnd::Link) => {
                    style = styles.pop().unwrap_or_default();
                    if let Some((start, url)) = links.pop() {
                        if current.text.len() > start {
                            current.links.push((start..current.text.len(), url));
                        }
                    }
                }
                Event::Start(Tag::Image { dest_url, .. }) => {
                    let mut alt = String::new();
                    let mut depth = 1;
                    while let Some(event) = self.next() {
                        match event {
                            Event::Start(Tag::Image { .. }) => depth += 1,
                            Event::End(TagEnd::Image) => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                            _ => {}
                        }
                    }
                    match route(&dest_url, self.path) {
                        LinkTarget::File(path) if images => {
                            // The text so far keeps its part of the open links,
                            // whose text resumes after the image.
                            for (start, url) in &mut links {
                                if current.text.len() > *start {
                                    current
                                        .links
                                        .push((*start..current.text.len(), url.clone()));
                                }
                                *start = 0;
                            }
                            if !current.is_blank() {
                                pieces.push(Piece::Text(std::mem::take(&mut current)));
                            }
                            current = Inline::default();
                            pieces.push(Piece::Image(ImageRef {
                                path,
                                alt,
                                link: links.last().map(|(_, url)| url.clone()),
                            }));
                        }
                        _ => current.push(&alt, style),
                    }
                }
                Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough) => {
                    style = styles.pop().unwrap_or_default();
                }
                _ => {}
            }
        }
        if !current.is_blank() || pieces.is_empty() {
            pieces.push(Piece::Text(current));
        }
        pieces
    }
}

fn is_inline(event: &Event) -> bool {
    match event {
        Event::Start(tag) => matches!(
            tag,
            Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough
                | Tag::Superscript
                | Tag::Subscript
                | Tag::Link { .. }
                | Tag::Image { .. }
        ),
        Event::End(tag) => matches!(
            tag,
            TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Superscript
                | TagEnd::Subscript
                | TagEnd::Link
                | TagEnd::Image
        ),
        Event::Rule => false,
        _ => true,
    }
}

fn push_pieces(blocks: &mut Vec<Block>, pieces: Vec<Piece>) {
    for piece in pieces {
        match piece {
            Piece::Text(inline) if inline.is_blank() => {}
            Piece::Text(inline) => blocks.push(Block::Paragraph(inline)),
            Piece::Image(image) => blocks.push(Block::Image(image)),
        }
    }
}

fn flatten_inline(pieces: Vec<Piece>) -> Inline {
    pieces
        .into_iter()
        .find_map(|piece| match piece {
            Piece::Text(inline) => Some(inline),
            Piece::Image(_) => None,
        })
        .unwrap_or_default()
}

// ── Drawing ────────────────────────────────────────────────────────────

/// Opens a link's target: a URL in the browser, a file in a new tab.
pub(crate) type OpenLink = Rc<dyn Fn(LinkTarget, &mut Window, &mut App)>;

/// A place in the page's text: a row and a byte offset in its text.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Position {
    pub(crate) row: usize,
    pub(crate) offset: usize,
}

pub(crate) struct MarkdownView {
    page: Arc<Page>,
    /// The Markdown file's path from the root, for its links.
    path: String,
    /// Decoded local images, by their path from the root.
    images: HashMap<String, Arc<gpui::Image>>,
    list: ListState,
    focus: FocusHandle,
    open_link: Option<OpenLink>,
    /// The anchor and the head, which moves.
    selection: Option<(Position, Position)>,
    dragging: bool,
    /// Where the left button went down in the page, and whether it has
    /// moved since: a press that moved was a drag, which opens no link.
    press: Option<gpui::Point<Pixels>>,
    pressed_moved: bool,
    /// The file was deleted: the page is dimmed and its links do nothing.
    deleted: bool,
}

impl MarkdownView {
    pub(crate) fn new(
        page: Arc<Page>,
        path: String,
        images: HashMap<String, Arc<gpui::Image>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = ListState::new(page.rows.len(), ListAlignment::Top, px(400.));
        Self {
            page,
            path,
            images,
            list,
            focus: cx.focus_handle(),
            open_link: None,
            selection: None,
            dragging: false,
            press: None,
            pressed_moved: false,
            deleted: false,
        }
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub(crate) fn set_open_link(&mut self, open_link: OpenLink) {
        self.open_link = Some(open_link);
    }

    /// The file read again: the scroll stays at the row it was on, and a
    /// selection, which names places in the old text, goes.
    pub(crate) fn set_page(
        &mut self,
        page: Arc<Page>,
        images: HashMap<String, Arc<gpui::Image>>,
        cx: &mut Context<Self>,
    ) {
        let old = self.page.rows.len();
        let top = self.list.logical_scroll_top();
        self.page = page;
        self.images = images;
        self.selection = None;
        let count = self.page.rows.len();
        self.list.splice(0..old, count);
        if count > 0 {
            self.list.scroll_to(ListOffset {
                item_ix: top.item_ix.min(count - 1),
                offset_in_item: top.offset_in_item,
            });
        }
        cx.notify();
    }

    pub(crate) fn set_deleted(&mut self, deleted: bool, cx: &mut Context<Self>) {
        if self.deleted != deleted {
            self.deleted = deleted;
            cx.notify();
        }
    }

    /// The selected text, rows joined by line breaks; a table row is taken
    /// whole, its cells joined by tabs.
    pub(crate) fn selected_text(&self) -> Option<String> {
        let (anchor, head) = self.selection?;
        let (start, end) = (anchor.min(head), anchor.max(head));
        if start == end {
            return None;
        }
        let mut copied = String::new();
        for row in start.row..=end.row.min(self.page.rows.len().saturating_sub(1)) {
            let line = &self.page.rows[row];
            let text = line.text();
            let range = selected_range(line, row, start, end, text.len());
            if row > start.row && !line.continues() {
                copied.push('\n');
            }
            copied.push_str(&text[range]);
        }
        Some(copied)
    }

    pub(crate) fn select(&mut self, anchor: Position, head: Position, cx: &mut Context<Self>) {
        self.selection = Some((anchor, head));
        cx.notify();
    }

    fn copy(&mut self, _: &crate::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }

    fn select_all(&mut self, _: &crate::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.page.rows.len().checked_sub(1) else {
            return;
        };
        let end = Position {
            row: last,
            offset: self.page.rows[last].text().len(),
        };
        self.select(Position::default(), end, cx);
    }

    fn begin_selection(
        &mut self,
        at: Position,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        let anchor = match self.selection {
            Some((anchor, _)) if extend => anchor,
            _ => at,
        };
        self.selection = Some((anchor, at));
        self.dragging = true;
        cx.notify();
    }

    /// Whether the press now ending moved: a click on a link then opens
    /// nothing, as the press was a drag.
    fn press_moved(&self) -> bool {
        self.pressed_moved
    }

    fn drag_to(&mut self, at: Position, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        if let Some((anchor, head)) = self.selection {
            if head != at {
                self.selection = Some((anchor, at));
                cx.notify();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn scroll_top(&self) -> ListOffset {
        self.list.logical_scroll_top()
    }

    #[cfg(test)]
    pub(crate) fn scroll_to_row(&mut self, row: usize) {
        self.list.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: px(0.),
        });
    }

    #[cfg(test)]
    pub(crate) fn page(&self) -> &Arc<Page> {
        &self.page
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &str {
        &self.path
    }
}

/// Whether the press that ends in a click moved, as a drag that selects
/// does, so the link under it opens nothing; a selection left from an
/// earlier press does not count.
fn dragged(view: &gpui::WeakEntity<MarkdownView>, cx: &App) -> bool {
    view.upgrade()
        .is_some_and(|view| view.read(cx).press_moved())
}

/// The bytes of `row`'s text (`len` long) the selection from `start` to
/// `end` covers; all of a table row it touches.
fn selected_range(
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

/// `runs` with the bytes in `selected` drawn on the selection colour.
fn with_selection(runs: Vec<TextRun>, selected: Range<usize>, color: Hsla) -> Vec<TextRun> {
    if selected.is_empty() {
        return runs;
    }
    let mut out = Vec::with_capacity(runs.len() + 2);
    let mut at = 0;
    for run in runs {
        let (start, end) = (at, at + run.len);
        at = end;
        let cuts = [
            start,
            selected.start.clamp(start, end),
            selected.end.clamp(start, end),
            end,
        ];
        for pair in cuts.windows(2) {
            if pair[1] > pair[0] {
                let mut part = run.clone();
                part.len = pair[1] - pair[0];
                if pair[0] >= selected.start && pair[1] <= selected.end {
                    part.background_color = Some(color);
                }
                out.push(part);
            }
        }
    }
    out
}

/// The page's sizes, from the base font size, as Settings → Side panel →
/// Files → Font size will set them.
#[derive(Clone)]
struct Look {
    base: f32,
    ui: Font,
    mono: Font,
    syntax: SyntaxTheme,
}

impl Look {
    fn current(window: &Window, cx: &App) -> Self {
        let (mono, size) = match cx.try_global::<crate::app_store::GlobalAppStore>() {
            Some(store) => {
                let settings = &store.0.read(cx).settings;
                (
                    settings.terminal_font_family.font(cx),
                    settings.terminal_font_size as f32,
                )
            }
            None => (gpui::font(theme::UI_MONOSPACE_FONT), 13.),
        };
        Self {
            base: size,
            ui: window.text_style().font(),
            mono,
            syntax: SyntaxTheme::current(cx),
        }
    }

    fn line_height(&self) -> Pixels {
        px((self.base * 1.6).round())
    }

    fn code_size(&self) -> Pixels {
        px((self.base * 0.92).round())
    }
}

#[cfg(test)]
thread_local! {
    /// The bytes of code the rows drawn on this thread have shaped.
    pub(crate) static DRAWN_CODE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How far each list level indents its rows.
const LIST_INDENT: f32 = 22.;

impl Render for MarkdownView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Rc::new(Look::current(window, cx));
        let page = self.page.clone();
        let images = Rc::new(self.images.clone());
        let path: SharedString = self.path.clone().into();
        let open_link = self.open_link.clone().filter(|_| !self.deleted);
        let selection = self.selection;
        let view = cx.weak_entity();
        let count = page.rows.len();
        div()
            .id("file-markdown")
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_MARKDOWN".into())
            })
            .track_focus(&self.focus)
            .size_full()
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|view, event: &gpui::MouseDownEvent, _, _| {
                    view.press = Some(event.position);
                    view.pressed_moved = false;
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &gpui::MouseMoveEvent, _, _| {
                let Some(press) = view.press else {
                    return;
                };
                if event.pressed_button != Some(gpui::MouseButton::Left) {
                    view.press = None;
                    return;
                }
                let moved = event.position - press;
                if moved.x.abs() > px(2.) || moved.y.abs() > px(2.) {
                    view.pressed_moved = true;
                }
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| view.dragging = false),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| view.dragging = false),
            )
            .when(self.deleted, |element| element.opacity(0.5))
            .child(
                gpui::list(self.list.clone(), move |index, _window, cx| {
                    #[cfg(test)]
                    crate::render_counts::count_render("file-markdown-rows", cx);
                    let _ = cx;
                    let row = &page.rows[index];
                    let selected = match selection {
                        Some((anchor, head)) => {
                            let (start, end) = (anchor.min(head), anchor.max(head));
                            let len = if (start.row..=end.row).contains(&index) {
                                row.text().len()
                            } else {
                                0
                            };
                            selected_range(row, index, start, end, len)
                        }
                        None => 0..0,
                    };
                    let draw = Draw {
                        look: &look,
                        images: &images,
                        from: &path,
                        open_link: open_link.clone(),
                        view: view.clone(),
                        index,
                        selected,
                    };
                    let top = if index == 0 { 0. } else { row.top };
                    div()
                        .when(cfg!(test), move |element| {
                            element.debug_selector(move || format!("FILE_MD_ROW {index}"))
                        })
                        .px(px(MARGIN))
                        .when(index == 0, |element| element.pt(px(MARGIN - 4.)))
                        .when(index + 1 == count, |element| element.pb(px(MARGIN * 2.)))
                        .child(draw.row(row, top))
                        .into_any_element()
                })
                .size_full(),
            )
    }
}

struct Draw<'a> {
    look: &'a Look,
    images: &'a HashMap<String, Arc<gpui::Image>>,
    from: &'a str,
    open_link: Option<OpenLink>,
    view: gpui::WeakEntity<MarkdownView>,
    index: usize,
    /// The bytes of the row's text the selection covers.
    selected: Range<usize>,
}

impl Draw<'_> {
    /// A row inside its quotes and list indent, with its marker.
    fn row(&self, row: &Row, top: f32) -> AnyElement {
        let look = self.look;
        let mut body = div()
            .flex()
            .pt(px(top))
            .text_size(px(look.base))
            .line_height(look.line_height());
        if row.indent > 0 {
            let marker = row.marker.map(|marker| self.marker(marker));
            body = body.pl(px(LIST_INDENT * f32::from(row.indent - 1))).child(
                div()
                    .when(cfg!(test) && marker.is_some(), |element| {
                        let index = self.index;
                        element.debug_selector(move || format!("FILE_MD_MARKER {index}"))
                    })
                    .flex_none()
                    .w(px(LIST_INDENT))
                    .children(marker),
            );
        }
        let content = div().flex_1().min_w(px(0.)).child(self.content(row));
        let mut element = body.child(content).into_any_element();
        for _ in 0..row.quotes {
            element = div()
                .when(cfg!(test), |element| {
                    element.debug_selector(|| "FILE_MD_QUOTE".into())
                })
                .pl(px(12.))
                .border_l(px(3.))
                .border_color(theme::border_strong())
                .child(element)
                .into_any_element();
        }
        element
    }

    fn marker(&self, marker: Marker) -> AnyElement {
        let look = self.look;
        let base = div()
            .h(look.line_height())
            .flex()
            .items_center()
            .text_color(theme::faint());
        match marker {
            Marker::Task(checked) => base
                .when(cfg!(test), move |element| {
                    element.debug_selector(move || {
                        if checked {
                            "FILE_MD_TASK_DONE".into()
                        } else {
                            "FILE_MD_TASK".into()
                        }
                    })
                })
                .child(
                    svg()
                        .size(px(look.base))
                        .path(if checked {
                            "square-check.svg"
                        } else {
                            "checkbox.svg"
                        })
                        .text_color(if checked {
                            theme::text()
                        } else {
                            theme::faint()
                        }),
                )
                .into_any_element(),
            Marker::Number(number) => base.child(format!("{number}.")).into_any_element(),
            Marker::Bullet => base.child("•").into_any_element(),
        }
    }

    fn content(&self, row: &Row) -> AnyElement {
        let look = self.look;
        match &row.kind {
            RowKind::Heading(level, text) => {
                let scale = match level {
                    1 => 1.54,
                    2 => 1.23,
                    3 => 1.08,
                    _ => 1.,
                };
                div()
                    .pb(px(if *level == 1 { 8. } else { 6. }))
                    .when(*level == 1, |element| {
                        element
                            .mb(px(4.))
                            .border_b_1()
                            .border_color(theme::border())
                    })
                    .text_size(px((look.base * scale).round()))
                    .line_height(px((look.base * scale * 1.3).round()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .map(|element| {
                        #[cfg(test)]
                        let element =
                            crate::theme_snapshot::record_text("FILE_MD_HEADING", element);
                        element
                    })
                    .child(self.inline(text, theme::text(), FontWeight::SEMIBOLD))
                    .into_any_element()
            }
            RowKind::Paragraph(text) => div()
                .text_color(theme::muted())
                .map(|element| {
                    #[cfg(test)]
                    let element = crate::theme_snapshot::record_text("FILE_MD_PARAGRAPH", element);
                    element
                })
                .child(self.inline(text, theme::muted(), FontWeight::NORMAL))
                .into_any_element(),
            RowKind::Image(image) => self.image(image, format!("md-{}-image", self.index)),
            RowKind::Code { code, line, bytes } => self.code_line(code, *line, bytes.clone()),
            RowKind::TableRow {
                alignments,
                cells,
                head,
                last,
            } => self.table_row(alignments, cells, *head, *last),
            RowKind::Rule => div()
                .mt(px(4.))
                .mb(px(4.))
                .h(px(1.))
                .w_full()
                .bg(theme::border())
                .into_any_element(),
            RowKind::Html(line) => {
                let runs = vec![self.run(line.len(), self.look.ui.clone(), theme::muted())];
                div()
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILE_MD_HTML".into())
                    })
                    .text_color(theme::muted())
                    .child(self.selectable(line.clone(), runs, Vec::new()))
                    .into_any_element()
            }
        }
    }

    fn run(&self, len: usize, font: Font, color: Hsla) -> TextRun {
        TextRun {
            len,
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    /// Text that selects by dragging, with its links clickable.
    fn selectable(
        &self,
        text: String,
        runs: Vec<TextRun>,
        links: Vec<(Range<usize>, String)>,
    ) -> AnyElement {
        let mut text = text;
        let mut runs = with_selection(
            runs,
            self.selected.clone(),
            theme::with_alpha(theme::accent(), 0.28),
        );
        if text.is_empty() {
            // An empty line still takes a line's height.
            text.push(' ');
            runs = vec![self.run(1, self.look.ui.clone(), theme::muted())];
        }
        let styled = StyledText::new(text).with_runs(runs);
        let layout = styled.layout().clone();
        let at = move |layout: &gpui::TextLayout, position| {
            layout
                .index_for_position(position)
                .unwrap_or_else(|index| index)
        };
        let (row, down_view, down_layout) = (self.index, self.view.clone(), layout.clone());
        let (move_view, move_layout) = (self.view.clone(), layout);
        let element = div()
            .when(cfg!(test), move |element| {
                element.debug_selector(move || format!("FILE_MD_TEXT {row}"))
            })
            .on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
                let offset = at(&down_layout, event.position);
                let _ = down_view.update(cx, |view, cx| {
                    let at = Position { row, offset };
                    view.begin_selection(at, event.modifiers.shift, window, cx)
                });
            })
            .on_mouse_move(move |event, _, cx| {
                if event.pressed_button != Some(gpui::MouseButton::Left) {
                    return;
                }
                let offset = at(&move_layout, event.position);
                let _ = move_view.update(cx, |view, cx| view.drag_to(Position { row, offset }, cx));
            });
        if links.is_empty() {
            return element.child(styled).into_any_element();
        }
        let ranges = links
            .iter()
            .map(|(range, _)| range.clone())
            .collect::<Vec<_>>();
        let targets = links
            .iter()
            .map(|(_, href)| route(href, self.from))
            .collect::<Vec<_>>();
        let open_link = self.open_link.clone();
        let click_view = self.view.clone();
        element
            .child(
                InteractiveText::new(
                    SharedString::from(format!("md-{}-text", self.index)),
                    styled,
                )
                .on_click(ranges, move |index, window, cx| {
                    if dragged(&click_view, cx) {
                        return;
                    }
                    if let (Some(open), Some(target)) = (&open_link, targets.get(index)) {
                        open(target.clone(), window, cx);
                    }
                }),
            )
            .into_any_element()
    }

    /// A row that selects whole, as a table row or an image does.
    fn whole_row(&self, element: gpui::Div, len: usize) -> gpui::Div {
        let row = self.index;
        let (down_view, move_view) = (self.view.clone(), self.view.clone());
        element
            .when(!self.selected.is_empty(), |element| {
                element.bg(theme::with_alpha(theme::accent(), 0.28))
            })
            .on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
                let _ = down_view.update(cx, |view, cx| {
                    let at = Position { row, offset: 0 };
                    view.begin_selection(at, event.modifiers.shift, window, cx)
                });
            })
            .on_mouse_move(move |event, _, cx| {
                if event.pressed_button == Some(gpui::MouseButton::Left) {
                    let _ = move_view.update(cx, |view, cx| {
                        view.drag_to(Position { row, offset: len }, cx)
                    });
                }
            })
    }

    /// An image, clickable when it sits in a link; `id` is unique in the
    /// row, so two linked images in one table row each take their own click.
    fn image(&self, image: &ImageRef, id: String) -> AnyElement {
        let look = self.look;
        let body = match self.images.get(&image.path) {
            Some(decoded) => img(ImageSource::Image(decoded.clone()))
                .max_w(relative(1.))
                .when(cfg!(test), |element| {
                    element.debug_selector(|| "FILE_MD_IMAGE".into())
                })
                .into_any_element(),
            None => div()
                .when(cfg!(test), |element| {
                    element.debug_selector(|| "FILE_MD_IMAGE_ALT".into())
                })
                .text_size(px(look.base))
                .text_color(theme::faint())
                .child(if image.alt.is_empty() {
                    image.path.clone()
                } else {
                    image.alt.clone()
                })
                .into_any_element(),
        };
        let target = image.link.as_deref().map(|href| route(href, self.from));
        let open_link = self.open_link.clone();
        div()
            .id(SharedString::from(id))
            .when_some(
                image.link.clone().filter(|_| cfg!(test)),
                |element, link| {
                    element.debug_selector(move || format!("FILE_MD_IMAGE_LINK {link}"))
                },
            )
            .child(body)
            .when_some(target.zip(open_link), |element, (target, open)| {
                let view = self.view.clone();
                element.cursor_pointer().on_click(move |_, window, cx| {
                    if !dragged(&view, cx) {
                        open(target.clone(), window, cx)
                    }
                })
            })
            .into_any_element()
    }

    fn code_line(&self, code: &CodeBlock, line: usize, bytes: Range<usize>) -> AnyElement {
        let look = self.look;
        let size = look.code_size();
        let line_len = code.document.line(line).len();
        let (first, last) = (
            line == 0 && bytes.start == 0,
            line + 1 == code.document.line_count() && bytes.end == line_len,
        );
        let (text, spans) = code.fragment(line, bytes);
        #[cfg(test)]
        DRAWN_CODE_BYTES.set(DRAWN_CODE_BYTES.get() + text.len());
        let runs = spans
            .into_iter()
            .map(|(len, capture)| {
                let color = capture
                    .and_then(|capture| look.syntax.color(capture))
                    .unwrap_or(theme::text());
                self.run(len, look.mono.clone(), color)
            })
            .collect();
        div()
            .when(cfg!(test) && first, |element| {
                element.debug_selector(|| "FILE_MD_CODE".into())
            })
            .border_l_1()
            .border_r_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .px(px(12.))
            .when(first, |element| {
                element.border_t_1().rounded_t(px(6.)).pt(px(10.))
            })
            .when(last, |element| {
                element.border_b_1().rounded_b(px(6.)).pb(px(10.))
            })
            .text_size(size)
            .line_height(px((f32::from(size) * 1.6).round()))
            .child(self.selectable(text, runs, Vec::new()))
            .into_any_element()
    }

    fn table_row(
        &self,
        alignments: &[Alignment],
        cells: &[Cell],
        head: bool,
        last: bool,
    ) -> AnyElement {
        let columns = alignments.len().max(cells.len()).max(1);
        let (color, weight) = if head {
            (theme::text(), FontWeight::SEMIBOLD)
        } else {
            (theme::muted(), FontWeight::NORMAL)
        };
        let len = cells
            .iter()
            .map(|cell| cell_text(cell).len() + 1)
            .sum::<usize>()
            .saturating_sub(1);
        let row = div()
            .when(cfg!(test) && head, |element| {
                element.debug_selector(|| "FILE_MD_TABLE".into())
            })
            .flex()
            .border_l_1()
            .border_r_1()
            .border_t_1()
            .border_color(theme::border())
            .when(head, |element| element.rounded_t(px(6.)).bg(theme::panel()))
            .when(last, |element| element.border_b_1().rounded_b(px(6.)))
            .children((0..columns).map(|column| {
                let cell = cells.get(column).cloned().unwrap_or_default();
                let align = alignments.get(column).copied().unwrap_or(Alignment::None);
                let row = self.index;
                div()
                    .when(cfg!(test), move |element| {
                        element.debug_selector(move || format!("FILE_MD_CELL {row}-{column}"))
                    })
                    .flex_1()
                    .min_w(px(0.))
                    .px(px(10.))
                    .py(px(6.))
                    .when(column > 0, |element| {
                        element.border_l_1().border_color(theme::border())
                    })
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(4.))
                    .when(align == Alignment::Center, |element| {
                        element.justify_center()
                    })
                    .when(align == Alignment::Right, |element| element.justify_end())
                    .children(
                        cell.iter()
                            .enumerate()
                            .map(|(piece_index, piece)| match piece {
                                Piece::Text(text) => {
                                    let id = format!("md-{}-{column}-{piece_index}", self.index);
                                    self.inline_plain(text, color, weight, id)
                                }
                                Piece::Image(image) => self.image(
                                    image,
                                    format!("md-{}-{column}-{piece_index}-image", self.index),
                                ),
                            }),
                    )
            }));
        self.whole_row(row, len).into_any_element()
    }

    fn inline_runs(&self, text: &Inline, color: Hsla, weight: FontWeight) -> Vec<TextRun> {
        let look = self.look;
        text.styles
            .iter()
            .map(|(range, style)| {
                let mut font = if style.code {
                    look.mono.clone()
                } else {
                    look.ui.clone()
                };
                if style.strong {
                    font.weight = FontWeight::SEMIBOLD;
                } else if !style.code {
                    font.weight = weight;
                }
                if style.emphasis {
                    font.style = FontStyle::Italic;
                }
                let color = if style.link {
                    theme::info()
                } else if style.strong || style.code {
                    theme::text()
                } else {
                    color
                };
                TextRun {
                    len: range.len(),
                    font,
                    color,
                    background_color: style.code.then(theme::raised),
                    underline: None::<UnderlineStyle>,
                    strikethrough: style.strike.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(color),
                    }),
                }
            })
            .collect()
    }

    /// A heading's or paragraph's text: selectable, its links clickable.
    fn inline(&self, text: &Inline, color: Hsla, weight: FontWeight) -> AnyElement {
        let runs = self.inline_runs(text, color, weight);
        self.selectable(text.text.clone(), runs, text.links.clone())
    }

    /// A table cell's text, which selects with its row.
    fn inline_plain(
        &self,
        text: &Inline,
        color: Hsla,
        weight: FontWeight,
        id: String,
    ) -> AnyElement {
        let runs = self.inline_runs(text, color, weight);
        let styled = StyledText::new(text.text.clone()).with_runs(runs);
        if text.links.is_empty() {
            return div().child(styled).into_any_element();
        }
        let ranges = text
            .links
            .iter()
            .map(|(range, _)| range.clone())
            .collect::<Vec<_>>();
        let targets = text
            .links
            .iter()
            .map(|(_, href)| route(href, self.from))
            .collect::<Vec<_>>();
        let open_link = self.open_link.clone();
        let view = self.view.clone();
        div()
            .child(
                InteractiveText::new(SharedString::from(id), styled).on_click(
                    ranges,
                    move |index, window, cx| {
                        if dragged(&view, cx) {
                            return;
                        }
                        if let (Some(open), Some(target)) = (&open_link, targets.get(index)) {
                            open(target.clone(), window, cx);
                        }
                    },
                ),
            )
            .into_any_element()
    }
}

/// The image format a local image's extension names; any other file is
/// not read as an image.
pub(crate) fn image_format(path: &str) -> Option<gpui::ImageFormat> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => gpui::ImageFormat::Png,
        "jpg" | "jpeg" => gpui::ImageFormat::Jpeg,
        "gif" => gpui::ImageFormat::Gif,
        "webp" => gpui::ImageFormat::Webp,
        "svg" => gpui::ImageFormat::Svg,
        "bmp" => gpui::ImageFormat::Bmp,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_open_urls_and_files_inside_the_root_only() {
        let from = "docs/guide/README.md";
        assert_eq!(
            route("https://runner.dev/a?b#c", from),
            LinkTarget::Url("https://runner.dev/a?b#c".into())
        );
        assert_eq!(
            route("mailto:a@b.c", from),
            LinkTarget::Url("mailto:a@b.c".into())
        );
        assert_eq!(route("#install", from), LinkTarget::None);
        assert_eq!(route("javascript:alert(1)", from), LinkTarget::None);
        assert_eq!(route("file:///etc/passwd", from), LinkTarget::None);
        assert_eq!(route("C:/Windows/win.ini", from), LinkTarget::None);
        assert_eq!(route("..\\..\\secret", from), LinkTarget::None);
        assert_eq!(
            route("setup.md", from),
            LinkTarget::File("docs/guide/setup.md".into())
        );
        assert_eq!(
            route("./setup.md#step-2", from),
            LinkTarget::File("docs/guide/setup.md".into())
        );
        assert_eq!(
            route("../../crates/a%20b.rs?plain=1", from),
            LinkTarget::File("crates/a b.rs".into())
        );
        assert_eq!(
            route("/Cargo.toml", from),
            LinkTarget::File("Cargo.toml".into())
        );
        assert_eq!(
            route("../../../outside.md", from),
            LinkTarget::None,
            "leaves the root"
        );
        assert_eq!(route("/../outside.md", from), LinkTarget::None);
        assert_eq!(route("../..", from), LinkTarget::None, "the root itself");
        assert_eq!(
            route("img.png", "README.md"),
            LinkTarget::File("img.png".into())
        );
    }

    fn page(text: &str) -> Page {
        Page::parse(text, "docs/README.md")
    }

    #[test]
    fn headings_paragraphs_and_inline_styles_parse() {
        let page = page(
            "# Runner\n\nA *local* **cockpit** with `code`, ~~old~~ and [a guide](guide.md).\n",
        );
        let [Block::Heading(1, title), Block::Paragraph(text)] = &page.blocks[..] else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(title.text, "Runner");
        assert_eq!(text.text, "A local cockpit with code, old and a guide.");
        let style_of = |word: &str| {
            let at = text.text.find(word).unwrap();
            text.styles
                .iter()
                .find(|(range, _)| range.contains(&at))
                .unwrap()
                .1
        };
        assert!(style_of("local").emphasis);
        assert!(style_of("cockpit").strong);
        assert!(style_of("code").code);
        assert!(style_of("old").strike);
        assert!(style_of("guide").link);
        assert_eq!(text.links.len(), 1);
        assert_eq!(&text.text[text.links[0].0.clone()], "a guide");
        assert_eq!(text.links[0].1, "guide.md");
    }

    #[test]
    fn lists_task_lists_and_nested_blocks_parse() {
        let page = page(
            "- one\n- [x] done\n- [ ] todo\n  - nested\n\n3. three\n4. four\n\n> quoted\n\n---\n",
        );
        let Block::List { start: None, items } = &page.blocks[0] else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].task, None);
        assert_eq!(items[1].task, Some(true));
        assert_eq!(items[2].task, Some(false));
        let Block::Paragraph(text) = &items[1].blocks[0] else {
            panic!("{:?}", items[1].blocks);
        };
        assert_eq!(text.text, "done");
        assert!(
            matches!(items[2].blocks[1], Block::List { .. }),
            "{:?}",
            items[2].blocks
        );
        let Block::List {
            start: Some(3),
            items,
        } = &page.blocks[1]
        else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(items.len(), 2);
        assert!(
            matches!(&page.blocks[2], Block::Quote(inner) if matches!(inner[0], Block::Paragraph(_)))
        );
        assert!(matches!(page.blocks[3], Block::Rule));
    }

    #[test]
    fn code_blocks_are_highlighted_as_their_language() {
        let page = page("```rust\nfn main() {}\n```\n\n```\nplain\n```\n\n    indented\n");
        let Block::Code(rust) = &page.blocks[0] else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(rust.document.line(0), "fn main() {}");
        assert_eq!(rust.document.line_count(), 1);
        let spans = rust.highlights.as_ref().expect("highlighted").line(0);
        assert_eq!(
            super::super::syntax::CAPTURES[spans[0].capture as usize],
            "keyword"
        );
        let Block::Code(plain) = &page.blocks[1] else {
            panic!("{:?}", page.blocks);
        };
        assert!(plain.highlights.is_none());
        assert!(
            matches!(&page.blocks[2], Block::Code(code) if code.document.line(0) == "indented")
        );
    }

    #[test]
    fn tables_keep_their_alignment_and_cells() {
        let page = page("| Role | Slots |\n| :--- | ---: |\n| **coder** | 1 |\n| reviewer | 2 |\n");
        let Block::Table {
            alignments,
            header,
            rows,
        } = &page.blocks[0]
        else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(alignments, &[Alignment::Left, Alignment::Right]);
        assert_eq!(
            header
                .iter()
                .map(|cell| cell_text(cell))
                .collect::<Vec<_>>(),
            ["Role", "Slots"]
        );
        assert_eq!(rows.len(), 2);
        let [Piece::Text(coder)] = &rows[0][0][..] else {
            panic!("{:?}", rows[0][0]);
        };
        assert_eq!(coder.text, "coder");
        assert!(coder.styles[0].1.strong);
    }

    #[test]
    fn html_shows_as_text_and_footnotes_and_math_stay_as_written() {
        let page = page("<div align=\"center\">\n  <b>hi</b>\n</div>\n\nInline <kbd>⌘</kbd> key.\n\nA note[^1] and $x$.\n\n[^1]: The note.\n");
        let Block::Html(html) = &page.blocks[0] else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(html, "<div align=\"center\">\n  <b>hi</b>\n</div>");
        let Block::Paragraph(text) = &page.blocks[1] else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(text.text, "Inline <kbd>⌘</kbd> key.");
        let Block::Paragraph(text) = &page.blocks[2] else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(text.text, "A note[^1] and $x$.");
        assert!(
            matches!(&page.blocks[3], Block::Paragraph(text) if text.text == "[^1]: The note.")
        );
    }

    #[test]
    fn local_images_stand_alone_and_remote_ones_show_their_alt_text() {
        let page = page(
            "Intro\n![Shot](../assets/shot.png)\nafter\n\n![badge](https://img.shields.io/x.svg) [![ci](https://x/ci.svg)](https://x)\n\n![up](../../escape.png)\n",
        );
        let kinds: Vec<_> = page
            .blocks
            .iter()
            .map(|block| match block {
                Block::Paragraph(text) => format!("p:{}", text.text),
                Block::Image(image) => format!("img:{}:{}", image.path, image.alt),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "p:Intro ",
                "img:assets/shot.png:Shot",
                "p: after",
                "p:badge ci",
                "p:up",
            ]
        );
        assert_eq!(page.images(), ["assets/shot.png"]);
    }

    #[test]
    fn images_are_recognised_by_their_extension() {
        assert_eq!(image_format("a/b.PNG"), Some(gpui::ImageFormat::Png));
        assert_eq!(image_format("logo.svg"), Some(gpui::ImageFormat::Svg));
        assert_eq!(image_format("notes.txt"), None);
        assert_eq!(image_format("noext"), None);
    }

    #[test]
    fn loose_task_items_keep_their_boxes() {
        let page = page("- [x] done\n\n  more about it\n\n- [ ] next\n");
        let Block::List { items, .. } = &page.blocks[0] else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(
            items.iter().map(|item| item.task).collect::<Vec<_>>(),
            [Some(true), Some(false)]
        );
        let texts: Vec<_> = items
            .iter()
            .flat_map(|item| &item.blocks)
            .map(|block| match block {
                Block::Paragraph(text) => text.text.clone(),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(texts, ["done", "more about it", "next"], "no literal [x]");
        let markers: Vec<_> = page.rows.iter().map(|row| row.marker).collect();
        assert_eq!(
            markers,
            [Some(Marker::Task(true)), None, Some(Marker::Task(false))]
        );
    }

    #[test]
    fn linked_and_table_images_stay_images() {
        let page = page(
            "[![Shot](shot.png)](guide.md) after\n\n| Logo | Name |\n| --- | --- |\n| ![Logo](logo.svg) | Runner |\n",
        );
        let Block::Image(image) = &page.blocks[0] else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(
            image,
            &ImageRef {
                path: "docs/shot.png".into(),
                alt: "Shot".into(),
                link: Some("guide.md".into()),
            }
        );
        assert!(matches!(&page.blocks[1], Block::Paragraph(text) if text.text == " after"));
        let Block::Table { rows, .. } = &page.blocks[2] else {
            panic!("{:?}", page.blocks);
        };
        assert!(matches!(&rows[0][0][..], [Piece::Image(image)] if image.path == "docs/logo.svg"));
        assert_eq!(page.images(), ["docs/shot.png", "docs/logo.svg"]);
    }

    #[test]
    fn long_blocks_lay_out_as_a_row_each() {
        let mut text = String::from("```\n");
        for line in 0..1000 {
            text.push_str(&format!("line {line}\n"));
        }
        text.push_str("```\n\n");
        for item in 0..50 {
            text.push_str(&format!("- item {item}\n"));
        }
        text.push_str("\n| a |\n| - |\n| 1 |\n| 2 |\n\n> quoted\n>\n> - inside\n");
        let page = page(&text);
        let code = page
            .rows
            .iter()
            .filter(|row| matches!(row.kind, RowKind::Code { .. }))
            .count();
        assert_eq!(code, 1000);
        assert_eq!(page.rows[999].text(), "line 999");
        let bullets = page
            .rows
            .iter()
            .filter(|row| row.marker == Some(Marker::Bullet))
            .count();
        assert_eq!(bullets, 51, "50 items and the quoted one");
        let table: Vec<_> = page
            .rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::TableRow { head, last, .. } => Some((*head, *last)),
                _ => None,
            })
            .collect();
        assert_eq!(table, [(true, false), (false, false), (false, true)]);
        let quoted: Vec<_> = page.rows.iter().filter(|row| row.quotes == 1).collect();
        assert_eq!(quoted.len(), 2);
        assert_eq!(quoted[1].indent, 1);
    }

    #[test]
    fn a_selection_copies_rows_and_marks_only_its_bytes() {
        let runs = |lens: &[usize]| {
            lens.iter()
                .map(|len| TextRun {
                    len: *len,
                    font: gpui::font("x"),
                    color: gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                })
                .collect::<Vec<_>>()
        };
        let marked = with_selection(runs(&[4, 6]), 2..7, gpui::white());
        assert_eq!(
            marked
                .iter()
                .map(|run| (run.len, run.background_color.is_some()))
                .collect::<Vec<_>>(),
            [(2, false), (2, true), (3, true), (3, false)]
        );
        assert_eq!(with_selection(runs(&[4]), 0..0, gpui::white()).len(), 1);
    }

    #[test]
    fn a_long_code_line_splits_into_rows_that_copy_back_whole() {
        let line = "é\t".repeat(600) + "end";
        let pieces = fragments(&line);
        assert!(pieces.len() > 1);
        assert!(pieces.iter().all(|piece| piece.len() <= FRAGMENT_BYTES + 4));
        assert_eq!(pieces.first().unwrap().start, 0);
        assert_eq!(pieces.last().unwrap().end, line.len());
        assert!(pieces.windows(2).all(|pair| pair[0].end == pair[1].start));
        let empty = fragments("");
        assert!(
            empty.len() == 1 && empty[0].is_empty(),
            "an empty line is one row"
        );
        // A combining mark stays with its base.
        let marks = format!("{}e\u{301}x", "a".repeat(FRAGMENT_BYTES - 1));
        assert_eq!(fragments(&marks)[0], 0..FRAGMENT_BYTES + 2);

        let page = page(&format!("```\n{line}\nnext\n```\n"));
        let rows = page.rows.len();
        assert_eq!(rows, pieces.len() + 1);
        // Tabs keep their stops across a split: every piece expands as the
        // whole line would.
        let drawn: String = (0..pieces.len()).map(|row| page.rows[row].text()).collect();
        assert_eq!(drawn, "é   ".repeat(600) + "end");
        let view_text = {
            let mut copied = String::new();
            for (index, row) in page.rows.iter().enumerate() {
                if index > 0 && !row.continues() {
                    copied.push('\n');
                }
                copied.push_str(&row.text());
            }
            copied
        };
        assert_eq!(view_text, format!("{drawn}\nnext"));
    }

    #[test]
    fn link_text_on_both_sides_of_a_linked_image_stays_linked() {
        let page = page(
            "[before ![image](image.png) after](guide.md)\n\n| x |\n| - |\n| [a ![i](i.png) b](t.md) |\n",
        );
        let [Block::Paragraph(before), Block::Image(image), Block::Paragraph(after), Block::Table { rows, .. }] =
            &page.blocks[..]
        else {
            panic!("{:?}", page.blocks);
        };
        assert_eq!(before.text, "before ");
        assert_eq!(before.links, [(0..7, "guide.md".to_owned())]);
        assert_eq!(image.link.as_deref(), Some("guide.md"));
        assert_eq!(after.text, " after");
        assert_eq!(after.links, [(0..6, "guide.md".to_owned())]);
        let [Piece::Text(a), Piece::Image(i), Piece::Text(b)] = &rows[0][0][..] else {
            panic!("{:?}", rows[0][0]);
        };
        assert_eq!(a.links, [(0..2, "t.md".to_owned())]);
        assert_eq!(i.link.as_deref(), Some("t.md"));
        assert_eq!(b.links, [(0..2, "t.md".to_owned())]);
    }
}
