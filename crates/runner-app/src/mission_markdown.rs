use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, FontWeight, HighlightStyle, SharedString, StrikethroughStyle,
    StyledText,
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

pub(crate) fn render_markdown(id: &str, text: &str) -> AnyElement {
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
                .map(|(index, block)| render_block(&id, index, block)),
        )
        .into_any_element()
}

fn render_block(id: &str, index: usize, block: MarkdownBlock) -> AnyElement {
    match block {
        MarkdownBlock::Paragraph(text) => div()
            .flex()
            .flex_col()
            .line_height(rems(20. / 16.))
            .children(text.lines().map(render_inline_line))
            .into_any_element(),
        MarkdownBlock::Heading(level, text) => div()
            .mt_2()
            .mb_1()
            .text_size(rems(if level == 1 { 14. / 16. } else { 13. / 16. }))
            .font_weight(FontWeight::SEMIBOLD)
            .child(render_inline_line(&text))
            .into_any_element(),
        MarkdownBlock::ListItem { marker, text } => div()
            .pl_4()
            .flex()
            .items_start()
            .gap_2()
            .line_height(rems(20. / 16.))
            .child(marker)
            .child(
                div()
                    .min_w(gpui::px(0.))
                    .flex_1()
                    .child(render_inline_line(&text)),
            )
            .into_any_element(),
        MarkdownBlock::Quote(text) => div()
            .my_1()
            .border_l_2()
            .border_color(theme::border_strong())
            .pl_3()
            .text_color(theme::muted())
            .child(render_inline_line(&text))
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
                    .child(render_table_row(header, true))
                    .children(rows.into_iter().map(|row| render_table_row(row, false))),
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

fn render_table_row(cells: Vec<String>, heading: bool) -> AnyElement {
    div()
        .flex()
        .children(cells.into_iter().map(|cell| {
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
                .child(render_inline_line(&cell))
        }))
        .into_any_element()
}

fn render_inline_line(source: &str) -> AnyElement {
    let mut children = Vec::new();
    let mut rest = source;
    while let Some((start, label, url, consumed)) = next_link(rest) {
        if start > 0 {
            children.push(
                div()
                    .whitespace_normal()
                    .child(styled_inline(&rest[..start]))
                    .into_any_element(),
            );
        }
        let open_url = url.to_owned();
        children.push(
            div()
                .id(SharedString::from(format!("markdown-link-{open_url}")))
                .cursor_pointer()
                .text_color(theme::accent())
                .underline()
                .text_decoration_color(theme::with_alpha(theme::accent(), 0.4))
                .hover(|link| link.text_decoration_color(theme::accent()))
                .on_click(move |_, _, cx| cx.open_url(&open_url))
                .child(label.to_owned())
                .into_any_element(),
        );
        rest = &rest[start + consumed..];
    }
    if !rest.is_empty() || children.is_empty() {
        children.push(
            div()
                .whitespace_normal()
                .child(styled_inline(rest))
                .into_any_element(),
        );
    }
    div()
        .min_w(px(0.))
        .flex()
        .flex_wrap()
        .children(children)
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

fn styled_inline(source: &str) -> StyledText {
    let mut text = String::new();
    let mut highlights = Vec::<(Range<usize>, HighlightStyle)>::new();
    let mut rest = source;
    while !rest.is_empty() {
        let Some((start, marker, end_marker, style)) = next_marker(rest) else {
            text.push_str(rest);
            break;
        };
        text.push_str(&rest[..start]);
        let body_start = start + marker.len();
        let Some(relative_end) = rest[body_start..].find(end_marker) else {
            text.push_str(&rest[start..]);
            break;
        };
        let body_end = body_start + relative_end;
        let rendered_start = text.len();
        text.push_str(&rest[body_start..body_end]);
        highlights.push((rendered_start..text.len(), style));
        rest = &rest[body_end + end_marker.len()..];
    }
    StyledText::new(text).with_highlights(highlights)
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
}
