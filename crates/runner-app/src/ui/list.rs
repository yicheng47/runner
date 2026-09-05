use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, App, BoxShadow, Context, CursorStyle, Entity, FontWeight,
    KeyDownEvent, Render, RenderOnce, ScrollHandle, SharedString, Subscription, Window,
};

use crate::theme;
use crate::ui::field::TextField;
use crate::ui::scrollbar::Scrollbar;

pub const PAGE_SIZE: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageWindowItem {
    Page(usize),
    Ellipsis,
}

pub fn clamp_page(page: usize, total_pages: usize) -> usize {
    page.max(1).min(total_pages.max(1))
}

pub fn page_window(current_page: usize, total_pages: usize) -> Vec<PageWindowItem> {
    use PageWindowItem::{Ellipsis, Page};
    if total_pages == 0 {
        return Vec::new();
    }
    if total_pages <= 5 {
        return (1..=total_pages).map(Page).collect();
    }
    let current = clamp_page(current_page, total_pages);
    if current <= 3 {
        return vec![
            Page(1),
            Page(2),
            Page(3),
            Page(4),
            Ellipsis,
            Page(total_pages),
        ];
    }
    if current >= total_pages - 2 {
        return vec![
            Page(1),
            Ellipsis,
            Page(total_pages - 3),
            Page(total_pages - 2),
            Page(total_pages - 1),
            Page(total_pages),
        ];
    }
    vec![
        Page(1),
        Ellipsis,
        Page(current - 1),
        Page(current),
        Page(current + 1),
        Ellipsis,
        Page(total_pages),
    ]
}

pub type SearchHandler = Rc<dyn Fn(String, &mut App)>;

pub struct SearchInput {
    input: Entity<TextField>,
    label: SharedString,
    last_value: String,
    disabled: bool,
    on_change: SearchHandler,
    _input_subscription: Subscription,
}

impl SearchInput {
    pub fn new(
        value: impl Into<String>,
        label: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        on_change: SearchHandler,
        cx: &mut Context<Self>,
    ) -> Self {
        let value = value.into();
        let input = cx.new(|input_cx| {
            let mut input =
                TextField::new(input_cx.focus_handle(), value.clone(), placeholder, false)
                    .text_size(13.);
            input.set_bare(true, input_cx);
            input
        });
        let input_subscription = cx.observe(&input, |this, input, cx| {
            let value = input.read(cx).text().to_owned();
            if value != this.last_value {
                this.last_value = value.clone();
                (this.on_change)(value, cx);
            }
            cx.notify();
        });
        Self {
            input,
            label: label.into(),
            last_value: value,
            disabled: false,
            on_change,
            _input_subscription: input_subscription,
        }
    }

    pub fn input(&self) -> Entity<TextField> {
        self.input.clone()
    }

    pub fn set_value(&mut self, value: impl Into<String>, cx: &mut Context<Self>) {
        let value = value.into();
        self.input
            .update(cx, |input, input_cx| input.reset(value, input_cx));
    }

    pub fn reset_value(&mut self, value: impl Into<String>, cx: &mut Context<Self>) {
        let value = value.into();
        self.last_value.clone_from(&value);
        self.input
            .update(cx, |input, input_cx| input.reset(value, input_cx));
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        self.input
            .update(cx, |input, input_cx| input.set_disabled(disabled, input_cx));
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.input
            .update(cx, |input, input_cx| input.reset("", input_cx));
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.disabled && event.keystroke.key == "escape" {
            cx.stop_propagation();
            self.clear(cx);
        }
    }
}

