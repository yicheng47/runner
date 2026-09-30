//! The profile split the role and crew pages share: a fixed column of who it
//! is and what it runs beside a flexible column of markdown cards, the card
//! that clamps long markdown, and the in-place markdown editor.

use std::rc::Rc;

use chrono::{DateTime, Datelike, TimeZone};
use gpui::prelude::*;
use gpui::{
    div, linear_color_stop, linear_gradient, px, rems, svg, AnyElement, App, Div, Entity, EntityId,
    FontWeight, KeyDownEvent, SharedString, Window,
};
use runner_app::ui::{focus_ring, TextField};

use crate::surfaces::mission_markdown::render_markdown;
use crate::theme;

/// The profile column's width: the picture, name, setup and short lists.
pub(crate) const PROFILE_COLUMN_WIDTH: f32 = 272.;
/// Below this the card column wraps under the profile column.
const CARD_COLUMN_BASIS: f32 = 360.;
const COLUMN_GAP: f32 = 48.;
const PAGE_MAX_WIDTH: f32 = 1104.;
const PAGE_PADDING_X: f32 = 32.;
/// Source lines a collapsed markdown card renders.
pub(crate) const PREVIEW_LINES: usize = 24;

/// The profile column's width on a page with `available` unzoomed pixels:
/// the fixed column beside the cards, or the page's whole width once the
/// cards wrap under it.
pub(crate) fn profile_column_width(available: f32) -> f32 {
    let content = available.min(PAGE_MAX_WIDTH) - 2. * PAGE_PADDING_X;
    if content < PROFILE_COLUMN_WIDTH + COLUMN_GAP + CARD_COLUMN_BASIS {
        content.max(0.)
    } else {
        PROFILE_COLUMN_WIDTH
    }
}

impl crate::NativeRoot {
    /// The profile column's width on the current window: the window less the
    /// sidebar, in unzoomed pixels, through [`profile_column_width`].
    pub(crate) fn profile_page_column_width(&self, window: &Window, cx: &App) -> f32 {
        let settings = self.settings(cx);
        let sidebar = if self.sidebar_collapsed {
            0.
        } else {
            settings.sidebar_width
        };
        profile_column_width(f32::from(window.viewport_size().width) / settings.app_zoom - sidebar)
    }
}

pub(crate) type ClickHandler = Rc<dyn Fn(&mut Window, &mut App)>;
pub(crate) type ModeHandler = Rc<dyn Fn(bool, &mut App)>;

/// The page body's centred container.
pub(crate) fn page_container() -> Div {
    div()
        .mx_auto()
        .w_full()
        .max_w(rems(PAGE_MAX_WIDTH / 16.))
        .flex()
        .flex_col()
        .gap(rems(28. / 16.))
        .px(rems(PAGE_PADDING_X / 16.))
        .pt(rems(40. / 16.))
        .pb_8()
}

/// `Roles › @handle`: the list the page came from, then the page itself.
pub(crate) fn breadcrumb(
    back_id: &'static str,
    back_label: &'static str,
    on_back: ClickHandler,
    current: impl IntoElement,
) -> Div {
    let key_back = Rc::clone(&on_back);
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_size(theme::text_body())
        .text_color(theme::muted())
        .child(
            div()
                .id(back_id)
                .flex_none()
                .tab_index(0)
                .cursor_pointer()
                .hover(|text| text.text_color(theme::text()))
                .focus_visible(|text| text.text_color(theme::text()).underline())
                .on_click(move |_, window, cx| on_back(window, cx))
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        key_back(window, cx);
                    }
                })
                .child(back_label),
        )
        .child(
            div()
                .flex_none()
                .text_color(theme::border_strong())
                .child("›"),
        )
        .child(current)
}

pub(crate) fn editing_tag() -> Div {
    div()
        .flex_none()
        .rounded(rems(3. / 16.))
        .bg(theme::raised())
        .px(rems(6. / 16.))
        .py(rems(1. / 16.))
        .font_family(theme::UI_MONOSPACE_FONT)
        .text_size(theme::text_micro())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::muted())
        .child("EDITING")
}

/// An amber dot and a short note, such as "Unsaved changes".
pub(crate) fn dot_note(note: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .text_size(theme::text_meta())
        .text_color(theme::faint())
        .child(override_dot())
        .child(note.into())
}

/// The amber dot that marks a changed or overridden value.
pub(crate) fn override_dot() -> Div {
    div()
        .flex_none()
        .size(rems(6. / 16.))
        .rounded_full()
        .bg(theme::warning())
}

/// A text control that a click, Enter or Space activates, such as Reset.
pub(crate) fn text_action(
    id: &'static str,
    focus: &gpui::FocusHandle,
    on_press: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<Div> {
    let on_press = Rc::new(on_press);
    let key_press = Rc::clone(&on_press);
    div()
        .id(id)
        .track_focus(focus)
        .tab_index(0)
        .flex()
        .items_center()
        .rounded(rems(3. / 16.))
        .cursor_pointer()
        .focus_visible(|action| action.shadow(focus_ring(theme::border_strong())))
        .on_click(move |_, window, cx| on_press(window, cx))
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                key_press(window, cx);
            }
        })
}

