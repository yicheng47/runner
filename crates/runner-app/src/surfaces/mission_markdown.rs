use std::collections::HashMap;
use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, App, EntityId, FontWeight, Global, HighlightStyle, InteractiveText,
    SharedString, StrikethroughStyle, StyledText, UnderlineStyle,
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

pub(crate) fn render_markdown(id: &str, text: &str, view: EntityId, cx: &App) -> AnyElement {
    let blocks = parse_blocks(text);
    let id = id.to_owned();
    div()
        .min_w(gpui::px(0.))
        .flex()
        .flex_col()
        .gap_1()
        .children(
            blocks
                .into_iter()
                .enumerate()
                .map(|(index, block)| render_block(&id, index, block, view, cx)),
        )
        .into_any_element()
}

fn render_block(
    id: &str,
    index: usize,
    block: MarkdownBlock,
    view: EntityId,
    cx: &App,
) -> AnyElement {
    match block {
        MarkdownBlock::Paragraph(text) => div()
            .flex()
            .flex_col()
            .line_height(rems(20. / 16.))
            .children(text.lines().enumerate().map(|(line, text)| {
                render_inline_line(format!("{id}-{index}-paragraph-{line}"), text, view, cx)
            }))
            .into_any_element(),
        MarkdownBlock::Heading(level, text) => div()
            .mt_2()
            .mb_1()
            .text_size(rems(if level == 1 { 14. / 16. } else { 13. / 16. }))
            .font_weight(FontWeight::SEMIBOLD)
            .child(render_inline_line(
                format!("{id}-{index}-heading"),
                &text,
                view,
                cx,
            ))
            .into_any_element(),
        MarkdownBlock::ListItem { marker, text } => div()
            .pl_4()
            .flex()
            .items_start()
            .gap_2()
            .line_height(rems(20. / 16.))
            .child(marker)
            .child(div().min_w(gpui::px(0.)).flex_1().child(render_inline_line(
                format!("{id}-{index}-list"),
                &text,
                view,
                cx,
            )))
            .into_any_element(),
        MarkdownBlock::Quote(text) => div()
            .my_1()
            .border_l_2()
            .border_color(theme::border_strong())
            .pl_3()
            .text_color(theme::muted())
            .child(render_inline_line(
                format!("{id}-{index}-quote"),
                &text,
                view,
                cx,
            ))
            .into_any_element(),
        MarkdownBlock::Code(text) => div()
            .id((SharedString::from(id.to_owned()), index))
            .my_1()
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
            .child(text)
            .into_any_element(),
        MarkdownBlock::Table { header, rows } => div()
            .id((SharedString::from(id.to_owned()), index))
            .my_2()
            .overflow_x_scroll()
            .child(
                div()
                    .min_w(px(420.))
                    .flex()
                    .flex_col()
                    .child(render_table_row(id, index, 0, header, true, view, cx))
                    .children(rows.into_iter().enumerate().map(|(row_index, row)| {
                        render_table_row(id, index, row_index + 1, row, false, view, cx)
                    })),
            )
            .into_any_element(),
        MarkdownBlock::Rule => div()
            .my_2()
            .h(rems(1. / 16.))
            .w_full()
            .bg(theme::border())
            .into_any_element(),
    }
}

fn render_table_row(
    id: &str,
    block_index: usize,
    row_index: usize,
    cells: Vec<String>,
    heading: bool,
    view: EntityId,
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
                    &cell,
                    view,
                    cx,
                ))
        }))
        .into_any_element()
}

fn render_inline_line(id: String, source: &str, view: EntityId, cx: &App) -> AnyElement {
    let hovered_link = cx
        .try_global::<MarkdownLinkHover>()
        .and_then(|hover| hover.link_for(&id));
    let mut text = String::new();
    let mut highlights = Vec::<(Range<usize>, HighlightStyle)>::new();
    let mut link_ranges = Vec::new();
    let mut urls = Vec::new();
    let mut rest = source;
    while let Some((start, label, url, consumed)) = next_link(rest) {
        if start > 0 {
            append_styled_segment(&rest[..start], None, &mut text, &mut highlights);
        }
        let link_start = text.len();
        let link_index = link_ranges.len();
        append_styled_segment(
            label,
            Some(HighlightStyle {
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
            }),
            &mut text,
            &mut highlights,
        );
        link_ranges.push(link_start..text.len());
        urls.push(url.to_owned());
        rest = &rest[start + consumed..];
    }
    append_styled_segment(rest, None, &mut text, &mut highlights);

    let styled = StyledText::new(text).with_highlights(highlights);
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
    div()
        .id(SharedString::from(format!("{id}-hover")))
        .w_full()
        .min_w(px(0.))
        .whitespace_normal()
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
        .child(interactive)
        .into_any_element()
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

fn append_styled_segment(
    source: &str,
    base: Option<HighlightStyle>,
    text: &mut String,
    highlights: &mut Vec<(Range<usize>, HighlightStyle)>,
) {
    let mut rest = source;
    while !rest.is_empty() {
        let Some((start, marker, end_marker, style)) = next_marker(rest) else {
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
        append_highlighted(
            &rest[body_start..body_end],
            Some(merge_highlight(base.unwrap_or_default(), style)),
            text,
            highlights,
        );
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
    use super::*;
    use gpui::{Context, Render, TestAppContext, VisualTestContext, Window};

    struct MarkdownWrapTest;

    impl Render for MarkdownWrapTest {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(280.))
                .debug_selector(|| "MISSION_MARKDOWN_CONTAINER".into())
                .child(render_markdown(
                    "wrap-test",
                    "A narrow feed line with **styled text**, `inline code`, and [a link whose label also needs to wrap](https://example.com) before the final words.",
                    cx,
                ))
        }
    }

    #[test]
    fn block_parser_preserves_chat_markdown_structure() {
        assert_eq!(
            parse_blocks("# Title\n\n- one\n2. two\n\n> quote\n\n```rs\nlet x = 1;\n```"),
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
}
