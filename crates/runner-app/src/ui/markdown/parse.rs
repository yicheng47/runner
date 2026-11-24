use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, Options, Tag, TagEnd};

use super::links::{route, LinkTarget};
use super::{Block, Cell, CodeBlock, ImageRef, Inline, InlineStyle, ListItem, Piece, MAX_NESTING};
use crate::surfaces::files::document::Document;
use crate::surfaces::files::highlight;

pub(super) fn parse_document_blocks(text: &str, path: &str) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let mut parser = Parser {
        events: bounded(pulldown_cmark::Parser::new_ext(text, options)),
        at: 0,
        path,
    };
    parser.blocks(None)
}

/// `events` with the quotes, lists and items below [`MAX_NESTING`] taken away:
/// what they hold still shows, at that depth, with each item on its own line.
fn bounded<'a>(events: impl Iterator<Item = Event<'a>>) -> Vec<Event<'a>> {
    let mut depth = 0;
    let mut bounded = Vec::new();
    for event in events {
        match &event {
            Event::Start(Tag::BlockQuote(_) | Tag::Item) => {
                depth += 1;
                if depth > MAX_NESTING {
                    continue;
                }
            }
            // A list only goes where its items can: it draws nothing else.
            Event::Start(Tag::List(_)) => {
                depth += 1;
                if depth >= MAX_NESTING {
                    continue;
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                depth -= 1;
                if depth >= MAX_NESTING {
                    continue;
                }
            }
            Event::End(TagEnd::Item) => {
                depth -= 1;
                if depth >= MAX_NESTING {
                    bounded.push(Event::HardBreak);
                    continue;
                }
            }
            Event::End(TagEnd::List(_)) => {
                depth -= 1;
                if depth + 1 >= MAX_NESTING {
                    continue;
                }
            }
            _ => {}
        }
        bounded.push(event);
    }
    bounded
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