impl Render for SearchInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let value_present = !self.input.read(cx).text().is_empty();
        let disabled = self.disabled;
        let focus_handle = self.input.read(cx).focus_handle();
        let entity = cx.entity();
        let clear_key_entity = entity.clone();
        let label = self.label.clone();
        div()
            .id(SharedString::from(format!("search-input-{label}")))
            .w_full()
            .max_w(rems(320. / 16.))
            .flex()
            .items_center()
            .gap_2()
            .rounded(rems(4. / 16.))
            .border_1()
            .border_color(if focus_handle.is_focused(_window) {
                theme::border_strong()
            } else {
                theme::border()
            })
            .bg(theme::bg())
            .px_3()
            .py_2()
            .opacity(if disabled { 0.6 } else { 1. })
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                svg()
                    .path("search.svg")
                    .size(rems(14. / 16.))
                    .flex_none()
                    .text_color(theme::faint()),
            )
            .child(div().min_w(px(0.)).flex_1().child(self.input.clone()))
            .when(value_present && !disabled, |search| {
                let button = div()
                    .id(SharedString::from(format!("search-clear-{label}")))
                    .tab_index(0)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rems(20. / 16.))
                    .rounded(rems(4. / 16.))
                    .text_color(theme::faint())
                    .cursor_pointer()
                    .hover(|button| button.bg(theme::raised()).text_color(theme::text()))
                    .focus_visible(|button| {
                        button
                            .bg(theme::raised())
                            .text_color(theme::text())
                            .shadow(vec![BoxShadow {
                                color: theme::border_strong(),
                                offset: gpui::point(px(0.), px(0.)),
                                blur_radius: px(0.),
                                spread_radius: px(2.),
                            }])
                    })
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |search, cx| search.clear(cx));
                    })
                    .on_key_down(move |event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            clear_key_entity.update(cx, |search, cx| search.clear(cx));
                        }
                    })
                    .child(svg().path("close.svg").size(rems(14. / 16.)));
                search.child(button)
            })
    }
}

pub type PageHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Pager {
    page: usize,
    page_count: usize,
    on_page_change: PageHandler,
}

impl Pager {
    pub fn new(page: usize, page_count: usize, on_page_change: PageHandler) -> Self {
        Self {
            page,
            page_count,
            on_page_change,
        }
    }
}

impl RenderOnce for Pager {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let page = clamp_page(self.page, self.page_count);
        let previous = Rc::clone(&self.on_page_change);
        let next = Rc::clone(&self.on_page_change);
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(pager_icon_button(
                "pager-previous",
                "chevron-left.svg",
                self.page_count == 0 || page == 1,
                move |window, cx| previous(page.saturating_sub(1), window, cx),
            ))
            .children(
                page_window(page, self.page_count)
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| match item {
                        PageWindowItem::Ellipsis => div()
                            .size(rems(28. / 16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(rems(12. / 16.))
                            .text_color(theme::faint())
                            .child("…")
                            .into_any_element(),
                        PageWindowItem::Page(page) => {
                            let handler = Rc::clone(&self.on_page_change);
                            let key_handler = Rc::clone(&handler);
                            let active = page == clamp_page(self.page, self.page_count);
                            div()
                                .id(("pager-page", index))
                                .tab_index(0)
                                .size(rems(28. / 16.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(rems(4. / 16.))
                                .border_1()
                                .border_color(if active {
                                    theme::border_strong()
                                } else {
                                    gpui::transparent_black()
                                })
                                .bg(if active {
                                    theme::raised()
                                } else {
                                    gpui::transparent_black()
                                })
                                .font_weight(if active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_size(rems(12. / 16.))
                                .text_color(if active {
                                    theme::text()
                                } else {
                                    theme::muted()
                                })
                                .cursor_pointer()
                                .focus_visible(|style| {
                                    style.shadow(vec![BoxShadow {
                                        color: theme::border_strong(),
                                        offset: gpui::point(px(0.), px(0.)),
                                        blur_radius: px(0.),
                                        spread_radius: px(2.),
                                    }])
                                })
                                .hover(|button| {
                                    button.bg(theme::raised()).text_color(theme::text())
                                })
                                .on_click(move |_, window, cx| handler(page, window, cx))
                                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        cx.stop_propagation();
                                        key_handler(page, window, cx);
                                    }
                                })
                                .child(page.to_string())
                                .into_any_element()
                        }
                    }),
            )
            .child(pager_icon_button(
                "pager-next",
                "chevron-right.svg",
                self.page_count == 0 || page >= self.page_count,
                move |window, cx| next(page.saturating_add(1), window, cx),
            ))
    }
}