/// The profile column and the card column. The cards wrap under the profile
/// when the page is too narrow for both; with `stretch`, the card column runs
/// the profile column's height.
pub(crate) fn page_columns(left: Div, right: Div, stretch: bool) -> Div {
    div()
        .flex()
        .flex_wrap()
        .when(!stretch, |body| body.items_start())
        .gap_x(rems(COLUMN_GAP / 16.))
        .gap_y(rems(32. / 16.))
        .child(left)
        .child(right)
}

pub(crate) fn profile_column(width: f32) -> Div {
    div()
        .w(rems(width / 16.))
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(rems(20. / 16.))
}

pub(crate) fn card_column() -> Div {
    div()
        .flex_1()
        .flex_basis(rems(CARD_COLUMN_BASIS / 16.))
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_3()
}

pub(crate) fn caption(text: &'static str) -> AnyElement {
    div()
        .flex_none()
        .text_size(theme::text_ui())
        .line_height(rems(18. / 16.))
        .text_color(theme::faint())
        .child(text)
        .into_any_element()
}

/// A bordered card with an icon and title in its header, the title's figures
/// beside it, and `header_right` at the header's end.
pub(crate) fn card(
    icon: &'static str,
    title: &'static str,
    title_meta: Option<String>,
    header_right: AnyElement,
) -> Div {
    div()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded_lg()
        .border_1()
        .border_color(theme::border())
        .bg(theme::panel())
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py(rems(10. / 16.))
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            svg()
                                .flex_none()
                                .path(icon)
                                .size(rems(14. / 16.))
                                .text_color(theme::muted()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_body())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::text())
                                .child(title),
                        )
                        .children(title_meta.map(|meta| card_meta(Some(meta)))),
                )
                .child(header_right),
        )
}

/// The faint mono figures in a card's header, such as `48 lines · 2.9 KB`.
pub(crate) fn card_meta(meta: Option<String>) -> AnyElement {
    div()
        .flex_none()
        .font_family(theme::UI_MONOSPACE_FONT)
        .text_size(theme::text_caption())
        .text_color(theme::faint())
        .children(meta)
        .into_any_element()
}

/// Markdown clamped to its first lines behind a fade, with a Show all toggle
/// when it runs longer. `test_prefix` names the `_TEXT` and `_TOGGLE` hooks.
pub(crate) fn clamped_markdown(
    id: &str,
    text: &str,
    expanded: bool,
    test_prefix: &'static str,
    on_toggle: ClickHandler,
    view: EntityId,
    cx: &App,
) -> AnyElement {
    let key_toggle = Rc::clone(&on_toggle);
    let preview = prompt_preview(text);
    let clamped = !expanded && preview.is_some();
    let shown = if clamped {
        preview.unwrap_or(text)
    } else {
        text
    };
    let lines = text.lines().count();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .when(cfg!(test), |text| {
                    text.debug_selector(move || format!("{test_prefix}_TEXT"))
                })
                .relative()
                .min_w(px(0.))
                .text_size(theme::text_body())
                .text_color(theme::text())
                .child(render_markdown(
                    id,
                    shown,
                    view,
                    None,
                    theme::accent(),
                    None,
                    cx,
                ))
                .when(clamped, |text| {
                    text.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(rems(64. / 16.))
                            .bg(linear_gradient(
                                180.,
                                linear_color_stop(theme::with_alpha(theme::panel(), 0.), 0.),
                                linear_color_stop(theme::panel(), 1.),
                            )),
                    )
                }),
        )
        .children(preview.is_some().then(|| {
            div().flex().child(
                div()
                    .id(SharedString::from(format!("{id}-toggle")))
                    .when(cfg!(test), |toggle| {
                        toggle.debug_selector(move || format!("{test_prefix}_TOGGLE"))
                    })
                    .tab_index(0)
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded(rems(3. / 16.))
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|toggle| toggle.text_color(theme::text()))
                    .focus_visible(|toggle| {
                        toggle
                            .text_color(theme::text())
                            .shadow(focus_ring(theme::border_strong()))
                    })
                    .on_click(move |_, window, cx| on_toggle(window, cx))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            key_toggle(window, cx);
                        }
                    })
                    .child(if expanded {
                        "Show less".to_owned()
                    } else {
                        format!("Show all {lines} lines")
                    })
                    .child(
                        svg()
                            .flex_none()
                            .path(if expanded {
                                "chevron-up.svg"
                            } else {
                                "chevron-down.svg"
                            })
                            .size(rems(12. / 16.))
                            .text_color(theme::faint()),
                    ),
            )
        }))
        .into_any_element()
}

