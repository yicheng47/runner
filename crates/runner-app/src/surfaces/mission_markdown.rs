use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, App, CursorStyle, EntityId, FontWeight, Global, HighlightStyle,
    Hsla, InteractiveText, MouseButton, SharedString, StrikethroughStyle, StyledText, TextLayout,
    UnderlineStyle, Window,
};

use crate::theme;

#[derive(Clone, Debug, Eq, PartialEq)]
enum MarkdownBlock {
    Paragraph(String),
    Heading(u8, String),
    ListItem {
        marker: String,
        text: String,
    },
    Quote(String),
    Code(String),
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Rule,
}

pub(crate) type FeedPosition = (usize, usize);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FeedSelection {
    pub(crate) event_id: String,
    pub(crate) anchor: FeedPosition,
    pub(crate) head: FeedPosition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FeedSelectionPhase {
    Begin,
    Extend,
    End,
}

type FeedSelectionCallback = dyn Fn(FeedSelectionPhase, &str, FeedPosition, &mut Window, &mut App);

#[derive(Clone)]
pub(crate) struct FeedSelectionHandler(Rc<FeedSelectionCallback>);

impl FeedSelectionHandler {
    pub(crate) fn new(
        callback: impl Fn(FeedSelectionPhase, &str, FeedPosition, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self(Rc::new(callback))
    }

    fn handle(
        &self,
        phase: FeedSelectionPhase,
        event_id: &str,
        position: FeedPosition,
        window: &mut Window,
        cx: &mut App,
    ) {
        (self.0)(phase, event_id, position, window, cx);
    }
}

#[derive(Default)]
struct MarkdownLinkHover {
    active_line: Option<String>,
    links: HashMap<String, usize>,
}

impl Global for MarkdownLinkHover {}

impl MarkdownLinkHover {
    fn link_for(&self, line: &str) -> Option<usize> {
        (self.active_line.as_deref() == Some(line))
            .then(|| self.links.get(line).copied())
            .flatten()
    }

    fn set_link(&mut self, line: &str, link: Option<usize>) -> bool {
        match link {
            Some(link) => self.links.insert(line.to_owned(), link) != Some(link),
            None => self.links.remove(line).is_some(),
        }
    }

    fn set_line_active(&mut self, line: &str, active: bool) -> bool {
        let next = if active {
            Some(line.to_owned())
        } else if self.active_line.as_deref() == Some(line) {
            None
        } else {
            return false;
        };
        if self.active_line == next {
            return false;
        }
        self.active_line = next;
        true
    }
}

fn parse_blocks(text: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut paragraph = Vec::new();
    let mut code = None::<Vec<&str>>;
    let lines = text.lines().collect::<Vec<_>>();
    let flush_paragraph = |paragraph: &mut Vec<&str>, blocks: &mut Vec<MarkdownBlock>| {
        if !paragraph.is_empty() {
            blocks.push(MarkdownBlock::Paragraph(paragraph.join("\n")));
            paragraph.clear();
        }
    };
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if let Some(code_lines) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                blocks.push(MarkdownBlock::Code(code_lines.join("\n")));
                code = None;
            } else {
                code_lines.push(line);
            }
            index += 1;
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            flush_paragraph(&mut paragraph, &mut blocks);
            code = Some(Vec::new());
        } else if index + 1 < lines.len()
            && trimmed.contains('|')
            && is_table_separator(lines[index + 1])
        {
            flush_paragraph(&mut paragraph, &mut blocks);
            let header = parse_table_row(trimmed);
            index += 2;
            let mut rows = Vec::new();
            while index < lines.len() && lines[index].trim().contains('|') {
                rows.push(parse_table_row(lines[index].trim()));
                index += 1;
            }
            blocks.push(MarkdownBlock::Table { header, rows });
            continue;
        } else if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks);
        } else if matches!(trimmed, "---" | "***" | "___") {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(MarkdownBlock::Rule);
        } else if let Some((level, body)) = heading(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(MarkdownBlock::Heading(level, body.to_owned()));
        } else if let Some(body) = trimmed.strip_prefix("> ") {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(MarkdownBlock::Quote(body.to_owned()));
        } else if let Some(body) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(MarkdownBlock::ListItem {
                marker: "•".into(),
                text: body.to_owned(),
            });
        } else if let Some((number, body)) = ordered_list_body(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(MarkdownBlock::ListItem {
                marker: format!("{number}."),
                text: body.to_owned(),
            });
        } else {
            paragraph.push(line);
        }
        index += 1;
    }
    if let Some(code_lines) = code {
        blocks.push(MarkdownBlock::Code(code_lines.join("\n")));
    }
    flush_paragraph(&mut paragraph, &mut blocks);
    blocks
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let count = line.chars().take_while(|char| *char == '#').count();
    (1..=3)
        .contains(&count)
        .then(|| {
            line.get(count..)?
                .strip_prefix(' ')
                .map(|body| (count as u8, body))
        })
        .flatten()
}

