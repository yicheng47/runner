use std::cell::Cell as StdCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, img, px, relative, rems, svg, AnyElement, App, Bounds, ClipboardItem, CursorStyle,
    EntityId, Font, FontStyle, FontWeight, Global, HighlightStyle, Hsla, ImageSource,
    InteractiveText, MouseButton, Pixels, ScrollWheelEvent, SharedString, StrikethroughStyle,
    StyledText, TextLayout, TextRun, UnderlineStyle, Window,
};
use pulldown_cmark::Alignment;

use super::code::{
    CodeBlockId, CodeBlockStates, CompactCodeStates, COPIED_DURATION, MIN_SCROLLBAR_THUMB,
};
use super::feed::{CompactHeightHandler, FeedSelectionHandler, FeedSelectionPhase};
use super::layout::{Marker, Row, RowKind};
use super::links::{is_web_url, route};
use super::view::{document_allows_click, MarkdownView, OpenLink, Position};
use super::{cell_text, Cell, CodeBlock, ImageRef, Inline, Piece};
use crate::surfaces::files::syntax::SyntaxTheme;
use crate::theme;
use crate::ui::Tooltip;

/// The rendered page's side margins and the gap above its first block.
pub(super) const MARGIN: f32 = 20.;

/// How far each list level indents its rows.
const LIST_INDENT: f32 = 22.;

/// Guards link clicks against secondary-button clicks/drags, drags that
/// leave the preview or return to their origin, and displaced releases
/// without an intermediate mouse-move. Driven from capture-phase down/up
/// handlers and window-wide move/exit listeners before GPUI's
/// `InteractiveText` runs its button-agnostic bubble-phase click callback.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ClickGuard {
    press: Option<gpui::Point<Pixels>>,
    press_bounds: Option<gpui::Bounds<Pixels>>,
    moved: bool,
    allow_click: bool,
}

impl ClickGuard {
    fn moved_past_threshold(from: gpui::Point<Pixels>, to: gpui::Point<Pixels>) -> bool {
        let delta = to - from;
        delta.x.abs() > px(2.) || delta.y.abs() > px(2.)
    }

    pub(super) fn on_mouse_down(
        &mut self,
        button: MouseButton,
        position: gpui::Point<Pixels>,
        bounds: gpui::Bounds<Pixels>,
    ) {
        self.allow_click = false;
        if button == MouseButton::Left {
            self.press = Some(position);
            self.press_bounds = Some(bounds);
            self.moved = false;
        } else {
            self.press = None;
            self.press_bounds = None;
            self.moved = true;
        }
    }

    pub(super) fn on_mouse_move(
        &mut self,
        position: gpui::Point<Pixels>,
        pressed_button: Option<MouseButton>,
    ) {
        let Some(press) = self.press else {
            return;
        };
        if pressed_button != Some(MouseButton::Left) {
            self.press = None;
            self.press_bounds = None;
            self.moved = true;
            return;
        }
        let left_bounds = self
            .press_bounds
            .is_some_and(|bounds| !bounds.contains(&position));
        if left_bounds || Self::moved_past_threshold(press, position) {
            self.moved = true;
        }
    }

    pub(super) fn on_mouse_exit(&mut self) {
        if self.press.is_some() {
            self.moved = true;
        }
    }

    pub(super) fn on_mouse_up(&mut self, button: MouseButton, position: gpui::Point<Pixels>) {
        self.press_bounds = None;
        if button == MouseButton::Left {
            if let Some(press) = self.press.take() {
                if Self::moved_past_threshold(press, position) {
                    self.moved = true;
                }
                self.allow_click = !self.moved;
            } else {
                self.allow_click = false;
            }
        } else {
            self.allow_click = false;
            self.press = None;
        }
    }

    pub(super) fn reset(&mut self) {
        self.press = None;
        self.press_bounds = None;
        self.moved = false;
        self.allow_click = false;
    }

    pub(super) fn allows_click(&self) -> bool {
        self.allow_click
    }

    pub(super) fn moved(&self) -> bool {
        self.moved
    }
}

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

    pub(super) fn line_height(&self) -> Pixels {
        px((self.base * 1.6).round())
    }

    fn code_size(&self) -> Pixels {
        px((self.base * 0.92).round())
    }
}

#[derive(Default)]
pub(super) struct MarkdownLinkHover {
    active_line: Option<String>,
    links: HashMap<String, usize>,
    click_guard: ClickGuard,
}

impl Global for MarkdownLinkHover {}

impl MarkdownLinkHover {
    pub(super) fn link_for(&self, line: &str) -> Option<usize> {
        (self.active_line.as_deref() == Some(line))
            .then(|| self.links.get(line).copied())
            .flatten()
    }

    pub(super) fn set_link(&mut self, line: &str, link: Option<usize>) -> bool {
        match link {
            Some(link) => self.links.insert(line.to_owned(), link) != Some(link),
            None => self.links.remove(line).is_some(),
        }
    }

    pub(super) fn set_line_active(&mut self, line: &str, active: bool) -> bool {
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

pub(super) fn note_compact_press(
    button: MouseButton,
    position: gpui::Point<Pixels>,
    bounds: gpui::Bounds<Pixels>,
    cx: &mut App,
) {
    cx.default_global::<MarkdownLinkHover>()
        .click_guard
        .on_mouse_down(button, position, bounds);
}

pub(super) fn note_compact_move(
    position: gpui::Point<Pixels>,
    pressed_button: Option<MouseButton>,
    cx: &mut App,
) {
    if cx.has_global::<MarkdownLinkHover>() {
        cx.global_mut::<MarkdownLinkHover>()
            .click_guard
            .on_mouse_move(position, pressed_button);
    }
    if cx.has_global::<CompactCodeStates>() {
        let states = cx.global_mut::<CompactCodeStates>();
        let changed = if pressed_button == Some(MouseButton::Left) {
            states.update_scrollbar_drags(f32::from(position.x))
        } else {
            states.end_scrollbar_drags()
        };
        for view in changed {
            cx.notify(view);
        }
    }
}

pub(super) fn note_compact_exit(cx: &mut App) {
    if cx.has_global::<MarkdownLinkHover>() {
        cx.global_mut::<MarkdownLinkHover>()
            .click_guard
            .on_mouse_exit();
    }
    if cx.has_global::<CompactCodeStates>() {
        let changed = cx.global_mut::<CompactCodeStates>().clear_all_hovers();
        for view in changed {
            cx.notify(view);
        }
    }
}

pub(super) fn note_compact_release(
    button: MouseButton,
    position: gpui::Point<Pixels>,
    cx: &mut App,
) {
    if cx.has_global::<MarkdownLinkHover>() {
        cx.global_mut::<MarkdownLinkHover>()
            .click_guard
            .on_mouse_up(button, position);
    }
    if button == MouseButton::Left && cx.has_global::<CompactCodeStates>() {
        let changed = cx.global_mut::<CompactCodeStates>().end_scrollbar_drags();
        for view in changed {
            cx.notify(view);
        }
    }
}

/// Clears any in-flight compact press when a drag ends outside the feed.
pub(crate) fn reset_compact_press(cx: &mut App) {
    if cx.has_global::<MarkdownLinkHover>() {
        cx.global_mut::<MarkdownLinkHover>().click_guard.reset();
    }
    if cx.has_global::<CompactCodeStates>() {
        let changed = cx.global_mut::<CompactCodeStates>().end_scrollbar_drags();
        for view in changed {
            cx.notify(view);
        }
    }
}

fn compact_allows_click(cx: &App) -> bool {
    cx.try_global::<MarkdownLinkHover>()
        .is_some_and(|hover| hover.click_guard.allows_click())
}

fn compact_press_moved(cx: &App) -> bool {
    cx.try_global::<MarkdownLinkHover>()
        .is_some_and(|hover| hover.click_guard.moved())
}

pub(super) fn openable_links(
    links: &[(Range<usize>, Arc<str>)],
) -> (Vec<Range<usize>>, Vec<Arc<str>>) {
    let mut ranges = Vec::new();
    let mut urls = Vec::new();
    for (range, href) in links {
        if is_web_url(href) {
            ranges.push(range.clone());
            urls.push(Arc::clone(href));
        }
    }
    (ranges, urls)
}

/// Builds GPUI [`HighlightStyle`] runs for a Compact [`Inline`], overlaying
/// the hovered link underline and the active selection background.
pub(super) fn compact_highlights(
    inline: &Inline,
    hovered_range: Option<Range<usize>>,
    selected: Range<usize>,
    selection_color: Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut merged: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for (range, style) in &inline.styles {
        let (start, end) = (range.start, range.end);
        let mut cuts = [start, end, start, end, start, end];
        let mut len = 2;
        if let Some(hovered) = &hovered_range {
            cuts[len] = hovered.start.clamp(start, end);
            cuts[len + 1] = hovered.end.clamp(start, end);
            len += 2;
        }
        if !selected.is_empty() {
            cuts[len] = selected.start.clamp(start, end);
            cuts[len + 1] = selected.end.clamp(start, end);
            len += 2;
        }
        cuts[..len].sort_unstable();
        for pair in cuts[..len].windows(2) {
            if pair[1] <= pair[0] {
                continue;
            }
            let sub = pair[0]..pair[1];
            let is_hovered = hovered_range
                .as_ref()
                .is_some_and(|hovered| sub.start >= hovered.start && sub.end <= hovered.end);
            let is_selected =
                !selected.is_empty() && sub.start >= selected.start && sub.end <= selected.end;

            let mut hl = HighlightStyle::default();
            if style.strong && !style.code {
                hl.font_weight = Some(FontWeight::SEMIBOLD);
            }
            if style.emphasis {
                hl.font_style = Some(FontStyle::Italic);
            }
            if style.strike {
                hl.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                });
            }
            if style.code {
                hl.color = Some(theme::accent());
                hl.background_color = Some(theme::raised());
                hl.font_weight = Some(FontWeight::NORMAL);
            }
            if style.link {
                hl.color = Some(theme::accent());
                hl.underline = Some(UnderlineStyle {
                    color: Some(if is_hovered {
                        theme::accent()
                    } else {
                        theme::with_alpha(theme::accent(), 0.4)
                    }),
                    thickness: px(1.),
                    wavy: false,
                });
            }
            if is_selected {
                hl.background_color = Some(selection_color);
            }
            if hl == HighlightStyle::default() {
                continue;
            }
            if let Some(last) = merged
                .last_mut()
                .filter(|(prev, prev_style)| prev.end == sub.start && *prev_style == hl)
            {
                last.0.end = sub.end;
            } else {
                merged.push((sub, hl));
            }
        }
    }
    merged
}