/// The markdown editor's body: the mono text field, or the rendered draft.
/// `test_prefix` names the `_EDITOR` and `_PREVIEW` hooks.
pub(crate) fn markdown_editor_body(
    id: &str,
    input: Entity<TextField>,
    preview: bool,
    test_prefix: &'static str,
    view: EntityId,
    cx: &App,
) -> AnyElement {
    if preview {
        let draft = input.read(cx).text().to_owned();
        div()
            .id(SharedString::from(format!("{id}-preview")))
            .when(cfg!(test), |body| {
                body.debug_selector(move || format!("{test_prefix}_PREVIEW"))
            })
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .text_size(theme::text_body())
            .text_color(theme::text())
            .child(if draft.trim().is_empty() {
                div()
                    .italic()
                    .text_color(theme::faint())
                    .child("Nothing to preview.")
                    .into_any_element()
            } else {
                render_markdown(id, &draft, view, None, theme::accent(), None, cx)
            })
            .into_any_element()
    } else {
        div()
            .when(cfg!(test), |body| {
                body.debug_selector(move || format!("{test_prefix}_EDITOR"))
            })
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .px_5()
            .py_4()
            .child(input)
            .into_any_element()
    }
}

/// The Markdown | Preview switch in an editing card's header.
pub(crate) fn markdown_mode_switch(
    id: &'static str,
    preview: bool,
    on_select: ModeHandler,
) -> AnyElement {
    let segment = |label: &'static str, active: bool, show_preview: bool| {
        let click = Rc::clone(&on_select);
        let key = Rc::clone(&on_select);
        div()
            .id(SharedString::from(format!("{id}-{}", label.to_lowercase())))
            .tab_index(0)
            .px_2()
            .py(rems(2. / 16.))
            .rounded(rems(3. / 16.))
            .text_size(theme::text_ui())
            .text_color(if active {
                theme::text()
            } else {
                theme::muted()
            })
            .when(active, |segment| segment.bg(theme::raised()))
            .cursor_pointer()
            .hover(|segment| segment.text_color(theme::text()))
            .focus_visible(|segment| segment.shadow(focus_ring(theme::border_strong())))
            .on_click(move |_, _, cx| click(show_preview, cx))
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    key(show_preview, cx);
                }
            })
            .child(label)
    };
    div()
        .flex()
        .items_center()
        .gap(rems(2. / 16.))
        .p(rems(2. / 16.))
        .rounded(rems(4. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg())
        .child(segment("Markdown", !preview, false))
        .child(segment("Preview", preview, true))
        .into_any_element()
}

/// A profile-column block under a hairline rule.
pub(crate) fn section() -> Div {
    div()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(theme::border())
        .pt(rems(20. / 16.))
}

/// A section or field label. It never wraps: a flex column's first sizing
/// pass offers a `min_w(0)` row no width, and a wrapping label would be
/// measured a character per line, leaving that height behind in the column.
pub(crate) fn section_label(label: impl Into<SharedString>) -> AnyElement {
    div()
        .whitespace_nowrap()
        .text_size(theme::text_meta())
        .text_color(theme::faint())
        .child(label.into())
        .into_any_element()
}

/// One line of truncated text in the fixed-width profile column. GPUI shapes
/// a non-wrapping line once, at the first width layout offers, and a flex
/// column's first sizing pass offers none (a 0 px width), which left every
/// value as a bare "…". An explicit width makes that first pass the real one.
pub(crate) fn column_text(text: impl Into<SharedString>, width: f32) -> Div {
    div().w(rems(width / 16.)).truncate().child(text.into())
}

pub(crate) fn plural(count: i64, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

/// A markdown card's header: `48 lines · 2.9 KB`.
pub(crate) fn prompt_meta(prompt: &str) -> String {
    let lines = plural(prompt.lines().count() as i64, "line", "lines");
    let bytes = prompt.len();
    if bytes < 1024 {
        format!("{lines} · {bytes} B")
    } else {
        format!("{lines} · {:.1} KB", bytes as f64 / 1024.)
    }
}

/// The first lines of markdown too long to show whole, or `None` when it fits.
pub(crate) fn prompt_preview(prompt: &str) -> Option<&str> {
    if prompt.lines().count() <= PREVIEW_LINES {
        return None;
    }
    let end = prompt
        .match_indices('\n')
        .nth(PREVIEW_LINES - 1)
        .map_or(prompt.len(), |(index, _)| index);
    Some(&prompt[..end])
}

/// `Sep 23, 17:41` this year, `Sep 23, 2025` before it.
pub(crate) fn short_timestamp<Tz: TimeZone>(timestamp: &DateTime<Tz>, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if timestamp.year() == now.year() {
        timestamp.format("%b %-d, %H:%M").to_string()
    } else {
        timestamp.format("%b %-d, %Y").to_string()
    }
}

pub(crate) fn local_short_timestamp(timestamp: runner_backend::model::Timestamp) -> String {
    short_timestamp(
        &timestamp.with_timezone(&chrono::Local),
        &chrono::Local::now(),
    )
}

/// `01K0…RCODER01`: enough of an id to tell rows apart.
pub(crate) fn short_id(id: &str) -> String {
    if id.len() <= 12 || !id.is_ascii() {
        return id.to_owned();
    }
    format!("{}…{}", &id[..4], &id[id.len() - 8..])
}