fn ordered_list_body(line: &str) -> Option<(&str, &str)> {
    let dot = line.find(". ")?;
    (dot > 0 && line[..dot].chars().all(|char| char.is_ascii_digit()))
        .then(|| (&line[..dot], &line[dot + 2..]))
}

fn is_table_separator(line: &str) -> bool {
    let cells = parse_table_row(line.trim());
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let cell = cell.trim_matches(':');
            cell.len() >= 3 && cell.chars().all(|character| character == '-')
        })
}

fn parse_table_row(line: &str) -> Vec<String> {
    line.trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

fn normalized_offset(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn ordered_selection(selection: &FeedSelection) -> (FeedPosition, FeedPosition) {
    if selection.anchor <= selection.head {
        (selection.anchor, selection.head)
    } else {
        (selection.head, selection.anchor)
    }
}

fn selection_range_for_block(
    selection: &FeedSelection,
    event_id: &str,
    block: usize,
    text: &str,
) -> Option<Range<usize>> {
    if selection.event_id != event_id || selection.anchor == selection.head {
        return None;
    }
    let (start, end) = ordered_selection(selection);
    if block < start.0 || block > end.0 {
        return None;
    }
    let range_start = if block == start.0 { start.1 } else { 0 };
    let range_end = if block == end.0 { end.1 } else { text.len() };
    let range = normalized_offset(text, range_start)..normalized_offset(text, range_end);
    (!range.is_empty()).then_some(range)
}

fn plain_inline_text(source: &str, prefix: Option<&str>) -> String {
    let mut text = prefix.unwrap_or_default().to_owned();
    let mut highlights = Vec::new();
    let mut link_ranges = Vec::new();
    let mut urls = Vec::new();
    append_inline_segment(
        source,
        None,
        &mut text,
        &mut highlights,
        &mut link_ranges,
        &mut urls,
        None,
    );
    text
}

pub(crate) fn rendered_plain_text_blocks(source: &str) -> Vec<String> {
    parse_blocks(source)
        .into_iter()
        .flat_map(|block| match block {
            MarkdownBlock::Paragraph(text) => text
                .lines()
                .map(|line| plain_inline_text(line, None))
                .collect(),
            MarkdownBlock::Heading(_, text) | MarkdownBlock::Quote(text) => {
                vec![plain_inline_text(&text, None)]
            }
            MarkdownBlock::ListItem { marker, text } => {
                vec![plain_inline_text(&text, Some(&format!("{marker} ")))]
            }
            MarkdownBlock::Code(text) => vec![text],
            MarkdownBlock::Table { header, rows } => header
                .into_iter()
                .chain(rows.into_iter().flatten())
                .map(|cell| plain_inline_text(&cell, None))
                .collect(),
            MarkdownBlock::Rule => Vec::new(),
        })
        .collect()
}

pub(crate) fn selected_plain_text(source: &str, selection: &FeedSelection) -> Option<String> {
    let blocks = rendered_plain_text_blocks(source);
    let (start, end) = ordered_selection(selection);
    if start == end || start.0 >= blocks.len() {
        return None;
    }
    let end_block = end.0.min(blocks.len().saturating_sub(1));
    let selected = blocks[start.0..=end_block]
        .iter()
        .enumerate()
        .map(|(relative, text)| {
            let block = start.0 + relative;
            let start_offset = if block == start.0 { start.1 } else { 0 };
            let end_offset = if block == end.0 { end.1 } else { text.len() };
            &text[normalized_offset(text, start_offset)..normalized_offset(text, end_offset)]
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!selected.is_empty()).then_some(selected)
}

pub(crate) fn render_markdown(
    id: &str,
    text: &str,
    view: EntityId,
    selection: Option<&FeedSelection>,
    selection_color: Hsla,
    selection_handler: Option<&FeedSelectionHandler>,
    cx: &App,
) -> AnyElement {
    let blocks = parse_blocks(text);
    let bottom_padding = blocks
        .last()
        .map(block_margins)
        .map_or(0., |margins| margins.1);
    let id = id.to_owned();
    let mut previous = None::<(MarkdownBlockKind, f32)>;
    let mut selection_block = 0;
    let rendered = blocks
        .into_iter()
        .enumerate()
        .map(|(index, block)| {
            let margins = block_margins(&block);
            let kind = block_kind(&block);
            let top_margin = match previous {
                Some((MarkdownBlockKind::ListItem, _)) if kind == MarkdownBlockKind::ListItem => 4.,
                Some((MarkdownBlockKind::Paragraph, bottom))
                    if kind == MarkdownBlockKind::Paragraph =>
                {
                    bottom + margins.0
                }
                Some((_, bottom)) => bottom.max(margins.0),
                None => margins.0,
            };
            previous = Some((kind, margins.1));
            let block_start = selection_block;
            selection_block += selection_block_count(&block);
            render_block(
                &id,
                index,
                block,
                block_start,
                top_margin,
                view,
                selection,
                selection_color,
                selection_handler,
                cx,
            )
        })
        .collect::<Vec<_>>();
    div()
        .min_w(gpui::px(0.))
        .flex()
        .flex_col()
        .pb(px(bottom_padding))
        .children(rendered)
        .into_any_element()
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MarkdownBlockKind {
    ListItem,
    Paragraph,
    Other,
}

fn block_kind(block: &MarkdownBlock) -> MarkdownBlockKind {
    match block {
        MarkdownBlock::ListItem { .. } => MarkdownBlockKind::ListItem,
        MarkdownBlock::Paragraph(_) => MarkdownBlockKind::Paragraph,
        _ => MarkdownBlockKind::Other,
    }
}

fn block_margins(block: &MarkdownBlock) -> (f32, f32) {
    match block {
        MarkdownBlock::Paragraph(_) | MarkdownBlock::ListItem { .. } => (6., 6.),
        MarkdownBlock::Heading(1, _) => (12., 6.),
        MarkdownBlock::Heading(2, _) => (12., 4.),
        MarkdownBlock::Heading(_, _) => (8., 4.),
        MarkdownBlock::Quote(_) | MarkdownBlock::Code(_) | MarkdownBlock::Table { .. } => (8., 8.),
        MarkdownBlock::Rule => (12., 12.),
    }
}

fn selection_block_count(block: &MarkdownBlock) -> usize {
    match block {
        MarkdownBlock::Paragraph(text) => text.lines().count(),
        MarkdownBlock::Heading(_, _)
        | MarkdownBlock::ListItem { .. }
        | MarkdownBlock::Quote(_)
        | MarkdownBlock::Code(_) => 1,
        MarkdownBlock::Table { header, rows } => {
            header.len() + rows.iter().map(Vec::len).sum::<usize>()
        }
        MarkdownBlock::Rule => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_block(
    id: &str,
    index: usize,
    block: MarkdownBlock,
    selection_block: usize,
    top_margin: f32,
    view: EntityId,
    selection: Option<&FeedSelection>,
    selection_color: Hsla,
    selection_handler: Option<&FeedSelectionHandler>,
    cx: &App,
) -> AnyElement {
    let content = match block {
        MarkdownBlock::Paragraph(text) => div()
            .flex()
            .flex_col()
            .line_height(rems(20. / 16.))
            .children(text.lines().enumerate().map(|(line, text)| {
                render_inline_line(
                    format!("{id}-{index}-paragraph-{line}"),
                    id,
                    selection_block + line,
                    text,
                    None,
                    view,
                    selection,
                    selection_color,
                    selection_handler,
                    cx,
                )
            }))
            .into_any_element(),
        MarkdownBlock::Heading(level, text) => div()
            .text_size(rems(if level == 1 { 14. / 16. } else { 13. / 16. }))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(if level == 3 {
                theme::muted()
            } else {
                theme::text()
            })
            .child(render_inline_line(
                format!("{id}-{index}-heading"),
                id,
                selection_block,
                &text,
                None,
                view,
                selection,
                selection_color,
                selection_handler,
                cx,
            ))
            .into_any_element(),
        MarkdownBlock::ListItem { marker, text } => div()
            .pl(px(24.))
            .line_height(rems(20. / 16.))
            .child(render_inline_line(
                format!("{id}-{index}-list"),
                id,
                selection_block,
                &text,
                Some(&format!("{marker} ")),
                view,
                selection,
                selection_color,
                selection_handler,
                cx,
            ))
            .into_any_element(),
        MarkdownBlock::Quote(text) => div()
            .border_l_2()
            .border_color(theme::border_strong())
            .pl_3()
            .text_color(theme::muted())
            .child(render_inline_line(
                format!("{id}-{index}-quote"),
                id,
                selection_block,
                &text,
                None,
                view,
                selection,
                selection_color,
                selection_handler,
                cx,
            ))
            .into_any_element(),
        MarkdownBlock::Code(text) => div()
            .id((SharedString::from(id.to_owned()), index))
            .overflow_x_scroll()
            .rounded_md()
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .px_3()
            .py_2()
            .font_family("JetBrains Mono")
            .text_size(rems(12. / 16.))
            .line_height(rems(19. / 16.))
            .whitespace_nowrap()
            .child(render_plain_line(
                format!("{id}-{index}-code"),
                id,
                selection_block,
                text,
                view,
                selection,
                selection_color,
                selection_handler,
            ))
            .into_any_element(),
        MarkdownBlock::Table { header, rows } => div()
            .id((SharedString::from(id.to_owned()), index))
            .overflow_x_scroll()
            .child(
                div()
                    .min_w(px(420.))
                    .flex()
                    .flex_col()
                    .child(render_table_row(
                        id,
                        index,
                        0,
                        selection_block,
                        header.clone(),
                        true,
                        view,
                        selection,
                        selection_color,
                        selection_handler,
                        cx,
                    ))
                    .children(rows.into_iter().enumerate().scan(
                        selection_block + header.len(),
                        |row_block, (row_index, row)| {
                            let start = *row_block;
                            *row_block += row.len();
                            Some(render_table_row(
                                id,
                                index,
                                row_index + 1,
                                start,
                                row,
                                false,
                                view,
                                selection,
                                selection_color,
                                selection_handler,
                                cx,
                            ))
                        },
                    )),
            )
            .into_any_element(),
        MarkdownBlock::Rule => div()
            .h(rems(1. / 16.))
            .w_full()
            .bg(theme::border())
            .into_any_element(),
    };

    div()
        .mt(px(top_margin))
        .when(cfg!(test), |block| {
            block.debug_selector(move || format!("MISSION_MARKDOWN_BLOCK_{index}"))
        })
        .child(content)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_table_row(
    id: &str,
    block_index: usize,
    row_index: usize,
    selection_block: usize,
    cells: Vec<String>,
    heading: bool,
    view: EntityId,
    selection: Option<&FeedSelection>,
    selection_color: Hsla,
    selection_handler: Option<&FeedSelectionHandler>,
    cx: &App,
) -> AnyElement {
    div()
        .flex()
        .children(cells.into_iter().enumerate().map(|(cell_index, cell)| {
            div()
                .min_w(px(100.))
                .flex_1()
                .border_1()
                .border_color(theme::border())
                .when(heading, |cell| {
                    cell.bg(theme::raised()).font_weight(FontWeight::SEMIBOLD)
                })
                .px_2()
                .py_1()
                .text_size(rems(12. / 16.))
                .text_color(if heading {
                    theme::text()
                } else {
                    theme::muted()
                })
                .child(render_inline_line(
                    format!("{id}-{block_index}-table-{row_index}-{cell_index}"),
                    id,
                    selection_block + cell_index,
                    &cell,
                    None,
                    view,
                    selection,
                    selection_color,
                    selection_handler,
                    cx,
                ))
        }))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_inline_line(
    id: String,
    event_id: &str,
    selection_block: usize,
    source: &str,
    prefix: Option<&str>,
    view: EntityId,
    selection: Option<&FeedSelection>,
    selection_color: Hsla,
    selection_handler: Option<&FeedSelectionHandler>,
    cx: &App,
) -> AnyElement {
    let hovered_link = cx
        .try_global::<MarkdownLinkHover>()
        .and_then(|hover| hover.link_for(&id));
    let mut text = prefix.unwrap_or_default().to_owned();
    let mut highlights = Vec::<(Range<usize>, HighlightStyle)>::new();
    let mut link_ranges = Vec::new();
    let mut urls = Vec::new();
    append_inline_segment(
        source,
        None,
        &mut text,
        &mut highlights,
        &mut link_ranges,
        &mut urls,
        hovered_link,
    );

    render_prepared_line(
        id,
        event_id,
        selection_block,
        text,
        highlights,
        link_ranges,
        urls,
        view,
        selection,
        selection_color,
        selection_handler,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_plain_line(
    id: String,
    event_id: &str,
    selection_block: usize,
    text: String,
    view: EntityId,
    selection: Option<&FeedSelection>,
    selection_color: Hsla,
    selection_handler: Option<&FeedSelectionHandler>,
) -> AnyElement {
    render_prepared_line(
        id,
        event_id,
        selection_block,
        text,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        view,
        selection,
        selection_color,
        selection_handler,
        false,
    )
}

fn add_selection_highlight(
    text: &str,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    selection: Range<usize>,
    color: Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut boundaries = vec![0, text.len(), selection.start, selection.end];
    for (range, _) in &highlights {
        boundaries.push(range.start);
        boundaries.push(range.end);
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut merged = Vec::<(Range<usize>, HighlightStyle)>::new();
    for points in boundaries.windows(2) {
        let range = points[0]..points[1];
        if range.is_empty() {
            continue;
        }
        let base = highlights
            .iter()
            .find(|(highlighted, _)| {
                highlighted.start <= range.start && highlighted.end >= range.end
            })
            .map(|(_, style)| *style);
        let selected = selection.start <= range.start && selection.end >= range.end;
        if base.is_none() && !selected {
            continue;
        }
        let mut style = base.unwrap_or_default();
        if selected {
            style.background_color = Some(color);
        }
        let extend_previous = merged.last().is_some_and(|(previous, previous_style)| {
            previous.end == range.start && *previous_style == style
        });
        if extend_previous {
            merged.last_mut().unwrap().0.end = range.end;
        } else {
            merged.push((range, style));
        }
    }
    merged
}

fn feed_position(
    layout: &TextLayout,
    block: usize,
    position: gpui::Point<gpui::Pixels>,
) -> FeedPosition {
    (
        block,
        layout
            .index_for_position(position)
            .unwrap_or_else(|index| index),
    )
}

#[allow(clippy::too_many_arguments)]
fn render_prepared_line(
    id: String,
    event_id: &str,
    selection_block: usize,
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    link_ranges: Vec<Range<usize>>,
    urls: Vec<String>,
    view: EntityId,
    selection: Option<&FeedSelection>,
    selection_color: Hsla,
    selection_handler: Option<&FeedSelectionHandler>,
    wrap: bool,
) -> AnyElement {
    let highlights = match selection.and_then(|selection| {
        selection_range_for_block(selection, event_id, selection_block, &text)
    }) {
        Some(range) => add_selection_highlight(&text, highlights, range, selection_color),
        None => highlights,
    };

    let styled = StyledText::new(text).with_highlights(highlights);
    let layout = styled.layout().clone();
    let hover_id = id.clone();
    let hover_ranges = link_ranges.clone();
    let interactive = InteractiveText::new(SharedString::from(id.clone()), styled)
        .on_click(link_ranges, move |index, _, cx| cx.open_url(&urls[index]))
        .on_hover(move |character, _, _, cx| {
            let link = character.and_then(|character| {
                hover_ranges
                    .iter()
                    .position(|range| range.contains(&character))
            });
            let hover = cx.default_global::<MarkdownLinkHover>();
            if hover.set_link(&hover_id, link) {
                cx.notify(view);
            }
        });
    let leave_id = id.clone();
    let mut line = div()
        .id(SharedString::from(format!("{id}-hover")))
        .w_full()
        .min_w(px(0.))
        .when(wrap, |line| line.whitespace_normal())
        .when(!wrap, |line| line.whitespace_nowrap())
        .cursor(CursorStyle::IBeam)
        .on_hover(move |hovered, _, cx| {
            if cx
                .default_global::<MarkdownLinkHover>()
                .set_line_active(&leave_id, *hovered)
            {
                cx.notify(view);
            }
        })
        .when(cfg!(test), |line| {
            line.debug_selector(|| "MISSION_MARKDOWN_LINE".into())
        })
        .child(interactive);
    if let Some(handler) = selection_handler.cloned() {
        let down_handler = handler.clone();
        let down_layout = layout.clone();
        let down_event_id = event_id.to_owned();
        let move_handler = handler.clone();
        let move_layout = layout.clone();
        let move_event_id = event_id.to_owned();
        let up_handler = handler.clone();
        let up_layout = layout.clone();
        let up_event_id = event_id.to_owned();
        line = line
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                down_handler.handle(
                    FeedSelectionPhase::Begin,
                    &down_event_id,
                    feed_position(&down_layout, selection_block, event.position),
                    window,
                    cx,
                );
            })
            .on_mouse_move(move |event, window, cx| {
                move_handler.handle(
                    FeedSelectionPhase::Extend,
                    &move_event_id,
                    feed_position(&move_layout, selection_block, event.position),
                    window,
                    cx,
                );
            })
            .on_mouse_up(MouseButton::Left, move |event, window, cx| {
                up_handler.handle(
                    FeedSelectionPhase::End,
                    &up_event_id,
                    feed_position(&up_layout, selection_block, event.position),
                    window,
                    cx,
                );
            });
    }
    line.into_any_element()
}

fn next_link(source: &str) -> Option<(usize, &str, &str, usize)> {
    let markdown = source.find('[').and_then(|start| {
        let label_end = source[start + 1..].find(']')? + start + 1;
        let url_start = label_end + 1;
        let url = source.get(url_start..)?.strip_prefix('(')?;
        let url_end = url.find(')')?;
        Some((
            start,
            &source[start + 1..label_end],
            &url[..url_end],
            url_start + 1 + url_end + 1 - start,
        ))
    });
    let autolink = source.find("<http").and_then(|start| {
        let tail = &source[start + 1..];
        let end = tail.find('>')?;
        Some((start, &tail[..end], &tail[..end], end + 2))
    });
    [markdown, autolink]
        .into_iter()
        .flatten()
        .min_by_key(|candidate| candidate.0)
}

#[cfg(test)]
fn append_styled_segment(
    source: &str,
    base: Option<HighlightStyle>,
    text: &mut String,
    highlights: &mut Vec<(Range<usize>, HighlightStyle)>,
) {
    let mut link_ranges = Vec::new();
    let mut urls = Vec::new();
    append_inline_segment(
        source,
        base,
        text,
        highlights,
        &mut link_ranges,
        &mut urls,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn append_inline_segment(
    source: &str,
    base: Option<HighlightStyle>,
    text: &mut String,
    highlights: &mut Vec<(Range<usize>, HighlightStyle)>,
    link_ranges: &mut Vec<Range<usize>>,
    urls: &mut Vec<String>,
    hovered_link: Option<usize>,
) {
    let mut rest = source;
    while !rest.is_empty() {
        let marker = next_marker(rest);
        let link = next_link(rest);
        let marker_first = match (marker.as_ref(), link.as_ref()) {
            (Some(marker), Some(link)) => marker.0 <= link.0,
            (Some(_), None) => true,
            _ => false,
        };
        if !marker_first {
            let Some((start, label, url, consumed)) = link else {
                append_highlighted(rest, base, text, highlights);
                break;
            };
            append_highlighted(&rest[..start], base, text, highlights);
            let link_start = text.len();
            let link_index = link_ranges.len();
            let link_style = merge_highlight(
                base.unwrap_or_default(),
                HighlightStyle {
                    color: Some(theme::accent()),
                    underline: Some(UnderlineStyle {
                        color: Some(if hovered_link == Some(link_index) {
                            theme::accent()
                        } else {
                            theme::with_alpha(theme::accent(), 0.4)
                        }),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..Default::default()
                },
            );
            append_inline_segment(
                label,
                Some(link_style),
                text,
                highlights,
                link_ranges,
                urls,
                hovered_link,
            );
            link_ranges.push(link_start..text.len());
            urls.push(url.to_owned());
            rest = &rest[start + consumed..];
            continue;
        }
        let Some((start, marker, end_marker, style)) = marker else {
            append_highlighted(rest, base, text, highlights);
            break;
        };
        append_highlighted(&rest[..start], base, text, highlights);
        let body_start = start + marker.len();
        let Some(relative_end) = rest[body_start..].find(end_marker) else {
            append_highlighted(&rest[start..], base, text, highlights);
            break;
        };
        let body_end = body_start + relative_end;
        let merged = Some(merge_highlight(base.unwrap_or_default(), style));
        if marker == "`" {
            append_highlighted(&rest[body_start..body_end], merged, text, highlights);
        } else {
            append_inline_segment(
                &rest[body_start..body_end],
                merged,
                text,
                highlights,
                link_ranges,
                urls,
                hovered_link,
            );
        }
        rest = &rest[body_end + end_marker.len()..];
    }
}

fn append_highlighted(
    source: &str,
    style: Option<HighlightStyle>,
    text: &mut String,
    highlights: &mut Vec<(Range<usize>, HighlightStyle)>,
) {
    let start = text.len();
    text.push_str(source);
    if let Some(style) = style.filter(|_| start < text.len()) {
        highlights.push((start..text.len(), style));
    }
}

fn merge_highlight(base: HighlightStyle, overlay: HighlightStyle) -> HighlightStyle {
    HighlightStyle {
        color: overlay.color.or(base.color),
        font_weight: overlay.font_weight.or(base.font_weight),
        font_style: overlay.font_style.or(base.font_style),
        background_color: overlay.background_color.or(base.background_color),
        underline: overlay.underline.or(base.underline),
        strikethrough: overlay.strikethrough.or(base.strikethrough),
        fade_out: overlay.fade_out.or(base.fade_out),
    }
}

fn next_marker(source: &str) -> Option<(usize, &'static str, &'static str, HighlightStyle)> {
    let candidates = [
        (
            "~~",
            "~~",
            HighlightStyle {
                strikethrough: Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                }),
                ..Default::default()
            },
        ),
        (
            "**",
            "**",
            HighlightStyle {
                font_weight: Some(FontWeight::SEMIBOLD),
                ..Default::default()
            },
        ),
        (
            "`",
            "`",
            HighlightStyle {
                color: Some(theme::accent()),
                background_color: Some(theme::raised()),
                font_weight: Some(FontWeight::NORMAL),
                ..Default::default()
            },
        ),
        (
            "*",
            "*",
            HighlightStyle {
                font_style: Some(gpui::FontStyle::Italic),
                ..Default::default()
            },
        ),
    ];
    candidates
        .into_iter()
        .filter_map(|(marker, end, style)| {
            source.find(marker).map(|index| (index, marker, end, style))
        })
        .min_by_key(|candidate| candidate.0)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use gpui::{point, Context, Modifiers, Render, TestAppContext, VisualTestContext, Window};

    struct MarkdownWrapTest;

    struct MarkdownSpacingTest;

    struct MarkdownDragTest {
        handler: FeedSelectionHandler,
    }

    impl Render for MarkdownWrapTest {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(280.))
                .debug_selector(|| "MISSION_MARKDOWN_CONTAINER".into())
                .child(render_markdown(
                    "wrap-test",
                    "A narrow feed line with **styled text**, `inline code`, and [a link whose label also needs to wrap](https://example.com) before the final words.",
                    cx.entity_id(),
                    None,
                    theme::raised(),
                    None,
                    cx,
                ))
        }
    }

    impl Render for MarkdownSpacingTest {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(280.))
                .debug_selector(|| "MISSION_MARKDOWN_SPACING".into())
                .child(render_markdown(
                    "spacing-test",
                    "First paragraph\n\nSecond paragraph\n\n# Heading",
                    cx.entity_id(),
                    None,
                    theme::raised(),
                    None,
                    cx,
                ))
        }
    }

    impl Render for MarkdownDragTest {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().w(px(280.)).child(render_markdown(
                "drag-test",
                "Select this sentence",
                cx.entity_id(),
                None,
                theme::raised(),
                Some(&self.handler),
                cx,
            ))
        }
    }

    #[test]
    fn block_parser_preserves_chat_markdown_structure() {
        assert_eq!(
            parse_blocks(
                "# Title\n\n- one\n2. two\n\n> quote\n\n---\n\n***\n\n```rs\nlet x = 1;\n```"
            ),
            vec![
                MarkdownBlock::Heading(1, "Title".into()),
                MarkdownBlock::ListItem {
                    marker: "•".into(),
                    text: "one".into()
                },
                MarkdownBlock::ListItem {
                    marker: "2.".into(),
                    text: "two".into()
                },
                MarkdownBlock::Quote("quote".into()),
                MarkdownBlock::Rule,
                MarkdownBlock::Rule,
                MarkdownBlock::Code("let x = 1;".into()),
            ]
        );
    }

    #[test]
    fn block_parser_supports_gfm_tables_and_links() {
        assert_eq!(
            parse_blocks("| Name | State |\n| --- | :---: |\n| coder | busy |"),
            vec![MarkdownBlock::Table {
                header: vec!["Name".into(), "State".into()],
                rows: vec![vec!["coder".into(), "busy".into()]],
            }]
        );
        assert_eq!(
            next_link("See [Runner](https://example.com)."),
            Some((4, "Runner", "https://example.com", 29))
        );
    }

    #[test]
    fn link_hover_returns_when_reentering_a_cached_line() {
        let mut hover = MarkdownLinkHover::default();
        assert!(hover.set_link("line-a", Some(1)));
        assert!(hover.set_line_active("line-a", true));
        assert_eq!(hover.link_for("line-a"), Some(1));

        assert!(hover.set_line_active("line-a", false));
        assert_eq!(hover.link_for("line-a"), None);
        assert!(hover.set_link("line-b", Some(0)));
        assert!(hover.set_line_active("line-b", true));
        assert_eq!(hover.link_for("line-b"), Some(0));

        assert!(hover.set_line_active("line-b", false));
        assert!(hover.set_line_active("line-a", true));
        assert_eq!(hover.link_for("line-a"), Some(1));
    }

    #[test]
    fn inline_markers_recurse_and_code_overrides_outer_weight() {
        let mut text = String::new();
        let mut highlights = Vec::new();
        append_styled_segment(
            "**`runner-app` is the binary**",
            None,
            &mut text,
            &mut highlights,
        );

        assert_eq!(text, "runner-app is the binary");
        let code = highlights
            .iter()
            .find(|(range, _)| range.contains(&0))
            .map(|(_, style)| style)
            .expect("inline code style");
        assert_eq!(code.font_weight, Some(FontWeight::NORMAL));
        assert_eq!(code.color, Some(theme::accent()));
        assert_eq!(code.background_color, Some(theme::raised()));
        let bold = highlights
            .iter()
            .find(|(range, _)| range.contains(&12))
            .map(|(_, style)| style)
            .expect("outer bold style");
        assert_eq!(bold.font_weight, Some(FontWeight::SEMIBOLD));
    }

    #[test]
    fn inline_links_nest_in_strong_but_stay_literal_inside_code() {
        let mut text = String::new();
        let mut highlights = Vec::new();
        let mut links = Vec::new();
        let mut urls = Vec::new();
        append_inline_segment(
            "**[Runner](https://example.com)** and `[literal](url)`",
            None,
            &mut text,
            &mut highlights,
            &mut links,
            &mut urls,
            None,
        );

        assert_eq!(text, "Runner and [literal](url)");
        assert_eq!(links, vec![0..6]);
        assert_eq!(urls, vec!["https://example.com"]);
        assert_eq!(
            highlights
                .iter()
                .find(|(range, _)| range.contains(&0))
                .unwrap()
                .1
                .font_weight,
            Some(FontWeight::SEMIBOLD)
        );
    }

    #[test]
    fn strong_marker_wins_ties_and_unbalanced_markers_stay_literal() {
        let mut text = String::new();
        let mut highlights = Vec::new();
        append_styled_segment("**bold**", None, &mut text, &mut highlights);
        assert_eq!(text, "bold");
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].1.font_weight, Some(FontWeight::SEMIBOLD));
        assert_eq!(highlights[0].1.font_style, None);

        text.clear();
        highlights.clear();
        append_styled_segment("**a", None, &mut text, &mut highlights);
        assert_eq!(text, "**a");
        assert!(highlights.is_empty());

        text.clear();
        append_styled_segment("`a**b`", None, &mut text, &mut highlights);
        assert_eq!(text, "a**b");
        assert_eq!(highlights.len(), 1);
    }

    #[test]
    fn feed_selection_orders_offsets_and_copies_rendered_blocks() {
        let source = "# **Title**\n\n- one `code`\n2. two\n\nParagraph";
        assert_eq!(
            rendered_plain_text_blocks(source),
            vec!["Title", "• one code", "2. two", "Paragraph"]
        );
        let selection = FeedSelection {
            event_id: "event-1".into(),
            anchor: (3, 4),
            head: (1, 4),
        };
        assert_eq!(
            selected_plain_text(source, &selection).as_deref(),
            Some("one code\n2. two\nPara")
        );
        assert_eq!(
            selection_range_for_block(&selection, "event-1", 2, "2. two"),
            Some(0..6)
        );
        assert_eq!(
            selection_range_for_block(&selection, "event-2", 2, "2. two"),
            None
        );
    }

    #[test]
    fn inline_markdown_wraps_inside_a_narrow_feed() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|_, _| MarkdownWrapTest);
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let container = window
            .debug_bounds("MISSION_MARKDOWN_CONTAINER")
            .expect("container bounds");
        let line = window
            .debug_bounds("MISSION_MARKDOWN_LINE")
            .expect("line bounds");
        assert_eq!(container.size.width, px(280.));
        assert!(line.size.height > px(20.));
    }

    #[test]
    fn markdown_layout_collapses_adjacent_block_margins() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|_, _| MarkdownSpacingTest);
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let first = window.debug_bounds("MISSION_MARKDOWN_BLOCK_0").unwrap();
        let second = window.debug_bounds("MISSION_MARKDOWN_BLOCK_1").unwrap();
        let heading = window.debug_bounds("MISSION_MARKDOWN_BLOCK_2").unwrap();
        let body = window.debug_bounds("MISSION_MARKDOWN_SPACING").unwrap();

        assert_eq!(
            second.origin.y - first.origin.y - first.size.height,
            px(12.)
        );
        assert_eq!(
            heading.origin.y - second.origin.y - second.size.height,
            px(12.)
        );
        assert_eq!(
            body.origin.y + body.size.height - heading.origin.y - heading.size.height,
            px(6.)
        );
    }

    #[test]
    fn markdown_drag_maps_pointer_positions_to_feed_offsets() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let recorded = events.clone();
        let handler = FeedSelectionHandler::new(move |phase, event_id, position, _, _| {
            recorded
                .borrow_mut()
                .push((phase, event_id.to_owned(), position));
        });
        let mut cx = TestAppContext::single();
        let window = cx.add_window(move |_, _| MarkdownDragTest { handler });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let line = window.debug_bounds("MISSION_MARKDOWN_LINE").unwrap();
        let start = point(
            line.origin.x + px(1.),
            line.origin.y + line.size.height / 2.,
        );
        let end = point(
            line.origin.x + line.size.width.min(px(80.)),
            line.origin.y + line.size.height / 2.,
        );
        window.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        window.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
        window.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());

        let events = events.borrow();
        assert_eq!(
            events[0],
            (FeedSelectionPhase::Begin, "drag-test".into(), (0, 0))
        );
        assert!(events.iter().any(|(phase, event_id, position)| {
            *phase == FeedSelectionPhase::Extend
                && event_id == "drag-test"
                && position.0 == 0
                && position.1 > 0
        }));
        assert_eq!(events.last().unwrap().0, FeedSelectionPhase::End);
    }
}
