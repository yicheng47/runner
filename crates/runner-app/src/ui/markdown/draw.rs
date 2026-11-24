use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, img, px, relative, svg, AnyElement, App, Font, FontStyle, FontWeight, Hsla, ImageSource,
    InteractiveText, Pixels, SharedString, StrikethroughStyle, StyledText, TextRun, UnderlineStyle,
    Window,
};
use pulldown_cmark::Alignment;

use super::layout::{Marker, Row, RowKind};
use super::links::route;
use super::view::{dragged, MarkdownView, OpenLink, Position};
use super::{cell_text, Cell, CodeBlock, ImageRef, Inline, Piece};
use crate::surfaces::files::syntax::SyntaxTheme;
use crate::theme;

/// The rendered page's side margins and the gap above its first block.
pub(super) const MARGIN: f32 = 20.;

/// How far each list level indents its rows.
const LIST_INDENT: f32 = 22.;

/// `runs` with the bytes in `selected` drawn on the selection colour.
pub(super) fn with_selection(
    runs: Vec<TextRun>,
    selected: Range<usize>,
    color: Hsla,
) -> Vec<TextRun> {
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

/// The page's sizes, from Settings → Side panel → Files → Font size, its
/// code font and whether code blocks are highlighted.
#[derive(Clone)]
pub(super) struct Look {
    base: f32,
    ui: Font,
    mono: Font,
    syntax: SyntaxTheme,
}

impl Look {
    pub(super) fn current(window: &Window, cx: &App) -> Self {
        let (mono, size) = match crate::surfaces::files::settings(cx) {
            Some(settings) => (settings.files_font(cx), settings.files_font_size() as f32),
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

pub(super) struct Draw<'a> {
    pub(super) look: &'a Look,
    pub(super) images: &'a HashMap<String, Arc<gpui::Image>>,
    pub(super) from: &'a str,
    pub(super) open_link: Option<OpenLink>,
    pub(super) view: gpui::WeakEntity<MarkdownView>,
    pub(super) index: usize,
    /// The bytes of the row's text the selection covers.
    pub(super) selected: Range<usize>,
}

impl Draw<'_> {
    /// A row inside its quotes and list indent, with its marker.
    pub(super) fn row(&self, row: &Row, top: f32) -> AnyElement {
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
        super::view::DRAWN_CODE_BYTES.set(super::view::DRAWN_CODE_BYTES.get() + text.len());
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
                    .children(cell.iter().enumerate().map(|(piece_index, piece)| {
                        let body = match piece {
                            Piece::Text(text) => {
                                let id = format!("md-{}-{column}-{piece_index}", self.index);
                                self.inline_plain(text, color, weight, id)
                            }
                            Piece::Image(image) => self.image(
                                image,
                                format!("md-{}-{column}-{piece_index}-image", self.index),
                            ),
                        };
                        div()
                            .when(cfg!(test), move |element| {
                                element.debug_selector(move || {
                                    format!("FILE_MD_CELL_PIECE {row}-{column}-{piece_index}")
                                })
                            })
                            // GPUI text gives its unwrapped width as its minimum.
                            .min_w(px(0.))
                            .child(body)
                    }))
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