/// Builds GPUI [`HighlightStyle`] runs for a highlighted Compact code line,
/// combining syntax colours with the active selection background.
pub(super) fn compact_code_highlights(
    spans: &[(usize, Option<u16>)],
    syntax: Option<&SyntaxTheme>,
    selected: Range<usize>,
    selection_color: Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut merged: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut at = 0;
    for &(len, capture) in spans {
        let (start, end) = (at, at + len);
        at = end;
        let color = capture.and_then(|capture| syntax?.color(capture));
        let (cuts, cut_len) = if selected.is_empty() {
            ([start, end, end, end], 2)
        } else {
            (
                [
                    start,
                    selected.start.clamp(start, end),
                    selected.end.clamp(start, end),
                    end,
                ],
                4,
            )
        };
        for pair in cuts[..cut_len].windows(2) {
            if pair[1] <= pair[0] {
                continue;
            }
            let sub = pair[0]..pair[1];
            let is_selected =
                !selected.is_empty() && sub.start >= selected.start && sub.end <= selected.end;
            let mut hl = HighlightStyle::default();
            if let Some(color) = color {
                hl.color = Some(color);
            }
            if is_selected {
                hl.background_color = Some(selection_color);
            }
            if hl == HighlightStyle::default() {
                continue;
            }
            if let Some(last) = merged
                .last_mut()
                .filter(|(prev, prev_style)| prev.end == sub.start && *prev_style == hl)
            {
                last.0.end = sub.end;
            } else {
                merged.push((sub, hl));
            }
        }
    }
    merged
}

fn text_position(layout: &TextLayout, row: usize, position: gpui::Point<Pixels>) -> Position {
    Position {
        row,
        offset: layout
            .index_for_position(position)
            .unwrap_or_else(|index| index),
    }
}

/// Look-specific styling, spacing and interaction state for [`Draw`].
pub(super) enum Surface<'a> {
    Document {
        look: &'a Look,
        images: &'a HashMap<String, Arc<gpui::Image>>,
        from: &'a str,
        open_link: Option<OpenLink>,
        view: gpui::WeakEntity<MarkdownView>,
        code_states: &'a CodeBlockStates,
        top: f32,
    },
    Compact {
        doc_id: &'a str,
        view: EntityId,
        syntax: Option<&'a SyntaxTheme>,
        selection_color: Hsla,
        selection_handler: Option<&'a FeedSelectionHandler>,
        on_height_change: Option<&'a CompactHeightHandler>,
        cx: &'a App,
    },
}

/// Shared row renderer for both Document (Files) and Compact (feed, question
/// cards, prompts, Skills, Info) looks.
pub(super) struct Draw<'a> {
    pub(super) surface: Surface<'a>,
    pub(super) index: usize,
    /// The bytes of the row's text the selection covers.
    pub(super) selected: Range<usize>,
}