fn pager_icon_button(
    id: &'static str,
    icon: &'static str,
    disabled: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_press = Rc::new(on_press);
    let key_press = Rc::clone(&on_press);
    let icon = svg()
        .path(icon)
        .size(rems(14. / 16.))
        .text_color(theme::muted())
        .when(!disabled, |icon| {
            icon.group_hover(id, |icon| icon.text_color(theme::text()))
        });
    let mut button = div()
        .id(id)
        .when(!disabled, |button| button.group(id))
        .tab_index(0)
        .tab_stop(!disabled)
        .size(rems(28. / 16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rems(4. / 16.))
        .border_1()
        .border_color(gpui::transparent_black())
        .text_color(theme::muted())
        .opacity(if disabled { 0.5 } else { 1. })
        .cursor(if disabled {
            CursorStyle::OperationNotAllowed
        } else {
            CursorStyle::PointingHand
        })
        .focus_visible(|style| {
            style.shadow(vec![BoxShadow {
                color: theme::border_strong(),
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
            }])
        })
        .child(icon);
    if !disabled {
        button = button
            .hover(|button| button.bg(theme::raised()))
            .on_click(move |_, window, cx| on_press(window, cx))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    key_press(window, cx);
                }
            });
    }
    button.into_any_element()
}

#[derive(IntoElement)]
pub struct EmptyStateCard {
    icon: AnyElement,
    title: SharedString,
    description: SharedString,
    action: AnyElement,
}

impl EmptyStateCard {
    pub fn new(
        icon: impl IntoElement,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        action: impl IntoElement,
    ) -> Self {
        Self {
            icon: icon.into_any_element(),
            title: title.into(),
            description: description.into(),
            action: action.into_any_element(),
        }
    }
}

impl RenderOnce for EmptyStateCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().flex_1().flex().items_center().justify_center().child(
            div()
                .w_full()
                .max_w(rems(520. / 16.))
                .flex()
                .flex_col()
                .items_center()
                .gap(rems(20. / 16.))
                .rounded(rems(12. / 16.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::with_alpha(theme::panel(), 0.4))
                .p(rems(48. / 16.))
                .text_center()
                .child(
                    div()
                        .size(rems(64. / 16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .border_1()
                        .border_color(theme::with_alpha(theme::accent(), 0.3))
                        .bg(theme::with_alpha(theme::accent(), 0.1))
                        .text_color(theme::accent())
                        .child(self.icon),
                )
                .child(
                    div()
                        .text_size(rems(20. / 16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(self.title),
                )
                .child(
                    div()
                        .max_w(rems(384. / 16.))
                        .text_size(rems(14. / 16.))
                        .line_height(rems(22. / 16.))
                        .text_color(theme::muted())
                        .child(self.description),
                )
                .child(div().pt_2().child(self.action)),
        )
    }
}

#[derive(IntoElement)]
pub struct PaginatedListPage {
    title: SharedString,
    description: AnyElement,
    action: AnyElement,
    error: Option<SharedString>,
    loading: bool,
    loaded: bool,
    total_count: usize,
    filtered_count: usize,
    noun: SharedString,
    empty_state: AnyElement,
    search: Entity<SearchInput>,
    no_matches: AnyElement,
    page: usize,
    page_count: usize,
    on_page_change: PageHandler,
    content: AnyElement,
    scroll_handle: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
}

impl PaginatedListPage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        title: impl Into<SharedString>,
        description: impl IntoElement,
        action: impl IntoElement,
        noun: impl Into<SharedString>,
        empty_state: impl IntoElement,
        search: Entity<SearchInput>,
        no_matches: impl IntoElement,
        page: usize,
        page_count: usize,
        on_page_change: PageHandler,
        content: impl IntoElement,
        scroll_handle: ScrollHandle,
        scrollbar: Entity<Scrollbar>,
    ) -> Self {
        Self {
            title: title.into(),
            description: description.into_any_element(),
            action: action.into_any_element(),
            error: None,
            loading: false,
            loaded: true,
            total_count: 0,
            filtered_count: 0,
            noun: noun.into(),
            empty_state: empty_state.into_any_element(),
            search,
            no_matches: no_matches.into_any_element(),
            page,
            page_count,
            on_page_change,
            content: content.into_any_element(),
            scroll_handle,
            scrollbar,
        }
    }

    pub fn counts(mut self, filtered: usize, total: usize) -> Self {
        self.filtered_count = filtered;
        self.total_count = total;
        self
    }

    pub fn load_state(mut self, loading: bool, loaded: bool, error: Option<SharedString>) -> Self {
        self.loading = loading;
        self.loaded = loaded;
        self.error = error;
        self
    }
}

impl RenderOnce for PaginatedListPage {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        #[cfg(not(windows))]
        let _ = window;
        let body = if self.loading && !self.loaded {
            div()
                .text_size(rems(14. / 16.))
                .text_color(theme::muted())
                .child("Loading…")
                .into_any_element()
        } else if !self.loaded {
            div()
                .rounded(rems(4. / 16.))
                .border_1()
                .border_color(theme::with_alpha(theme::danger(), 0.4))
                .bg(theme::with_alpha(theme::danger(), 0.1))
                .px_3()
                .py_2()
                .text_size(rems(14. / 16.))
                .text_color(theme::danger())
                .child(format!("Failed to load {}.", self.noun))
                .into_any_element()
        } else if self.total_count == 0 {
            self.empty_state
        } else {
            let scroll_handle = self.scroll_handle;
            let scrollbar = self.scrollbar;
            let list = if self.filtered_count == 0 {
                self.no_matches
            } else {
                div()
                    .min_h(px(0.))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .relative()
                            .min_h(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .id("paginated-list-scroll")
                                    .size_full()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .overflow_y_scroll()
                                    .scrollbar_width(px(0.))
                                    .track_scroll(&scroll_handle)
                                    .child(self.content),
                            )
                            .child(scrollbar),
                    )
                    .child(
                        div()
                            .mt_auto()
                            .flex_none()
                            .flex()
                            .justify_center()
                            .border_t_1()
                            .border_color(theme::border())
                            .pt(rems(10. / 16.))
                            .child(Pager::new(self.page, self.page_count, self.on_page_change)),
                    )
                    .into_any_element()
            };
            div()
                .min_h(px(0.))
                .flex_1()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .child(self.search)
                        .child(
                            div()
                                .flex_none()
                                .font_family("JetBrains Mono")
                                .text_size(rems(11. / 16.))
                                .text_color(theme::muted())
                                .child(format!(
                                    "{} of {} {}",
                                    self.filtered_count, self.total_count, self.noun
                                )),
                        ),
                )
                .child(list)
                .into_any_element()
        };

        div()
            .min_h(px(0.))
            .flex_1()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .min_h(px(0.))
                    .w_full()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(rems(24. / 16.))
                    .px(rems(32. / 16.))
                    .pt(rems(40. / 16.))
                    .pb(rems(18. / 16.))
                    .child(
                        div()
                            .map(|header| {
                                #[cfg(windows)]
                                let header = header.when(!window.is_fullscreen(), |header| {
                                    header.pr(rems(3. * super::CAPTION_BUTTON_WIDTH / 16.))
                                });
                                header
                            })
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(rems(24. / 16.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme::text())
                                            .child(self.title),
                                    )
                                    .child(
                                        div()
                                            .text_size(rems(14. / 16.))
                                            .text_color(theme::muted())
                                            .child(self.description),
                                    ),
                            )
                            .child(self.action),
                    )
                    .children(self.error.map(|error| {
                        div()
                            .rounded(rems(4. / 16.))
                            .border_1()
                            .border_color(theme::with_alpha(theme::danger(), 0.4))
                            .bg(theme::with_alpha(theme::danger(), 0.1))
                            .px_3()
                            .py_2()
                            .text_size(rems(14. / 16.))
                            .text_color(theme::danger())
                            .child(error)
                    }))
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PageWindowItem::{Ellipsis, Page};

    #[test]
    fn compact_page_window_matches_main() {
        assert_eq!(page_window(1, 0), []);
        assert_eq!(page_window(2, 4), [Page(1), Page(2), Page(3), Page(4)]);
        assert_eq!(
            page_window(1, 8),
            [Page(1), Page(2), Page(3), Page(4), Ellipsis, Page(8)]
        );
        assert_eq!(
            page_window(4, 8),
            [
                Page(1),
                Ellipsis,
                Page(3),
                Page(4),
                Page(5),
                Ellipsis,
                Page(8)
            ]
        );
        assert_eq!(
            page_window(8, 8),
            [Page(1), Ellipsis, Page(5), Page(6), Page(7), Page(8)]
        );
    }

    #[test]
    fn page_clamping_keeps_one_based_bounds() {
        assert_eq!(clamp_page(0, 0), 1);
        assert_eq!(clamp_page(99, 4), 4);
    }
}