impl Draw<'_> {
    /// A row inside its quotes and list indent, with its marker.
    pub(super) fn row(&self, row: &Row) -> AnyElement {
        let mut body = match &self.surface {
            Surface::Document { look, top, .. } => div()
                .flex()
                .pt(px(*top))
                .text_size(px(look.base))
                .line_height(look.line_height()),
            Surface::Compact { .. } => {
                #[cfg(test)]
                super::feed::RENDER_COUNT.set(super::feed::RENDER_COUNT.get() + 1);
                div()
                    .flex()
                    .w_full()
                    .min_w(px(0.))
                    .when(row.inner_top > 0., |el| el.pt(px(row.inner_top)))
            }
        };
        if row.indent > 0 {
            let marker = row.marker.map(|marker| self.marker(marker));
            let index = self.index;
            let gutter = match &self.surface {
                Surface::Document { .. } => {
                    body = body.pl(px(LIST_INDENT * f32::from(row.indent - 1)));
                    div()
                        .when(cfg!(test) && marker.is_some(), |el| {
                            el.debug_selector(move || format!("FILE_MD_MARKER {index}"))
                        })
                        .flex_none()
                        .w(px(LIST_INDENT))
                        .children(marker)
                }
                Surface::Compact { .. } => {
                    body = body.pl(rems(LIST_INDENT * f32::from(row.indent - 1) / 16.));
                    div()
                        .when(cfg!(test) && marker.is_some(), |el| {
                            el.debug_selector(move || format!("COMPACT_MD_MARKER {index}"))
                        })
                        .flex_none()
                        .min_w(rems(LIST_INDENT / 16.))
                        .pr(rems(4. / 16.))
                        .children(marker)
                }
            };
            body = body.child(gutter);
        }
        let content = div().flex_1().min_w(px(0.)).child(self.content(row));
        let mut element = body.child(content).into_any_element();
        for _ in 0..row.quotes {
            element = match &self.surface {
                Surface::Document { .. } => div()
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "FILE_MD_QUOTE".into())
                    })
                    .pl(px(12.))
                    .border_l(px(3.))
                    .border_color(theme::border_strong())
                    .child(element)
                    .into_any_element(),
                Surface::Compact { .. } => div()
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "COMPACT_MD_QUOTE".into())
                    })
                    .border_l_2()
                    .border_color(theme::border_strong())
                    .pl_3()
                    .text_color(theme::muted())
                    .child(element)
                    .into_any_element(),
            };
        }
        match &self.surface {
            Surface::Document { .. } => element,
            Surface::Compact { .. } => {
                let index = self.index;
                div()
                    .w_full()
                    .min_w(px(0.))
                    .when(row.top > 0., |el| el.mt(px(row.top)))
                    .when(cfg!(test), |el| {
                        el.debug_selector(move || format!("COMPACT_MD_ROW {index}"))
                    })
                    .child(element)
                    .into_any_element()
            }
        }
    }

    fn marker(&self, marker: Marker) -> AnyElement {
        let base = match &self.surface {
            Surface::Document { look, .. } => div().h(look.line_height()),
            Surface::Compact { .. } => div().h(rems(20. / 16.)),
        }
        .flex()
        .items_center()
        .text_color(theme::faint());
        match marker {
            Marker::Task(checked) => {
                let is_document = matches!(self.surface, Surface::Document { .. });
                let icon = svg()
                    .path(if checked {
                        "square-check.svg"
                    } else {
                        "checkbox.svg"
                    })
                    .text_color(if checked {
                        theme::text()
                    } else {
                        theme::faint()
                    });
                let icon = match &self.surface {
                    Surface::Document { look, .. } => icon.size(px(look.base)),
                    Surface::Compact { .. } => icon.size(rems(13. / 16.)),
                };
                base.when(cfg!(test), move |el| {
                    el.debug_selector(move || match (is_document, checked) {
                        (true, true) => "FILE_MD_TASK_DONE".into(),
                        (true, false) => "FILE_MD_TASK".into(),
                        (false, true) => "COMPACT_MD_TASK_DONE".into(),
                        (false, false) => "COMPACT_MD_TASK".into(),
                    })
                })
                .child(icon)
                .into_any_element()
            }
            Marker::Number(number) => base.child(format!("{number}.")).into_any_element(),
            Marker::Bullet => base.child("•").into_any_element(),
        }
    }

    fn content(&self, row: &Row) -> AnyElement {
        match &row.kind {
            RowKind::Heading(level, text) => self.heading(*level, text),
            RowKind::Paragraph(text) => self.paragraph(text),
            RowKind::Image(image) => self.image(image, format!("md-{}-image", self.index)),
            RowKind::CodeHeader { code } => self.code_header(code),
            RowKind::Code { code, line, bytes } => self.code_line(code, *line, bytes.clone()),
            RowKind::TableRow {
                alignments,
                cells,
                head,
                last,
            } => self.table_row(alignments, cells, *head, *last),
            RowKind::Rule => self.rule(),
            RowKind::Html(line) => self.html(line),
        }
    }

    fn heading(&self, level: u8, text: &Inline) -> AnyElement {
        let body = self.inline_line(text, "heading", theme::text(), FontWeight::SEMIBOLD);
        match &self.surface {
            Surface::Document { look, .. } => {
                let scale = match level {
                    1 => 1.54,
                    2 => 1.23,
                    3 => 1.08,
                    _ => 1.,
                };
                div()
                    .pb(px(if level == 1 { 8. } else { 6. }))
                    .when(level == 1, |el| {
                        el.mb(px(4.)).border_b_1().border_color(theme::border())
                    })
                    .text_size(px((look.base * scale).round()))
                    .line_height(px((look.base * scale * 1.3).round()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .map(|el| {
                        #[cfg(test)]
                        let el = crate::theme_snapshot::record_text("FILE_MD_HEADING", el);
                        el
                    })
                    .child(body)
                    .into_any_element()
            }
            Surface::Compact { .. } => div()
                .text_size(if level == 1 {
                    theme::text_title()
                } else {
                    theme::text_body()
                })
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if level >= 3 {
                    theme::muted()
                } else {
                    theme::text()
                })
                .child(body)
                .into_any_element(),
        }
    }

    fn paragraph(&self, text: &Inline) -> AnyElement {
        let body = self.inline_line(text, "para", theme::muted(), FontWeight::NORMAL);
        match &self.surface {
            Surface::Document { .. } => div()
                .text_color(theme::muted())
                .map(|el| {
                    #[cfg(test)]
                    let el = crate::theme_snapshot::record_text("FILE_MD_PARAGRAPH", el);
                    el
                })
                .child(body)
                .into_any_element(),
            Surface::Compact { .. } => div()
                .line_height(rems(20. / 16.))
                .child(body)
                .into_any_element(),
        }
    }

    fn rule(&self) -> AnyElement {
        match &self.surface {
            Surface::Document { .. } => div()
                .mt(px(4.))
                .mb(px(4.))
                .h(px(1.))
                .w_full()
                .bg(theme::border())
                .into_any_element(),
            Surface::Compact { .. } => div()
                .h(rems(1. / 16.))
                .w_full()
                .bg(theme::border())
                .into_any_element(),
        }
    }

    fn html(&self, line: &str) -> AnyElement {
        match &self.surface {
            Surface::Document { look, .. } => {
                let runs = vec![Self::run(line.len(), look.ui.clone(), theme::muted())];
                div()
                    .when(cfg!(test), |el| el.debug_selector(|| "FILE_MD_HTML".into()))
                    .text_color(theme::muted())
                    .child(self.document_selectable(line.to_owned(), runs, &[]))
                    .into_any_element()
            }
            Surface::Compact { doc_id, .. } => {
                div()
                    .line_height(rems(20. / 16.))
                    .child(self.compact_plain_line(
                        format!("{doc_id}-{}-html", self.index),
                        line.to_owned(),
                    ))
                    .into_any_element()
            }
        }
    }

    fn code_states(&self) -> Option<&CodeBlockStates> {
        match &self.surface {
            Surface::Document { code_states, .. } => Some(code_states),
            Surface::Compact { view, cx, .. } => cx
                .try_global::<CompactCodeStates>()
                .and_then(|states| states.view(*view)),
        }
    }

    fn code_header(&self, code: &Arc<CodeBlock>) -> AnyElement {
        let states = self.code_states();
        let hovered = states.is_some_and(|states| states.is_hovered(code));
        let unwrapped = states.is_some_and(|states| states.is_unwrapped(code));
        let copied = states.is_some_and(|states| states.is_copied(code));
        let wrap_on = !unwrapped;
        let show_wrap = hovered;
        let show_copy = hovered || copied;
        let row = self.index;
        let language = code.language.clone();

        match &self.surface {
            Surface::Document { look, view, .. } => {
                let scale = look.base / 13.;
                let bar_h = px((28. * scale).round());
                let lang_size = px((11. * scale).round());
                let icon_size = px((14. * scale).round());
                let btn_size = px((22. * scale).round());
                let btn_radius = px((4. * scale).round());
                let pad_l = px((12. * scale).round());
                let pad_r = px((4. * scale).round());
                let gap = px((2. * scale).round());
                let copy_gap = px((4. * scale).round());
                let copy_px = px((4. * scale).round());

                let hover_view = view.clone();
                let hover_code = Arc::clone(code);
                let wheel_view = view.clone();
                let wheel_code = Arc::clone(code);
                let wrap_view = view.clone();
                let wrap_code = Arc::clone(code);
                let copy_view = view.clone();
                let copy_code = Arc::clone(code);
                let line_height = px((f32::from(look.code_size()) * 1.6).round());
                let wrap_group: SharedString = format!("file-md-code-wrap-g-{row}").into();
                let copy_group: SharedString = format!("file-md-code-copy-g-{row}").into();

                let wrap_btn = show_wrap.then(|| {
                    let button = div()
                        .id(SharedString::from(format!("file-md-code-wrap-{row}")))
                        .group(wrap_group.clone())
                        .when(cfg!(test), move |el| {
                            el.debug_selector(move || format!("FILE_MD_CODE_WRAP {row}"))
                        })
                        .size(btn_size)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(btn_radius)
                        .cursor_pointer()
                        .hover(|el| el.bg(theme::raised()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let _ = wrap_view
                                .update(cx, |view, cx| view.toggle_code_wrap(&wrap_code, cx));
                        })
                        .child(
                            svg()
                                .flex_none()
                                .path("text-wrap.svg")
                                .size(icon_size)
                                .text_color(if wrap_on {
                                    theme::accent()
                                } else {
                                    theme::muted()
                                })
                                .when(!wrap_on, |icon| {
                                    icon.group_hover(wrap_group, |icon| {
                                        icon.text_color(theme::text())
                                    })
                                }),
                        );
                    Tooltip::new(
                        SharedString::from(format!("file-md-code-wrap-tip-{row}")),
                        crate::i18n::text("ui.wrap_lines"),
                        button,
                    )
                });

                let copy_btn = show_copy.then(|| {
                    let button = div()
                        .id(SharedString::from(format!("file-md-code-copy-{row}")))
                        .group(copy_group.clone())
                        .when(cfg!(test), move |el| {
                            el.debug_selector(move || format!("FILE_MD_CODE_COPY {row}"))
                        })
                        .h(btn_size)
                        .when(!copied, |el| el.w(btn_size))
                        .when(copied, |el| el.px(copy_px).gap(copy_gap))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(btn_radius)
                        .cursor_pointer()
                        .hover(|el| el.bg(theme::raised()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let _ = copy_view
                                .update(cx, |view, cx| view.copy_code_block(&copy_code, cx));
                        })
                        .child(
                            svg()
                                .flex_none()
                                .path(if copied { "check.svg" } else { "copy.svg" })
                                .size(icon_size)
                                .text_color(if copied {
                                    theme::accent()
                                } else {
                                    theme::muted()
                                })
                                .when(!copied, |icon| {
                                    icon.group_hover(copy_group, |icon| {
                                        icon.text_color(theme::text())
                                    })
                                }),
                        )
                        .when(copied, |el| {
                            el.child(
                                div()
                                    .when(cfg!(test), move |el| {
                                        el.debug_selector(move || {
                                            format!("FILE_MD_CODE_COPIED {row}")
                                        })
                                    })
                                    .font(look.ui.clone())
                                    .text_size(lang_size)
                                    .line_height(lang_size)
                                    .text_color(theme::accent())
                                    .child(crate::i18n::text("ui.copied")),
                            )
                        });
                    Tooltip::new(
                        SharedString::from(format!("file-md-code-copy-tip-{row}")),
                        if copied {
                            crate::i18n::text("ui.copied")
                        } else {
                            crate::i18n::text("ui.copy_code")
                        },
                        button,
                    )
                });

                div()
                    .id(SharedString::from(format!("file-md-code-hdr-{row}")))
                    .when(cfg!(test), move |el| {
                        el.debug_selector(move || format!("FILE_MD_CODE_HEADER {row}"))
                    })
                    .w_full()
                    .min_w(px(0.))
                    .h(bar_h)
                    .pl(pad_l)
                    .pr(pad_r)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px((8. * scale).round()))
                    .bg(theme::panel())
                    .border_1()
                    .border_color(theme::border())
                    .rounded_t(px(6.))
                    .on_hover(move |hovered, _, cx| {
                        let _ = hover_view.update(cx, |view, cx| {
                            view.set_code_row_hovered(&hover_code, row, *hovered, cx)
                        });
                    })
                    .when(unwrapped, |el| {
                        el.on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
                            let mut delta = event.delta.pixel_delta(line_height);
                            if event.modifiers.shift && delta.x == px(0.) {
                                delta.x = delta.y;
                                delta.y = px(0.);
                            }
                            if delta.x.abs() > px(0.) && delta.x.abs() >= delta.y.abs() {
                                let _ = wheel_view.update(cx, |view, cx| {
                                    if view.max_code_scroll_x(&wheel_code) > 0. {
                                        cx.stop_propagation();
                                        view.scroll_code_x(&wheel_code, -f32::from(delta.x), cx);
                                    }
                                });
                            }
                        })
                    })
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .truncate()
                            .font(look.mono.clone())
                            .text_size(lang_size)
                            .line_height(lang_size)
                            .text_color(theme::faint())
                            .when_some(language, |el, lang| {
                                el.when(cfg!(test), move |el| {
                                    el.debug_selector(move || format!("FILE_MD_CODE_LANG {row}"))
                                })
                                .child(lang)
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(gap)
                            .children(wrap_btn)
                            .children(copy_btn),
                    )
                    .into_any_element()
            }
            Surface::Compact {
                doc_id,
                view,
                on_height_change,
                ..
            } => {
                let doc = (*doc_id).to_owned();
                let view = *view;
                let hover_code = Arc::clone(code);
                let wheel_code = Arc::clone(code);
                let wrap_code = Arc::clone(code);
                let wrap_height_change = on_height_change.cloned();
                let copy_code = Arc::clone(code);
                let wrap_group: SharedString = format!("{doc}-{row}-code-wrap-g").into();
                let copy_group: SharedString = format!("{doc}-{row}-code-copy-g").into();

                let wrap_btn = show_wrap.then(|| {
                    let debug_doc = doc.clone();
                    let button = div()
                        .id(SharedString::from(format!("{doc}-{row}-code-wrap")))
                        .group(wrap_group.clone())
                        .when(cfg!(test), move |el| {
                            let debug_doc = debug_doc.clone();
                            el.debug_selector(move || {
                                format!("COMPACT_MD_CODE_WRAP {debug_doc} {row}")
                            })
                        })
                        .size(rems(22. / 16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(rems(4. / 16.))
                        .cursor_pointer()
                        .hover(|el| el.bg(theme::raised()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            cx.default_global::<CompactCodeStates>()
                                .view_mut(view)
                                .toggle_wrap(&wrap_code);
                            if let Some(on_height_change) = &wrap_height_change {
                                on_height_change(cx);
                            }
                            cx.notify(view);
                        })
                        .child(
                            svg()
                                .flex_none()
                                .path("text-wrap.svg")
                                .size(rems(14. / 16.))
                                .text_color(if wrap_on {
                                    theme::accent()
                                } else {
                                    theme::muted()
                                })
                                .when(!wrap_on, |icon| {
                                    icon.group_hover(wrap_group, |icon| {
                                        icon.text_color(theme::text())
                                    })
                                }),
                        );
                    Tooltip::new(
                        SharedString::from(format!("{doc}-{row}-code-wrap-tip")),
                        crate::i18n::text("ui.wrap_lines"),
                        button,
                    )
                });

                let copy_btn = show_copy.then(|| {
                    let debug_doc = doc.clone();
                    let copied_doc = doc.clone();
                    let button = div()
                        .id(SharedString::from(format!("{doc}-{row}-code-copy")))
                        .group(copy_group.clone())
                        .when(cfg!(test), move |el| {
                            let debug_doc = debug_doc.clone();
                            el.debug_selector(move || {
                                format!("COMPACT_MD_CODE_COPY {debug_doc} {row}")
                            })
                        })
                        .h(rems(22. / 16.))
                        .when(!copied, |el| el.w(rems(22. / 16.)))
                        .when(copied, |el| el.px(rems(4. / 16.)).gap(rems(4. / 16.)))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(rems(4. / 16.))
                        .cursor_pointer()
                        .hover(|el| el.bg(theme::raised()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                copy_code.copy_text().to_owned(),
                            ));
                            let code_id = CodeBlockId::of(&copy_code);
                            let generation = cx
                                .default_global::<CompactCodeStates>()
                                .view_mut(view)
                                .begin_copied(&copy_code);
                            let async_cx = cx.to_async();
                            let task = cx.foreground_executor().spawn(async move {
                                async_cx.background_executor().timer(COPIED_DURATION).await;
                                async_cx.update(|cx| {
                                    let cleared = if cx.has_global::<CompactCodeStates>() {
                                        let states = cx.global_mut::<CompactCodeStates>();
                                        let changed = states
                                            .view_mut(view)
                                            .finish_copied(code_id, generation);
                                        states.prune_empty_views();
                                        changed
                                    } else {
                                        false
                                    };
                                    if cleared {
                                        cx.notify(view);
                                    }
                                });
                            });
                            cx.default_global::<CompactCodeStates>()
                                .view_mut(view)
                                .set_copied_timer(code_id, generation, task);
                            cx.notify(view);
                        })
                        .child(
                            svg()
                                .flex_none()
                                .path(if copied { "check.svg" } else { "copy.svg" })
                                .size(rems(14. / 16.))
                                .text_color(if copied {
                                    theme::accent()
                                } else {
                                    theme::muted()
                                })
                                .when(!copied, |icon| {
                                    icon.group_hover(copy_group, |icon| {
                                        icon.text_color(theme::text())
                                    })
                                }),
                        )
                        .when(copied, |el| {
                            el.child(
                                div()
                                    .when(cfg!(test), move |el| {
                                        let copied_doc = copied_doc.clone();
                                        el.debug_selector(move || {
                                            format!("COMPACT_MD_CODE_COPIED {copied_doc} {row}")
                                        })
                                    })
                                    .text_size(theme::text_meta())
                                    .line_height(theme::text_meta())
                                    .text_color(theme::accent())
                                    .child(crate::i18n::text("ui.copied")),
                            )
                        });
                    Tooltip::new(
                        SharedString::from(format!("{doc}-{row}-code-copy-tip")),
                        if copied {
                            crate::i18n::text("ui.copied")
                        } else {
                            crate::i18n::text("ui.copy_code")
                        },
                        button,
                    )
                });

                let hdr_doc = doc.clone();
                let lang_doc = doc.clone();
                div()
                    .id(SharedString::from(format!("{doc}-{row}-code-hdr")))
                    .when(cfg!(test), move |el| {
                        let hdr_doc = hdr_doc.clone();
                        el.debug_selector(move || format!("COMPACT_MD_CODE_HEADER {hdr_doc} {row}"))
                    })
                    .w_full()
                    .min_w(px(0.))
                    .h(rems(28. / 16.))
                    .pl(rems(12. / 16.))
                    .pr(rems(4. / 16.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(rems(8. / 16.))
                    .bg(theme::bg())
                    .border_1()
                    .border_color(theme::border())
                    .rounded_t_md()
                    .on_hover(move |hovered, _, cx| {
                        let states = cx.default_global::<CompactCodeStates>();
                        let changed =
                            states
                                .view_mut(view)
                                .set_row_hovered(&hover_code, row, *hovered);
                        states.prune_empty_views();
                        if changed {
                            cx.notify(view);
                        }
                    })
                    .when(unwrapped, |el| {
                        el.on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                            let line_height = rems(19. / 16.).to_pixels(window.rem_size());
                            let mut delta = event.delta.pixel_delta(line_height);
                            if event.modifiers.shift && delta.x == px(0.) {
                                delta.x = delta.y;
                                delta.y = px(0.);
                            }
                            if delta.x.abs() > px(0.) && delta.x.abs() >= delta.y.abs() {
                                let (can_scroll, moved) = {
                                    let states =
                                        cx.default_global::<CompactCodeStates>().view_mut(view);
                                    if states.max_scroll_x(&wheel_code) > 0. {
                                        (true, states.scroll_by(&wheel_code, -f32::from(delta.x)))
                                    } else {
                                        (false, false)
                                    }
                                };
                                if can_scroll {
                                    cx.stop_propagation();
                                    if moved {
                                        cx.notify(view);
                                    }
                                }
                            }
                        })
                    })
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .truncate()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_meta())
                            .line_height(theme::text_meta())
                            .text_color(theme::faint())
                            .when_some(language, |el, lang| {
                                el.when(cfg!(test), move |el| {
                                    let lang_doc = lang_doc.clone();
                                    el.debug_selector(move || {
                                        format!("COMPACT_MD_CODE_LANG {lang_doc} {row}")
                                    })
                                })
                                .child(lang)
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(rems(2. / 16.))
                            .children(wrap_btn)
                            .children(copy_btn),
                    )
                    .into_any_element()
            }
        }
    }

    fn code_line(&self, code: &Arc<CodeBlock>, line: usize, bytes: Range<usize>) -> AnyElement {
        let line_len = code.document.line(line).len();
        let (first, last) = (
            line == 0 && bytes.start == 0,
            line + 1 == code.document.line_count() && bytes.end == line_len,
        );
        let states = self.code_states();
        let unwrapped = states.is_some_and(|states| states.is_unwrapped(code));
        let scroll_x = states.map_or(0., |states| states.scroll_x(code));
        let has_overflow = unwrapped && states.is_some_and(|states| states.has_overflow(code));
        let dragging_scrollbar =
            unwrapped && states.is_some_and(|states| states.is_dragging_scrollbar(code));
        let row = self.index;
        let max_row_columns = code.max_row_columns;

        if let Surface::Compact { view, cx, .. } = &self.surface {
            code.start_async_highlight(*view, cx);
        }

        let (text, spans) = code.fragment(line, bytes);
        let strip = div()
            .border_l_1()
            .border_r_1()
            .border_color(theme::border());
        match &self.surface {
            Surface::Document { look, view, .. } => {
                let size = look.code_size();
                let line_height = px((f32::from(size) * 1.6).round());
                #[cfg(test)]
                super::view::DRAWN_CODE_BYTES.set(super::view::DRAWN_CODE_BYTES.get() + text.len());
                let runs = spans
                    .into_iter()
                    .map(|(len, capture)| {
                        let color = capture
                            .and_then(|capture| look.syntax.color(capture))
                            .unwrap_or(theme::text());
                        Self::run(len, look.mono.clone(), color)
                    })
                    .collect();
                let selectable = self.document_selectable_with_wrap(text, runs, &[], !unwrapped);
                let hover_view = view.clone();
                let hover_code = Arc::clone(code);
                let base = strip
                    .id(SharedString::from(format!("file-md-code-line-{row}")))
                    .when(cfg!(test) && first, |el| {
                        el.debug_selector(|| "FILE_MD_CODE".into())
                    })
                    .w_full()
                    .min_w(px(0.))
                    .bg(theme::panel())
                    .when(first, |el| el.pt(px(10.)))
                    .when(last, |el| {
                        el.border_b_1()
                            .rounded_b(px(6.))
                            .when(!has_overflow, |el| el.pb(px(10.)))
                    })
                    .text_size(size)
                    .line_height(line_height)
                    .on_hover(move |hovered, _, cx| {
                        let _ = hover_view.update(cx, |view, cx| {
                            view.set_code_row_hovered(&hover_code, row, *hovered, cx)
                        });
                    });
                if !unwrapped {
                    return base.px(px(12.)).child(selectable).into_any_element();
                }

                let wheel_view = view.clone();
                let wheel_code = Arc::clone(code);
                let measure_view = view.clone();
                let measure_code = Arc::clone(code);
                let (prev_viewport, prev_content) =
                    states.map_or((0., 0.), |states| states.metrics(code));
                let mono = look.mono.clone();
                let measure = canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let viewport_width = f32::from(bounds.size.width);
                        if viewport_width <= 0. {
                            return;
                        }
                        let font_id = window.text_system().resolve_font(&mono);
                        let char_width = f32::from(
                            window
                                .text_system()
                                .em_advance(font_id, size)
                                .unwrap_or(size * 0.6),
                        );
                        let content_width = 24. + max_row_columns as f32 * char_width;
                        if (viewport_width - prev_viewport).abs() <= 0.5
                            && (content_width - prev_content).abs() <= 0.5
                        {
                            return;
                        }
                        let defer_view = measure_view.clone();
                        let defer_code = Arc::clone(&measure_code);
                        window.defer(cx, move |_, cx| {
                            let _ = defer_view.update(cx, |view, cx| {
                                view.update_code_metrics(
                                    &defer_code,
                                    viewport_width,
                                    content_width,
                                    cx,
                                )
                            });
                        });
                    },
                )
                .absolute()
                .inset_0();

                let scrollbar = (last && has_overflow).then(|| {
                    let track_width = (prev_viewport - 24.).max(1.);
                    let thumb = states
                        .and_then(|states| {
                            states.scrollbar_thumb(code, track_width, MIN_SCROLLBAR_THUMB)
                        })
                        .map(|(left, width)| (left / track_width, width / track_width));
                    let track_bounds = Rc::new(StdCell::new(Bounds::<Pixels>::default()));
                    let paint_track = Rc::clone(&track_bounds);
                    let down_track = Rc::clone(&track_bounds);
                    let drag_view = view.clone();
                    let drag_code = Arc::clone(code);
                    div().w_full().px(px(12.)).pt(px(4.)).pb(px(4.)).child(
                        div()
                            .id(SharedString::from(format!("file-md-code-sb-{row}")))
                            .when(cfg!(test), move |el| {
                                el.debug_selector(move || format!("FILE_MD_CODE_SCROLLBAR {row}"))
                            })
                            .relative()
                            .w_full()
                            .h(px(8.))
                            .flex()
                            .items_center()
                            .cursor(CursorStyle::Arrow)
                            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                                cx.stop_propagation();
                                let track = down_track.get();
                                let track_width = f32::from(track.size.width);
                                if track_width > 0. {
                                    let _ = drag_view.update(cx, |view, cx| {
                                        view.begin_code_scrollbar_drag(
                                            &drag_code,
                                            f32::from(event.position.x),
                                            f32::from(track.left()),
                                            track_width,
                                            MIN_SCROLLBAR_THUMB,
                                            cx,
                                        )
                                    });
                                }
                            })
                            .child(
                                canvas(
                                    move |bounds, _, _| paint_track.set(bounds),
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            )
                            .children(thumb.map(|(left, width)| {
                                div()
                                    .when(cfg!(test), move |el| {
                                        el.debug_selector(move || {
                                            format!("FILE_MD_CODE_SCROLLBAR_THUMB {row}")
                                        })
                                    })
                                    .absolute()
                                    .left(relative(left))
                                    .w(relative(width))
                                    .h(px(4.))
                                    .rounded_full()
                                    .bg(if dragging_scrollbar {
                                        theme::with_alpha(theme::muted(), 0.8)
                                    } else {
                                        theme::with_alpha(theme::muted(), 0.55)
                                    })
                                    .hover(|el| el.bg(theme::with_alpha(theme::muted(), 0.8)))
                            })),
                    )
                });

                base.flex()
                    .flex_col()
                    .on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
                        let mut delta = event.delta.pixel_delta(line_height);
                        if event.modifiers.shift && delta.x == px(0.) {
                            delta.x = delta.y;
                            delta.y = px(0.);
                        }
                        if delta.x.abs() > px(0.) && delta.x.abs() >= delta.y.abs() {
                            let _ = wheel_view.update(cx, |view, cx| {
                                if view.max_code_scroll_x(&wheel_code) > 0. {
                                    cx.stop_propagation();
                                    view.scroll_code_x(&wheel_code, -f32::from(delta.x), cx);
                                }
                            });
                        }
                    })
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .flex()
                            .child(
                                div()
                                    .flex_none()
                                    .min_w_full()
                                    .whitespace_nowrap()
                                    .px(px(12.))
                                    .ml(px(-scroll_x))
                                    .child(selectable),
                            )
                            .child(measure),
                    )
                    .children(scrollbar)
                    .into_any_element()
            }
            Surface::Compact {
                doc_id,
                view,
                syntax,
                selection_color,
                on_height_change,
                ..
            } => {
                let doc = (*doc_id).to_owned();
                let view = *view;
                let highlights = compact_code_highlights(
                    &spans,
                    *syntax,
                    self.selected.clone(),
                    *selection_color,
                );
                let line_el = self.compact_prepared_line_with_wrap(
                    format!("{doc}-{row}-code"),
                    text,
                    highlights,
                    Vec::new(),
                    Vec::new(),
                    !unwrapped,
                );
                let hover_code = Arc::clone(code);
                let base = strip
                    .id(SharedString::from(format!("{doc}-{row}-code-line")))
                    .when(cfg!(test) && first, |el| {
                        el.debug_selector(|| "COMPACT_MD_CODE".into())
                    })
                    .w_full()
                    .min_w(px(0.))
                    .bg(theme::bg())
                    .text_color(theme::text())
                    .when(first, |el| el.pt_2())
                    .when(last, |el| {
                        el.border_b_1()
                            .rounded_b_md()
                            .when(!has_overflow, |el| el.pb_2())
                    })
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_ui())
                    .line_height(rems(19. / 16.))
                    .on_hover(move |hovered, _, cx| {
                        let states = cx.default_global::<CompactCodeStates>();
                        let changed =
                            states
                                .view_mut(view)
                                .set_row_hovered(&hover_code, row, *hovered);
                        states.prune_empty_views();
                        if changed {
                            cx.notify(view);
                        }
                    });
                if !unwrapped {
                    return base.px_3().child(line_el).into_any_element();
                }

                let wheel_code = Arc::clone(code);
                let measure_code = Arc::clone(code);
                let measure_height_change = on_height_change.cloned();
                let (prev_viewport, prev_content) =
                    states.map_or((0., 0.), |states| states.metrics(code));
                let measure = canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let viewport_width = f32::from(bounds.size.width);
                        if viewport_width <= 0. {
                            return;
                        }
                        let rem = window.rem_size();
                        let font_size = theme::text_ui().to_pixels(rem);
                        let pad_x = f32::from(rems(12. / 16.).to_pixels(rem));
                        let mono = gpui::font(theme::UI_MONOSPACE_FONT);
                        let font_id = window.text_system().resolve_font(&mono);
                        let char_width = f32::from(
                            window
                                .text_system()
                                .em_advance(font_id, font_size)
                                .unwrap_or(font_size * 0.6),
                        );
                        let content_width = pad_x * 2. + max_row_columns as f32 * char_width;
                        if (viewport_width - prev_viewport).abs() <= 0.5
                            && (content_width - prev_content).abs() <= 0.5
                        {
                            return;
                        }
                        let defer_code = Arc::clone(&measure_code);
                        let defer_height_change = measure_height_change.clone();
                        window.defer(cx, move |_, cx| {
                            let change = cx
                                .default_global::<CompactCodeStates>()
                                .view_mut(view)
                                .update_metrics(&defer_code, viewport_width, content_width);
                            if change.overflow_toggled
                                && let Some(on_height_change) = &defer_height_change
                            {
                                on_height_change(cx);
                            }
                            if change.changed {
                                cx.notify(view);
                            }
                        });
                    },
                )
                .absolute()
                .inset_0();

                let scrollbar = (last && has_overflow).then(|| {
                    let track_width = (prev_viewport - 24.).max(1.);
                    let thumb = states
                        .and_then(|states| {
                            states.scrollbar_thumb(code, track_width, MIN_SCROLLBAR_THUMB)
                        })
                        .map(|(left, width)| (left / track_width, width / track_width));
                    let track_bounds = Rc::new(StdCell::new(Bounds::<Pixels>::default()));
                    let paint_track = Rc::clone(&track_bounds);
                    let down_track = Rc::clone(&track_bounds);
                    let drag_code = Arc::clone(code);
                    let sb_doc = doc.clone();
                    let thumb_doc = doc.clone();
                    div().w_full().px_3().py_1().child(
                        div()
                            .id(SharedString::from(format!("{doc}-{row}-code-sb")))
                            .when(cfg!(test), move |el| {
                                let sb_doc = sb_doc.clone();
                                el.debug_selector(move || {
                                    format!("COMPACT_MD_CODE_SCROLLBAR {sb_doc} {row}")
                                })
                            })
                            .relative()
                            .w_full()
                            .h(rems(8. / 16.))
                            .flex()
                            .items_center()
                            .cursor(CursorStyle::Arrow)
                            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                                cx.stop_propagation();
                                let track = down_track.get();
                                let track_width = f32::from(track.size.width);
                                if track_width > 0. {
                                    if cx
                                        .default_global::<CompactCodeStates>()
                                        .view_mut(view)
                                        .begin_scrollbar_drag(
                                            &drag_code,
                                            f32::from(event.position.x),
                                            f32::from(track.left()),
                                            track_width,
                                            MIN_SCROLLBAR_THUMB,
                                        )
                                    {
                                        cx.notify(view);
                                    }
                                }
                            })
                            .child(
                                canvas(
                                    move |bounds, _, _| paint_track.set(bounds),
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            )
                            .children(thumb.map(|(left, width)| {
                                div()
                                    .when(cfg!(test), move |el| {
                                        let thumb_doc = thumb_doc.clone();
                                        el.debug_selector(move || {
                                            format!(
                                                "COMPACT_MD_CODE_SCROLLBAR_THUMB {thumb_doc} {row}"
                                            )
                                        })
                                    })
                                    .absolute()
                                    .left(relative(left))
                                    .w(relative(width))
                                    .h(rems(4. / 16.))
                                    .rounded_full()
                                    .bg(if dragging_scrollbar {
                                        theme::with_alpha(theme::muted(), 0.8)
                                    } else {
                                        theme::with_alpha(theme::muted(), 0.55)
                                    })
                                    .hover(|el| el.bg(theme::with_alpha(theme::muted(), 0.8)))
                            })),
                    )
                });

                base.flex()
                    .flex_col()
                    .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                        let line_height = rems(19. / 16.).to_pixels(window.rem_size());
                        let mut delta = event.delta.pixel_delta(line_height);
                        if event.modifiers.shift && delta.x == px(0.) {
                            delta.x = delta.y;
                            delta.y = px(0.);
                        }
                        if delta.x.abs() > px(0.) && delta.x.abs() >= delta.y.abs() {
                            let (can_scroll, moved) = {
                                let states =
                                    cx.default_global::<CompactCodeStates>().view_mut(view);
                                if states.max_scroll_x(&wheel_code) > 0. {
                                    (true, states.scroll_by(&wheel_code, -f32::from(delta.x)))
                                } else {
                                    (false, false)
                                }
                            };
                            if can_scroll {
                                cx.stop_propagation();
                                if moved {
                                    cx.notify(view);
                                }
                            }
                        }
                    })
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .flex()
                            .child(
                                div()
                                    .flex_none()
                                    .min_w_full()
                                    .whitespace_nowrap()
                                    .px_3()
                                    .ml(px(-scroll_x))
                                    .child(line_el),
                            )
                            .child(measure),
                    )
                    .children(scrollbar)
                    .into_any_element()
            }
        }
    }

    fn table_row(
        &self,
        alignments: &[Alignment],
        cells: &[Cell],
        head: bool,
        last: bool,
    ) -> AnyElement {
        let columns = alignments.len().max(cells.len()).max(1);
        let len = cells
            .iter()
            .map(|cell| cell_text(cell).len() + 1)
            .sum::<usize>()
            .saturating_sub(1);
        let row_index = self.index;
        let is_document = matches!(self.surface, Surface::Document { .. });
        let row = div()
            .when(cfg!(test), |el| match &self.surface {
                Surface::Document { .. } => {
                    el.when(head, |el| el.debug_selector(|| "FILE_MD_TABLE".into()))
                }
                Surface::Compact { doc_id, .. } => {
                    let doc_id = (*doc_id).to_owned();
                    el.debug_selector(move || format!("COMPACT_MD_TABLE_ROW {doc_id} {row_index}"))
                }
            })
            .w_full()
            .min_w(px(0.))
            .flex()
            .border_l_1()
            .border_r_1()
            .border_t_1()
            .border_color(theme::border())
            .when(head, |el| {
                el.rounded_t(px(6.)).bg(if is_document {
                    theme::panel()
                } else {
                    theme::raised()
                })
            })
            .when(last, |el| el.border_b_1().rounded_b(px(6.)))
            .children((0..columns).map(|column| {
                let cell = cells.get(column).cloned().unwrap_or_default();
                let align = alignments.get(column).copied().unwrap_or(Alignment::None);
                let column_el = div()
                    .when(cfg!(test), move |el| {
                        el.debug_selector(move || {
                            if is_document {
                                format!("FILE_MD_CELL {row_index}-{column}")
                            } else {
                                format!("COMPACT_MD_CELL {row_index}-{column}")
                            }
                        })
                    })
                    .flex_1()
                    .min_w(px(0.))
                    .when(is_document, |el| el.px(px(10.)).py(px(6.)))
                    .when(!is_document, |el| {
                        el.px_2()
                            .py_1()
                            .text_size(theme::text_ui())
                            .when(head, |el| el.font_weight(FontWeight::SEMIBOLD))
                            .text_color(if head { theme::text() } else { theme::muted() })
                    })
                    .when(column > 0, |el| {
                        el.border_l_1().border_color(theme::border())
                    })
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(4.))
                    .when(align == Alignment::Center, |el| el.justify_center())
                    .when(align == Alignment::Right, |el| el.justify_end());
                column_el.children(cell.iter().enumerate().map(|(piece_index, piece)| {
                    let body = match piece {
                        Piece::Text(text) => self.inline_cell(text, head, column, piece_index),
                        Piece::Image(image) => self.image(
                            image,
                            format!("md-{}-{column}-{piece_index}-image", self.index),
                        ),
                    };
                    div()
                        .when(cfg!(test), move |el| {
                            el.debug_selector(move || {
                                if is_document {
                                    format!("FILE_MD_CELL_PIECE {row_index}-{column}-{piece_index}")
                                } else {
                                    format!(
                                        "COMPACT_MD_CELL_PIECE {row_index}-{column}-{piece_index}"
                                    )
                                }
                            })
                        })
                        // GPUI text gives its unwrapped width as its minimum.
                        .min_w(px(0.))
                        .max_w_full()
                        .child(body)
                }))
            }));
        self.whole_row(row, len).into_any_element()
    }

    /// An image, clickable when it sits in a link; `id` is unique in the
    /// row, so two linked images in one table row each take their own click.
    fn image(&self, image: &ImageRef, id: String) -> AnyElement {
        let Surface::Document {
            look,
            images,
            from,
            open_link,
            view,
            ..
        } = &self.surface
        else {
            // Compact parsing turns images into alt text, so this arm is a fallback only.
            let alt = if image.alt.is_empty() {
                image.path.clone()
            } else {
                image.alt.clone()
            };
            return div()
                .line_height(rems(20. / 16.))
                .text_color(theme::faint())
                .child(alt)
                .into_any_element();
        };
        let body = match images.get(&image.path) {
            Some(decoded) => img(ImageSource::Image(decoded.clone()))
                .max_w(relative(1.))
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "FILE_MD_IMAGE".into())
                })
                .into_any_element(),
            None => div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "FILE_MD_IMAGE_ALT".into())
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
        let target = image.link.as_deref().map(|href| route(href, from));
        div()
            .id(SharedString::from(id))
            .when_some(image.link.clone().filter(|_| cfg!(test)), |el, link| {
                el.debug_selector(move || format!("FILE_MD_IMAGE_LINK {link}"))
            })
            .child(body)
            .when_some(target.zip(open_link.clone()), |el, (target, open)| {
                let view = view.clone();
                el.cursor_pointer().on_click(move |_, window, cx| {
                    if document_allows_click(&view, cx) {
                        open(target.clone(), window, cx)
                    }
                })
            })
            .into_any_element()
    }

    /// A row that selects whole, as a table row does.
    fn whole_row(&self, mut element: gpui::Div, len: usize) -> gpui::Div {
        let row = self.index;
        match &self.surface {
            Surface::Document { view, .. } => {
                let (down_view, move_view) = (view.clone(), view.clone());
                element
                    .when(!self.selected.is_empty(), |el| {
                        el.bg(theme::with_alpha(theme::accent(), 0.28))
                    })
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        let _ = down_view.update(cx, |view, cx| {
                            let at = Position { row, offset: 0 };
                            view.begin_selection(at, event.modifiers.shift, window, cx)
                        });
                    })
                    .on_mouse_move(move |event, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            let _ = move_view.update(cx, |view, cx| {
                                view.drag_to(Position { row, offset: len }, cx)
                            });
                        }
                    })
            }
            Surface::Compact {
                doc_id,
                selection_color,
                selection_handler,
                ..
            } => {
                if !self.selected.is_empty() {
                    element = element.bg(*selection_color);
                }
                let Some(handler) = (*selection_handler).cloned() else {
                    return element;
                };
                let down_handler = handler.clone();
                let down_doc = (*doc_id).to_owned();
                let move_handler = handler.clone();
                let move_doc = (*doc_id).to_owned();
                let up_handler = handler;
                let up_doc = (*doc_id).to_owned();
                element
                    .cursor(CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        down_handler.handle(
                            FeedSelectionPhase::Begin,
                            &down_doc,
                            Position { row, offset: 0 },
                            window,
                            cx,
                        );
                    })
                    .on_mouse_move(move |event, window, cx| {
                        if event.pressed_button == Some(MouseButton::Left)
                            && compact_press_moved(cx)
                        {
                            move_handler.handle(
                                FeedSelectionPhase::Extend,
                                &move_doc,
                                Position { row, offset: len },
                                window,
                                cx,
                            );
                        }
                    })
                    .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                        let offset = if compact_press_moved(cx) { len } else { 0 };
                        up_handler.handle(
                            FeedSelectionPhase::End,
                            &up_doc,
                            Position { row, offset },
                            window,
                            cx,
                        );
                    })
            }
        }
    }

    fn inline_line(
        &self,
        text: &Inline,
        kind_tag: &str,
        doc_color: Hsla,
        doc_weight: FontWeight,
    ) -> AnyElement {
        match &self.surface {
            Surface::Document { look, .. } => {
                let runs = Self::document_inline_runs(look, text, doc_color, doc_weight);
                self.document_selectable(text.text.clone(), runs, &text.links)
            }
            Surface::Compact {
                doc_id,
                selection_color,
                cx,
                ..
            } => {
                let id = format!("{doc_id}-{}-{kind_tag}", self.index);
                let (ranges, urls) = openable_links(&text.links);
                let hovered_range = cx
                    .try_global::<MarkdownLinkHover>()
                    .and_then(|hover| hover.link_for(&id))
                    .and_then(|index| ranges.get(index).cloned());
                let highlights = compact_highlights(
                    text,
                    hovered_range,
                    self.selected.clone(),
                    *selection_color,
                );
                self.compact_prepared_line(id, text.text.clone(), highlights, ranges, urls)
            }
        }
    }

    fn inline_cell(
        &self,
        text: &Inline,
        head: bool,
        column: usize,
        piece_index: usize,
    ) -> AnyElement {
        match &self.surface {
            Surface::Document {
                look,
                from,
                open_link,
                view,
                ..
            } => {
                let (color, weight) = if head {
                    (theme::text(), FontWeight::SEMIBOLD)
                } else {
                    (theme::muted(), FontWeight::NORMAL)
                };
                let runs = Self::document_inline_runs(look, text, color, weight);
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
                    .map(|(_, href)| route(href, from))
                    .collect::<Vec<_>>();
                let open_link = open_link.clone();
                let view = view.clone();
                let id = format!("md-{}-{column}-{piece_index}", self.index);
                div()
                    .child(
                        InteractiveText::new(SharedString::from(id), styled).on_click(
                            ranges,
                            move |index, window, cx| {
                                if !document_allows_click(&view, cx) {
                                    return;
                                }
                                if let (Some(open), Some(target)) = (&open_link, targets.get(index))
                                {
                                    open(target.clone(), window, cx);
                                }
                            },
                        ),
                    )
                    .into_any_element()
            }
            Surface::Compact {
                doc_id,
                view,
                selection_color,
                cx,
                ..
            } => {
                #[cfg(test)]
                super::feed::LINE_COUNT.set(super::feed::LINE_COUNT.get() + 1);
                let id = format!("{doc_id}-{}-cell-{column}-{piece_index}", self.index);
                let (ranges, urls) = openable_links(&text.links);
                let hovered_range = cx
                    .try_global::<MarkdownLinkHover>()
                    .and_then(|hover| hover.link_for(&id))
                    .and_then(|index| ranges.get(index).cloned());
                let highlights = compact_highlights(text, hovered_range, 0..0, *selection_color);
                let body = if text.text.is_empty() {
                    " ".to_owned()
                } else {
                    text.text.clone()
                };
                let styled = StyledText::new(body).with_highlights(highlights);
                if ranges.is_empty() {
                    return div()
                        .min_w(px(0.))
                        .whitespace_normal()
                        .child(styled)
                        .into_any_element();
                }
                Self::compact_interactive_links(*view, id, styled, ranges, urls, false)
            }
        }
    }

    // ── Document-specific text styling & selection ─────────────────────

    fn run(len: usize, font: Font, color: Hsla) -> TextRun {
        TextRun {
            len,
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    fn document_inline_runs(
        look: &Look,
        text: &Inline,
        color: Hsla,
        weight: FontWeight,
    ) -> Vec<TextRun> {
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

    fn document_selectable(
        &self,
        text: String,
        runs: Vec<TextRun>,
        links: &[(Range<usize>, Arc<str>)],
    ) -> AnyElement {
        self.document_selectable_with_wrap(text, runs, links, true)
    }

    fn document_selectable_with_wrap(
        &self,
        mut text: String,
        runs: Vec<TextRun>,
        links: &[(Range<usize>, Arc<str>)],
        wrap: bool,
    ) -> AnyElement {
        let Surface::Document {
            look,
            from,
            open_link,
            view,
            ..
        } = &self.surface
        else {
            unreachable!();
        };
        let mut runs = with_selection(
            runs,
            self.selected.clone(),
            theme::with_alpha(theme::accent(), 0.28),
        );
        if text.is_empty() {
            // An empty line still takes a line's height.
            text.push(' ');
            runs = vec![Self::run(1, look.ui.clone(), theme::muted())];
        }
        let styled = StyledText::new(text).with_runs(runs);
        let layout = styled.layout().clone();
        let (row, down_view, down_layout) = (self.index, view.clone(), layout.clone());
        let (move_view, move_layout) = (view.clone(), layout);
        let element = div()
            .when(!wrap, |el| el.whitespace_nowrap())
            .when(cfg!(test), move |el| {
                el.debug_selector(move || format!("FILE_MD_TEXT {row}"))
            })
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                let at = text_position(&down_layout, row, event.position);
                let _ = down_view.update(cx, |view, cx| {
                    view.begin_selection(at, event.modifiers.shift, window, cx)
                });
            })
            .on_mouse_move(move |event, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let at = text_position(&move_layout, row, event.position);
                let _ = move_view.update(cx, |view, cx| view.drag_to(at, cx));
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
            .map(|(_, href)| route(href, from))
            .collect::<Vec<_>>();
        let open_link = open_link.clone();
        let click_view = view.clone();
        element
            .child(
                InteractiveText::new(
                    SharedString::from(format!("md-{}-text", self.index)),
                    styled,
                )
                .on_click(ranges, move |index, window, cx| {
                    if !document_allows_click(&click_view, cx) {
                        return;
                    }
                    if let (Some(open), Some(target)) = (&open_link, targets.get(index)) {
                        open(target.clone(), window, cx);
                    }
                }),
            )
            .into_any_element()
    }

    // ── Compact-specific text styling, hover & selection ───────────────

    fn compact_plain_line(&self, id: String, text: String) -> AnyElement {
        let Surface::Compact {
            selection_color, ..
        } = &self.surface
        else {
            unreachable!();
        };
        let highlights = if self.selected.is_empty() {
            Vec::new()
        } else {
            vec![(
                self.selected.clone(),
                HighlightStyle {
                    background_color: Some(*selection_color),
                    ..Default::default()
                },
            )]
        };
        self.compact_prepared_line(id, text, highlights, Vec::new(), Vec::new())
    }

    fn compact_interactive_links(
        view: EntityId,
        id: String,
        styled: StyledText,
        link_ranges: Vec<Range<usize>>,
        urls: Vec<Arc<str>>,
        full_width: bool,
    ) -> AnyElement {
        let hover_id = id.clone();
        let hover_ranges = link_ranges.clone();
        let leave_id = id.clone();
        div()
            .id(SharedString::from(format!("{id}-hover")))
            .when(full_width, |el| el.w_full())
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
            .child(
                InteractiveText::new(SharedString::from(id), styled)
                    .on_click(link_ranges, move |index, _, cx| {
                        if !compact_allows_click(cx) {
                            return;
                        }
                        if let Some(url) = urls.get(index) {
                            cx.open_url(url.trim());
                        }
                    })
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
                    }),
            )
            .into_any_element()
    }

    fn compact_prepared_line(
        &self,
        id: String,
        text: String,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
        link_ranges: Vec<Range<usize>>,
        urls: Vec<Arc<str>>,
    ) -> AnyElement {
        self.compact_prepared_line_with_wrap(id, text, highlights, link_ranges, urls, true)
    }

    fn compact_prepared_line_with_wrap(
        &self,
        id: String,
        mut text: String,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
        link_ranges: Vec<Range<usize>>,
        urls: Vec<Arc<str>>,
        wrap: bool,
    ) -> AnyElement {
        let Surface::Compact {
            doc_id,
            view,
            selection_handler,
            ..
        } = &self.surface
        else {
            unreachable!();
        };
        #[cfg(test)]
        super::feed::LINE_COUNT.set(super::feed::LINE_COUNT.get() + 1);

        if text.is_empty() {
            // An empty line still takes a line's height.
            text.push(' ');
        }
        let styled = StyledText::new(text).with_highlights(highlights);
        let layout = styled.layout().clone();
        let row = self.index;
        let content = if !link_ranges.is_empty() {
            Self::compact_interactive_links(*view, id, styled, link_ranges, urls, true)
        } else {
            styled.into_any_element()
        };

        let debug_doc = (*doc_id).to_owned();
        let mut line = div()
            .when(wrap, |el| el.w_full().min_w(px(0.)).whitespace_normal())
            .when(!wrap, |el| el.whitespace_nowrap())
            .when(cfg!(test), move |el| {
                let doc_id = debug_doc.clone();
                el.debug_selector(move || format!("COMPACT_MD_TEXT {doc_id} {row}"))
            })
            .child(content);

        if let Some(handler) = (*selection_handler).cloned() {
            let down_handler = handler.clone();
            let down_layout = layout.clone();
            let down_doc = (*doc_id).to_owned();
            let move_handler = handler.clone();
            let move_layout = layout.clone();
            let move_doc = (*doc_id).to_owned();
            let up_handler = handler;
            let up_layout = layout;
            let up_doc = (*doc_id).to_owned();
            line = line
                .cursor(CursorStyle::IBeam)
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    down_handler.handle(
                        FeedSelectionPhase::Begin,
                        &down_doc,
                        text_position(&down_layout, row, event.position),
                        window,
                        cx,
                    );
                })
                .on_mouse_move(move |event, window, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        move_handler.handle(
                            FeedSelectionPhase::Extend,
                            &move_doc,
                            text_position(&move_layout, row, event.position),
                            window,
                            cx,
                        );
                    }
                })
                .on_mouse_up(MouseButton::Left, move |event, window, cx| {
                    up_handler.handle(
                        FeedSelectionPhase::End,
                        &up_doc,
                        text_position(&up_layout, row, event.position),
                        window,
                        cx,
                    );
                });
        }

        line.into_any_element()
    }
}
